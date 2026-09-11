//! Interface IP polling loop and bound task execution logic.

use crate::config::{InterfaceConfig, TaskConfig};
use crate::engine::{HttpEngine, TemplateContext};
use crate::error::RdnsError;
use crate::ip::{DnsResolver, InterfaceIpResolver};
use crate::notification::{NotificationDispatcher, NotificationEvent};
use crate::persistence::StateStore;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio_util::sync::CancellationToken;

use parking_lot::Mutex;

/// Shared context and dependencies for executing tasks.
#[derive(Clone)]
pub struct TaskContext {
    pub engine: HttpEngine,
    pub state_store: StateStore,
    pub notifier: NotificationDispatcher,
    pub dns_resolver: DnsResolver,
    pub dns_server: Option<String>,
    pub timeout: Duration,
    pub normal_interval_secs: u64,
    pub dry_run: bool,
}

/// Execution outcome of a single task run.
#[derive(Debug, Default)]
pub struct TaskRunOutcome {
    /// Whether the interface loop should shorten the next polling cycle to `retry_interval`.
    /// Set to true ONLY when update failed AND DNS query is inconsistent with local IP.
    pub should_shorten_interval: bool,
    /// Error if task execution failed.
    pub error: Option<RdnsError>,
}

/// Execution outcome of a full interface polling round across all bound tasks.
#[derive(Debug, Clone, Copy, Default)]
pub struct InterfaceRunOutcome {
    /// Whether at least one task requested a shortened retry interval.
    pub should_shorten_interval: bool,
    /// Whether at least one task (or IP fetch) failed.
    pub has_error: bool,
}

#[derive(Default, Debug)]
struct HeartbeatState {
    last_attempt_time: Option<u64>,
    failure_count: u32,
}

pub struct TaskExecutor {
    config: TaskConfig,
    ctx: TaskContext,
    heartbeat_state: Mutex<HeartbeatState>,
}

impl TaskExecutor {
    pub fn new(config: TaskConfig, ctx: TaskContext) -> Self {
        Self {
            config,
            ctx,
            heartbeat_state: Mutex::new(HeartbeatState::default()),
        }
    }

    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// Execute diffing, templating, and Webhook updating using the provided IPs.
    pub async fn execute_with_ip(
        &self,
        v4_opt: Option<Ipv4Addr>,
        v6_opt: Option<Ipv6Addr>,
    ) -> TaskRunOutcome {
        let task_name = &self.config.name;
        let v4_str = v4_opt.map(|ip| ip.to_string());
        let v6_str = v6_opt.map(|ip| ip.to_string());

        // 1. Diff against cached state
        let cached = self.ctx.state_store.get(task_name).unwrap_or_default();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let new_ip_desc = match (&v4_str, &v6_str) {
            (Some(v4), Some(v6)) => format!("{}, {}", v4, v6),
            (Some(v4), None) => v4.clone(),
            (None, Some(v6)) => v6.clone(),
            (None, None) => "none".to_string(),
        };
        let old_ip_desc = match (&cached.ipv4, &cached.ipv6) {
            (Some(v4), Some(v6)) => format!("{}, {}", v4, v6),
            (Some(v4), None) => v4.clone(),
            (None, Some(v6)) => v6.clone(),
            (None, None) => "".to_string(),
        };

        let mut need_update = false;
        let mut known_dns_mismatch = false;

        if v4_str != cached.ipv4 || v6_str != cached.ipv6 {
            need_update = true;
            tracing::info!(
                "[{}] Update needed - L: '{}' <> R: '{}'",
                task_name,
                new_ip_desc,
                old_ip_desc
            );
        } else if let Some(ref domain) = self.config.domain {
            // Local IP hasn't changed. Check cloud DNS record if domain is configured.
            // TTL propagation cooldown: don't query DNS if updated within 60s
            let in_cooldown = cached
                .last_success_time
                .map(|t| {
                    if now < t {
                        tracing::warn!(
                            "[{}] System clock jumped backwards, bypassing cooldown",
                            task_name
                        );
                        false
                    } else {
                        now - t < 60
                    }
                })
                .unwrap_or(false);

            if !in_cooldown {
                match self
                    .ctx
                    .dns_resolver
                    .resolve(domain, self.ctx.dns_server.as_deref(), self.ctx.timeout)
                    .await
                {
                    Ok((dns_v4, dns_v6)) => {
                        let v4_mismatch = v4_opt.is_some() && dns_v4 != v4_opt;
                        let v6_mismatch = v6_opt.is_some() && dns_v6 != v6_opt;
                        if v4_mismatch || v6_mismatch {
                            need_update = true;
                            known_dns_mismatch = true;
                            let dns_ip_desc = match (dns_v4, dns_v6) {
                                (Some(v4), Some(v6)) => format!("{}, {}", v4, v6),
                                (Some(v4), None) => v4.to_string(),
                                (None, Some(v6)) => v6.to_string(),
                                (None, None) => "".to_string(),
                            };
                            tracing::info!(
                                "[{}] Update needed - L: '{}' <> R: '{}' (domain: '{}')",
                                task_name,
                                new_ip_desc,
                                dns_ip_desc,
                                domain
                            );
                        } else {
                            tracing::debug!(
                                "[{}] Registered IP matches local IP for domain '{}'",
                                task_name,
                                domain
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[{}] DNS lookup failed for domain '{}': {}",
                            task_name,
                            domain,
                            e
                        );
                    }
                }
            }
        }

        let mut is_heartbeat_attempt = false;
        if !need_update
            && let Some(force_interval) = self.config.force_update_interval
            && let Some(last_time) = cached.last_success_time
            && now.saturating_sub(last_time) >= force_interval
        {
            let hb = self.heartbeat_state.lock();
            let backoff_secs = if hb.failure_count == 0 {
                0
            } else {
                let shift = (hb.failure_count - 1).min(5);
                (60u64 * (1u64 << shift)).min(self.ctx.normal_interval_secs)
            };
            let can_retry = match hb.last_attempt_time {
                Some(last_attempt) => now.saturating_sub(last_attempt) >= backoff_secs,
                None => true,
            };
            if can_retry {
                need_update = true;
                is_heartbeat_attempt = true;
                tracing::info!(
                    "[{}] Forced update - interval {}s reached",
                    task_name,
                    force_interval
                );
            } else {
                tracing::info!(
                    "[{}] Forced update delayed due to backoff (retry in {}s, failures: {})",
                    task_name,
                    backoff_secs,
                    hb.failure_count
                );
            }
        }

        // If dry_run is true, always execute to show the preview
        if !need_update && !self.ctx.dry_run {
            tracing::debug!(
                "[{}] Update not needed - IP '{}' unchanged",
                task_name,
                new_ip_desc
            );
            return TaskRunOutcome::default();
        }

        // 2. Build Template Context
        let ctx = TemplateContext {
            ipv4: v4_str.as_deref(),
            ipv6: v6_str.as_deref(),
            domain: self.config.domain.as_deref(),
            timestamp: Some(now),
            args: Some(&self.config.args),
        };

        let req_cfg = match self.config.request.as_ref() {
            Some(r) => r,
            None => {
                let err = RdnsError::Assertion(format!(
                    "Task '{}' has no request configuration",
                    task_name
                ));
                tracing::error!("[{}] Task has no request configuration", task_name);
                return TaskRunOutcome {
                    should_shorten_interval: false,
                    error: Some(err),
                };
            }
        };

        // 3. Execute HTTP Webhook
        match self
            .ctx
            .engine
            .executor()
            .execute(task_name, req_cfg, &ctx, self.ctx.dry_run)
            .await
        {
            Ok(()) => {
                {
                    let mut hb = self.heartbeat_state.lock();
                    hb.failure_count = 0;
                    hb.last_attempt_time = None;
                }
                if self.ctx.dry_run {
                    tracing::info!(
                        "[{}] Dry Run: NO update sent (preview verified)",
                        task_name
                    );
                } else if is_heartbeat_attempt {
                    tracing::info!(
                        "[{}] Forced update successful - IP '{}' sent",
                        task_name,
                        new_ip_desc
                    );
                } else {
                    tracing::info!(
                        "[{}] Update successful - IP '{}' sent",
                        task_name,
                        new_ip_desc
                    );
                }
                if !self.ctx.dry_run {
                    let old_ip = format!("v4: {:?}, v6: {:?}", cached.ipv4, cached.ipv6);
                    let new_ip = format!("v4: {:?}, v6: {:?}", v4_str, v6_str);

                    // 1. Dispatch Recovery first: checks and resets failure count
                    self.ctx
                        .notifier
                        .dispatch(NotificationEvent::Recovery {
                            task_name,
                            current_ip: &new_ip,
                        })
                        .await;

                    // 2. Dispatch Change ONLY if IP actually changed (suppress heartbeat notification storms)
                    let ip_actually_changed = v4_str != cached.ipv4 || v6_str != cached.ipv6;
                    if ip_actually_changed {
                        self.ctx
                            .notifier
                            .dispatch(NotificationEvent::Change {
                                task_name,
                                old_ip: &old_ip,
                                new_ip: &new_ip,
                            })
                            .await;
                    }

                    // 3. Update state store
                    self.ctx
                        .state_store
                        .update(task_name, v4_str.clone(), v6_str.clone(), now);
                }
                // Update succeeded: do NOT shorten interval (DNS propagation delay is expected)
                TaskRunOutcome {
                    should_shorten_interval: false,
                    error: None,
                }
            }
            Err(e) => {
                let err_msg = e.to_string();
                tracing::error!(
                    "[{}] Updating IP at DDNS provider failed: {}",
                    task_name,
                    err_msg
                );
                if is_heartbeat_attempt {
                    let mut hb = self.heartbeat_state.lock();
                    hb.failure_count = hb.failure_count.saturating_add(1);
                    hb.last_attempt_time = Some(now);
                    tracing::warn!(
                        "[{}] Heartbeat failure recorded for backoff (count: {})",
                        task_name,
                        hb.failure_count
                    );
                }
                if !self.ctx.dry_run {
                    self.ctx
                        .notifier
                        .dispatch(NotificationEvent::Failure {
                            task_name,
                            error_message: &err_msg,
                        })
                        .await;
                }

                // Check condition: update failed AND DNS is inconsistent (or task has no domain)
                let mut should_shorten = false;
                if let Some(ref domain) = self.config.domain {
                    let dns_mismatch = if known_dns_mismatch {
                        true
                    } else {
                        match self
                            .ctx
                            .dns_resolver
                            .resolve(domain, self.ctx.dns_server.as_deref(), self.ctx.timeout)
                            .await
                        {
                            Ok((dns_v4, dns_v6)) => {
                                (v4_opt.is_some() && dns_v4 != v4_opt)
                                    || (v6_opt.is_some() && dns_v6 != v6_opt)
                            }
                            Err(dns_err) => {
                                tracing::warn!(
                                    "[{}] Failed to query DNS record for domain '{}' after update failure: {}",
                                    task_name,
                                    domain,
                                    dns_err
                                );
                                true
                            }
                        }
                    };

                    if dns_mismatch {
                        tracing::warn!(
                            "[{}] DNS record inconsistent after failure, requesting shortened retry interval",
                            task_name
                        );
                        should_shorten = true;
                    } else {
                        tracing::info!(
                            "[{}] DNS record matches actual IP after failure, maintaining normal interval",
                            task_name
                        );
                    }
                } else {
                    tracing::warn!(
                        "[{}] Update failed for domainless task, requesting shortened retry interval",
                        task_name
                    );
                    should_shorten = true;
                }

                TaskRunOutcome {
                    should_shorten_interval: should_shorten,
                    error: Some(e),
                }
            }
        }
    }
}

/// Scheduler managing a single interface IP query task and all bound Webhook tasks.
pub struct InterfaceScheduler {
    config: InterfaceConfig,
    resolver: InterfaceIpResolver,
    tasks: Vec<Arc<TaskExecutor>>,
    normal_interval: Duration,
    retry_interval: Duration,
}

impl InterfaceScheduler {
    pub fn new(
        config: InterfaceConfig,
        tasks: Vec<Arc<TaskExecutor>>,
        default_interval_secs: u64,
        default_retry_interval_secs: u64,
        timeout: Duration,
    ) -> Self {
        let interval_secs = config.interval.unwrap_or(default_interval_secs);
        let retry_secs = config.retry_interval.unwrap_or(default_retry_interval_secs);
        let resolver = InterfaceIpResolver::new(config.ipv4.clone(), config.ipv6.clone(), timeout);

        Self {
            config,
            resolver,
            tasks,
            normal_interval: Duration::from_secs(interval_secs),
            retry_interval: Duration::from_secs(retry_secs),
        }
    }

    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// Execute a single round of interface IP detection, driving all bound tasks concurrently.
    pub async fn run_round(&self) -> InterfaceRunOutcome {
        let iface_name = &self.config.name;
        tracing::debug!("[{}] Detecting current IP", iface_name);

        let (v4_opt, v6_opt) = match self.resolver.resolve().await {
            Ok(ips) => ips,
            Err(e) => {
                let err = RdnsError::IpFetch {
                    task: iface_name.clone(),
                    source: e,
                };
                tracing::error!(
                    "[{}] Failed to detect current IP: {}",
                    iface_name,
                    err
                );
                return InterfaceRunOutcome {
                    should_shorten_interval: true,
                    has_error: true,
                };
            }
        };

        let ip_desc = match (v4_opt, v6_opt) {
            (Some(v4), Some(v6)) => format!("'{}', IPv6 '{}'", v4, v6),
            (Some(v4), None) => format!("'{}'", v4),
            (None, Some(v6)) => format!("IPv6 '{}'", v6),
            (None, None) => "none".to_string(),
        };

        tracing::info!(
            "[{}] Current IP {} detected",
            iface_name,
            ip_desc
        );

        let mut join_set = tokio::task::JoinSet::new();
        for task in &self.tasks {
            let t = Arc::clone(task);
            join_set.spawn(async move {
                (
                    t.name().to_string(),
                    t.execute_with_ip(v4_opt, v6_opt).await,
                )
            });
        }

        let mut has_error = false;
        let mut should_shorten_interval = false;
        while let Some(res) = join_set.join_next().await {
            match res {
                Ok((task_name, outcome)) => {
                    if outcome.should_shorten_interval {
                        should_shorten_interval = true;
                    }
                    if let Some(e) = outcome.error {
                        tracing::error!(
                            "[{}] Task '{}' failed: {}",
                            iface_name,
                            task_name,
                            e
                        );
                        has_error = true;
                    }
                }
                Err(join_err) => {
                    tracing::error!(
                        "[{}] Task panicked or was aborted: {}",
                        iface_name,
                        join_err
                    );
                    has_error = true;
                }
            }
        }

        InterfaceRunOutcome {
            should_shorten_interval,
            has_error,
        }
    }

    /// Execute a single round and return Ok/Err (used for CLI --once mode).
    pub async fn run_once(&self) -> Result<InterfaceRunOutcome, RdnsError> {
        let outcome = self.run_round().await;
        if outcome.has_error {
            Err(RdnsError::Assertion(format!(
                "One or more tasks failed on interface '{}'",
                self.config.name
            )))
        } else {
            Ok(outcome)
        }
    }

    /// Asynchronous loop executing interface IP discovery periodically until cancellation.
    pub async fn run_loop(&self, token: CancellationToken) {
        let iface_name = self.config.name.clone();
        tracing::info!(
            "[{}] Starting periodic loop (check interval: {}s, retry interval: {}s)",
            iface_name,
            self.normal_interval.as_secs(),
            self.retry_interval.as_secs()
        );

        loop {
            if token.is_cancelled() {
                tracing::info!(
                    "[{}] Received cancellation, terminating loop",
                    iface_name
                );
                break;
            }

            let outcome = self.run_round().await;
            if outcome.has_error {
                tracing::warn!(
                    "[{}] Encountered error during check round",
                    iface_name
                );
            }

            let next_interval = if outcome.should_shorten_interval {
                tracing::warn!(
                    "[{}] Update failed - entering shortened retry interval ({}s)",
                    iface_name,
                    self.retry_interval.as_secs()
                );
                self.retry_interval
            } else {
                tracing::debug!(
                    "[{}] Waiting {} seconds (Check Interval)",
                    iface_name,
                    self.normal_interval.as_secs()
                );
                self.normal_interval
            };

            tokio::select! {
                _ = token.cancelled() => {
                    tracing::info!(
                        "[{}] Received cancellation during sleep, shutting down immediately",
                        iface_name
                    );
                    break;
                }
                _ = tokio::time::sleep(next_interval) => {}
            }
        }

        tracing::info!("[{}] Interface loop terminated cleanly", iface_name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RequestConfig;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_update_failure_and_dns_mismatch_shortens_interval() {
        let tmp_dir = std::env::temp_dir().join(format!("rdns_test_{}", std::process::id()));
        let state_path = tmp_dir.join("state.json");
        let state_store = StateStore::new(&state_path);

        let global = crate::config::GlobalConfig::default();
        let engine = HttpEngine::new(&global).unwrap();
        let notifier = NotificationDispatcher::new(None, engine.clone());
        let dns_resolver = DnsResolver::new();

        let ctx = TaskContext {
            engine,
            state_store,
            notifier,
            dns_resolver,
            dns_server: None,
            timeout: Duration::from_millis(500),
            normal_interval_secs: 60,
            dry_run: false,
        };

        // Task with unreachable URL (will fail update) and a domain that won't match
        let task_cfg = TaskConfig {
            name: "test-fail-task".to_string(),
            interface: None,
            force_update_interval: None,
            domain: Some("invalid-domain-xyz-never-exist.test".to_string()),
            provider: None,
            args: HashMap::new(),
            request: Some(RequestConfig {
                url: Some("http://127.0.0.1:1/unreachable".to_string()),
                ..Default::default()
            }),
        };

        let executor = TaskExecutor::new(task_cfg, ctx);
        let outcome = executor
            .execute_with_ip(Some("180.110.160.241".parse().unwrap()), None)
            .await;

        assert!(outcome.error.is_some(), "Update must fail");
        assert!(
            outcome.should_shorten_interval,
            "When update fails and DNS is inconsistent, interval must be shortened"
        );

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[tokio::test]
    async fn test_update_success_does_not_shorten_interval() {
        let tmp_dir = std::env::temp_dir().join(format!("rdns_test_succ_{}", std::process::id()));
        let state_path = tmp_dir.join("state.json");
        let state_store = StateStore::new(&state_path);

        let global = crate::config::GlobalConfig::default();
        let engine = HttpEngine::new(&global).unwrap();
        let notifier = NotificationDispatcher::new(None, engine.clone());
        let dns_resolver = DnsResolver::new();

        // Dry-run simulates a successful update without actual HTTP write
        let ctx = TaskContext {
            engine,
            state_store,
            notifier,
            dns_resolver,
            dns_server: None,
            timeout: Duration::from_millis(500),
            normal_interval_secs: 60,
            dry_run: true,
        };

        let task_cfg = TaskConfig {
            name: "test-succ-task".to_string(),
            interface: None,
            force_update_interval: None,
            domain: Some("wox-office.freeddns.org".to_string()),
            provider: None,
            args: HashMap::new(),
            request: Some(RequestConfig {
                url: Some("http://example.com".to_string()),
                ..Default::default()
            }),
        };

        let executor = TaskExecutor::new(task_cfg, ctx);
        let outcome = executor
            .execute_with_ip(Some("180.110.160.241".parse().unwrap()), None)
            .await;

        assert!(outcome.error.is_none(), "Dry-run update succeeds");
        assert!(
            !outcome.should_shorten_interval,
            "Successful update must never shorten interval"
        );

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[tokio::test]
    async fn test_task_with_args_and_provider_in_dry_run() {
        let tmp_dir = std::env::temp_dir().join(format!("rdns_test_args_{}", std::process::id()));
        let state_path = tmp_dir.join("state.json");
        let state_store = StateStore::new(&state_path);

        let global = crate::config::GlobalConfig::default();
        let engine = HttpEngine::new(&global).unwrap();
        let notifier = NotificationDispatcher::new(None, engine.clone());
        let dns_resolver = DnsResolver::new();

        let ctx = TaskContext {
            engine,
            state_store,
            notifier,
            dns_resolver,
            dns_server: None,
            timeout: Duration::from_millis(500),
            normal_interval_secs: 60,
            dry_run: true,
        };

        let mut args = HashMap::new();
        args.insert("password".to_string(), "my_test_password".to_string());

        let dynu_provider = crate::provider::get_provider("dynu").unwrap();
        let task_cfg = TaskConfig {
            name: "test-dynu-task".to_string(),
            interface: None,
            force_update_interval: None,
            domain: Some("test.freeddns.org".to_string()),
            provider: Some("dynu".to_string()),
            args,
            request: Some(dynu_provider.default_request()),
        };

        let executor = TaskExecutor::new(task_cfg, ctx);
        let outcome = executor
            .execute_with_ip(
                Some("1.2.3.4".parse().unwrap()),
                Some("2001:db8::1".parse().unwrap()),
            )
            .await;

        assert!(outcome.error.is_none(), "Dry-run with args should succeed");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[tokio::test]
    async fn test_force_update_unchanged_ip_recovers_and_suppresses_change() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let response =
                    "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ngood";
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });

        let tmp_dir =
            std::env::temp_dir().join(format!("rdns_test_suppress_{}", std::process::id()));
        let state_path = tmp_dir.join("state.json");
        let state_store = StateStore::new(&state_path);

        // Pre-populate state_store with current IP and old timestamp to trigger force update
        let ip_v4 = "192.168.1.100".to_string();
        state_store.update("test-heartbeat", Some(ip_v4.clone()), None, 100);

        let global = crate::config::GlobalConfig::default();
        let engine = HttpEngine::new(&global).unwrap();
        let notifier = NotificationDispatcher::new(
            Some(crate::config::NotificationConfig::default()),
            engine.clone(),
        );
        let dns_resolver = DnsResolver::new();

        // Seed a failure
        notifier
            .dispatch(NotificationEvent::Failure {
                task_name: "test-heartbeat",
                error_message: "simulated failure",
            })
            .await;

        let ctx = TaskContext {
            engine,
            state_store: state_store.clone(),
            notifier: notifier.clone(),
            dns_resolver,
            dns_server: None,
            timeout: Duration::from_millis(500),
            normal_interval_secs: 60,
            dry_run: false,
        };

        let task_cfg = TaskConfig {
            name: "test-heartbeat".to_string(),
            interface: None,
            force_update_interval: Some(10), // Triggers force update
            domain: None,
            provider: None,
            args: HashMap::new(),
            request: Some(RequestConfig {
                url: Some(format!("http://{}/update", addr)),
                success_contains: Some(vec!["good".to_string()]),
                ..Default::default()
            }),
        };

        let executor = TaskExecutor::new(task_cfg, ctx);
        let outcome = executor
            .execute_with_ip(Some("192.168.1.100".parse().unwrap()), None)
            .await;

        assert!(outcome.error.is_none(), "Heartbeat update should succeed");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[tokio::test]
    async fn test_no_domain_failure_shortens_interval() {
        let tmp_dir = std::env::temp_dir().join(format!("rdns_test_nodom_{}", std::process::id()));
        let state_path = tmp_dir.join("state.json");
        let state_store = StateStore::new(&state_path);

        let global = crate::config::GlobalConfig::default();
        let engine = HttpEngine::new(&global).unwrap();
        let notifier = NotificationDispatcher::new(None, engine.clone());
        let dns_resolver = DnsResolver::new();

        let ctx = TaskContext {
            engine,
            state_store,
            notifier,
            dns_resolver,
            dns_server: None,
            timeout: Duration::from_millis(500),
            normal_interval_secs: 60,
            dry_run: false,
        };

        let task_cfg = TaskConfig {
            name: "test-nodom-fail".to_string(),
            interface: None,
            force_update_interval: None,
            domain: None, // No domain
            provider: None,
            args: HashMap::new(),
            request: Some(RequestConfig {
                url: Some("http://127.0.0.1:1/unreachable".to_string()),
                ..Default::default()
            }),
        };

        let executor = TaskExecutor::new(task_cfg, ctx);
        let outcome = executor
            .execute_with_ip(Some("180.110.160.241".parse().unwrap()), None)
            .await;

        assert!(outcome.error.is_some(), "Update must fail");
        assert!(
            outcome.should_shorten_interval,
            "Tasks without domain must shorten interval on update failure (S2)"
        );

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[tokio::test]
    async fn test_heartbeat_failure_exponential_backoff() {
        let tmp_dir =
            std::env::temp_dir().join(format!("rdns_test_backoff_{}", std::process::id()));
        let state_path = tmp_dir.join("state.json");
        let state_store = StateStore::new(&state_path);

        let ip_v4 = "192.168.1.100".to_string();
        state_store.update("test-hb-backoff", Some(ip_v4.clone()), None, 100);

        let global = crate::config::GlobalConfig::default();
        let engine = HttpEngine::new(&global).unwrap();
        let notifier = NotificationDispatcher::new(None, engine.clone());
        let dns_resolver = DnsResolver::new();

        let ctx = TaskContext {
            engine,
            state_store: state_store.clone(),
            notifier,
            dns_resolver,
            dns_server: None,
            timeout: Duration::from_millis(500),
            normal_interval_secs: 300,
            dry_run: false,
        };

        let task_cfg = TaskConfig {
            name: "test-hb-backoff".to_string(),
            interface: None,
            force_update_interval: Some(10), // Triggers force update
            domain: None,
            provider: None,
            args: HashMap::new(),
            request: Some(RequestConfig {
                url: Some("http://127.0.0.1:1/unreachable".to_string()),
                ..Default::default()
            }),
        };

        let executor = TaskExecutor::new(task_cfg, ctx);
        let test_ip = Some("192.168.1.100".parse().unwrap());

        // First run: force update attempted and fails
        let out1 = executor.execute_with_ip(test_ip, None).await;
        assert!(out1.error.is_some(), "First attempt should fail");
        assert_eq!(executor.heartbeat_state.lock().failure_count, 1);

        // Immediate next run: should be suppressed by backoff (60s delay)
        let out2 = executor.execute_with_ip(test_ip, None).await;
        assert!(
            out2.error.is_none(),
            "Immediate retry must be skipped due to backoff"
        );

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
