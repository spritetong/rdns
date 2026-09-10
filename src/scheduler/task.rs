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

/// Shared context and dependencies for executing tasks.
#[derive(Clone)]
pub struct TaskContext {
    pub engine: HttpEngine,
    pub state_store: StateStore,
    pub notifier: NotificationDispatcher,
    pub dns_resolver: DnsResolver,
    pub dns_server: Option<String>,
    pub timeout: Duration,
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

pub struct TaskExecutor {
    config: TaskConfig,
    ctx: TaskContext,
}

impl TaskExecutor {
    pub fn new(config: TaskConfig, ctx: TaskContext) -> Self {
        Self { config, ctx }
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

        let mut need_update = false;
        let mut known_dns_mismatch = false;

        if v4_str != cached.ipv4 || v6_str != cached.ipv6 {
            need_update = true;
            tracing::info!(
                task = %task_name,
                old_v4 = ?cached.ipv4,
                new_v4 = ?v4_str,
                old_v6 = ?cached.ipv6,
                new_v6 = ?v6_str,
                "IP change detected"
            );
        } else if let Some(ref domain) = self.config.domain {
            // Local IP hasn't changed. Check cloud DNS record if domain is configured.
            // TTL propagation cooldown: don't query DNS if updated within 60s
            let in_cooldown = cached
                .last_success_time
                .map(|t| now.saturating_sub(t) < 60)
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
                            tracing::info!(
                                task = %task_name,
                                domain = %domain,
                                dns_v4 = ?dns_v4,
                                actual_v4 = ?v4_opt,
                                dns_v6 = ?dns_v6,
                                actual_v6 = ?v6_opt,
                                "DNS record discrepancy detected against cloud server, triggering reconciliation update"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            task = %task_name,
                            domain = %domain,
                            error = %e,
                            "Failed to query DNS record for discrepancy check"
                        );
                    }
                }
            }
        }

        if !need_update
            && let Some(force_interval) = self.config.force_update_interval
            && let Some(last_time) = cached.last_success_time
            && now.saturating_sub(last_time) >= force_interval
        {
            need_update = true;
            tracing::info!(
                task = %task_name,
                force_interval,
                "Force update interval reached, executing update"
            );
        }

        // If dry_run is true, always execute to show the preview
        if !need_update && !self.ctx.dry_run {
            tracing::info!(
                task = %task_name,
                "IP unchanged, DNS records match, and heartbeat interval not reached, skipping request"
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
                tracing::error!(task = %task_name, "Task has no request configuration");
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
                if !self.ctx.dry_run {
                    // Update state store
                    self.ctx
                        .state_store
                        .update(task_name, v4_str.clone(), v6_str.clone(), now);

                    // Dispatch notification
                    let old_ip = format!("v4: {:?}, v6: {:?}", cached.ipv4, cached.ipv6);
                    let new_ip = format!("v4: {:?}, v6: {:?}", v4_str, v6_str);
                    self.ctx
                        .notifier
                        .dispatch(NotificationEvent::Change {
                            task_name,
                            old_ip: &old_ip,
                            new_ip: &new_ip,
                        })
                        .await;

                    self.ctx
                        .notifier
                        .dispatch(NotificationEvent::Recovery {
                            task_name,
                            current_ip: &new_ip,
                        })
                        .await;
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
                    task = %task_name,
                    error = %err_msg,
                    "Failed to execute DDNS update request"
                );
                if !self.ctx.dry_run {
                    self.ctx
                        .notifier
                        .dispatch(NotificationEvent::Failure {
                            task_name,
                            error_message: &err_msg,
                        })
                        .await;
                }

                // Check condition: update failed AND DNS is inconsistent
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
                                    task = %task_name,
                                    domain = %domain,
                                    error = %dns_err,
                                    "Failed to query DNS record after update failure; treating as inconsistent"
                                );
                                true
                            }
                        }
                    };

                    if dns_mismatch {
                        tracing::warn!(
                            task = %task_name,
                            domain = %domain,
                            "Update failed and DNS record is inconsistent: requesting shortened retry interval"
                        );
                        should_shorten = true;
                    } else {
                        tracing::info!(
                            task = %task_name,
                            domain = %domain,
                            "Update failed but DNS record already matches actual IP: maintaining normal interval"
                        );
                    }
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

    /// Execute a single round of interface IP detection, driving all bound tasks.
    pub async fn run_round(&self) -> InterfaceRunOutcome {
        let iface_name = &self.config.name;
        tracing::info!(interface = %iface_name, "Starting IP resolution for interface");

        let (v4_opt, v6_opt) = match self.resolver.resolve().await {
            Ok(ips) => ips,
            Err(e) => {
                let err = RdnsError::IpFetch {
                    task: iface_name.clone(),
                    source: e,
                };
                tracing::error!(
                    interface = %iface_name,
                    error = %err,
                    "Failed to resolve IP for interface"
                );
                return InterfaceRunOutcome {
                    should_shorten_interval: false,
                    has_error: true,
                };
            }
        };

        tracing::info!(
            interface = %iface_name,
            ipv4 = ?v4_opt.map(|ip| ip.to_string()),
            ipv6 = ?v6_opt.map(|ip| ip.to_string()),
            "IP addresses resolved successfully for interface"
        );

        let mut has_error = false;
        let mut should_shorten_interval = false;
        for task in &self.tasks {
            let outcome = task.execute_with_ip(v4_opt, v6_opt).await;
            if outcome.should_shorten_interval {
                should_shorten_interval = true;
            }
            if let Some(e) = outcome.error {
                tracing::error!(
                    interface = %iface_name,
                    task = %task.name(),
                    error = %e,
                    "Task execution failed"
                );
                has_error = true;
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
            interface = %iface_name,
            interval_secs = self.normal_interval.as_secs(),
            retry_interval_secs = self.retry_interval.as_secs(),
            bound_tasks_count = self.tasks.len(),
            "Starting periodic interface IP resolution loop"
        );

        loop {
            if token.is_cancelled() {
                tracing::info!(
                    interface = %iface_name,
                    "Interface cancellation token set, terminating loop"
                );
                break;
            }

            let outcome = self.run_round().await;
            if outcome.has_error {
                tracing::warn!(
                    interface = %iface_name,
                    "Interface execution encountered an error in loop"
                );
            }

            let next_interval = if outcome.should_shorten_interval {
                tracing::info!(
                    interface = %iface_name,
                    retry_interval_secs = self.retry_interval.as_secs(),
                    "Update failed and DNS record is inconsistent: entering shortened retry interval"
                );
                self.retry_interval
            } else {
                self.normal_interval
            };

            tokio::select! {
                _ = token.cancelled() => {
                    tracing::info!(
                        interface = %iface_name,
                        "Interface received cancellation during sleep, shutting down immediately"
                    );
                    break;
                }
                _ = tokio::time::sleep(next_interval) => {}
            }
        }

        tracing::info!(interface = %iface_name, "Interface loop exited cleanly");
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
                method: "GET".to_string(),
                url: "http://127.0.0.1:1/unreachable".to_string(), // port 1 fails fast
                headers: HashMap::new(),
                body: None,
                success_regex: None,
                success_contains: vec![],
                tls_insecure: false,
                proxy: None,
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
                method: "GET".to_string(),
                url: "http://example.com".to_string(),
                headers: HashMap::new(),
                body: None,
                success_regex: None,
                success_contains: vec![],
                tls_insecure: false,
                proxy: None,
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
}
