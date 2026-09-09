//! Single task execution logic and polling loop.

use crate::config::TaskConfig;
use crate::engine::{HttpEngine, TemplateContext};
use crate::error::RdnsError;
use crate::ip::TaskIpResolver;
use crate::notification::{NotificationDispatcher, NotificationEvent};
use crate::persistence::StateStore;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio_util::sync::CancellationToken;

pub struct TaskScheduler {
    config: TaskConfig,
    resolver: TaskIpResolver,
    engine: HttpEngine,
    state_store: StateStore,
    notifier: NotificationDispatcher,
    interval: Duration,
    dry_run: bool,
}

impl TaskScheduler {
    pub fn new(
        config: TaskConfig,
        default_interval_secs: u64,
        timeout: Duration,
        engine: HttpEngine,
        state_store: StateStore,
        notifier: NotificationDispatcher,
        dry_run: bool,
    ) -> Self {
        let interval_secs = config.interval.unwrap_or(default_interval_secs);
        let resolver = TaskIpResolver::new(config.ipv4.clone(), config.ipv6.clone(), timeout);

        Self {
            config,
            resolver,
            engine,
            state_store,
            notifier,
            interval: Duration::from_secs(interval_secs),
            dry_run,
        }
    }

    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// Execute a single round of IP detection, diffing, and Webhook updating.
    pub async fn run_once(&self) -> Result<(), RdnsError> {
        let task_name = &self.config.name;
        tracing::info!(task = %task_name, "Starting IP resolution");

        // 1. Resolve IPs
        let v4_opt = self
            .resolver
            .resolve_ipv4()
            .await
            .map_err(|e| RdnsError::IpFetch {
                task: task_name.clone(),
                source: e,
            })?;
        let v6_opt = self
            .resolver
            .resolve_ipv6()
            .await
            .map_err(|e| RdnsError::IpFetch {
                task: task_name.clone(),
                source: e,
            })?;

        let v4_str = v4_opt.map(|ip| ip.to_string());
        let v6_str = v6_opt.map(|ip| ip.to_string());

        tracing::info!(
            task = %task_name,
            ipv4 = ?v4_str,
            ipv6 = ?v6_str,
            "IP addresses resolved successfully"
        );

        // 2. Diff against cached state
        let cached = self.state_store.get(task_name).unwrap_or_default();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut need_update = false;
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
        } else if let Some(force_interval) = self.config.force_update_interval
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
        if !need_update && !self.dry_run {
            tracing::info!(
                task = %task_name,
                "IP unchanged and heartbeat interval not reached, skipping request"
            );
            return Ok(());
        }

        // 3. Build Template Context
        let ctx = TemplateContext {
            ipv4: v4_str.as_deref(),
            ipv6: v6_str.as_deref(),
            domain: self.config.domain.as_deref(),
            timestamp: Some(now),
        };

        // 4. Execute HTTP Webhook
        match self
            .engine
            .executor()
            .execute(task_name, &self.config.request, &ctx, self.dry_run)
            .await
        {
            Ok(()) => {
                if !self.dry_run {
                    // Update state store
                    self.state_store
                        .update(task_name, v4_str.clone(), v6_str.clone(), now);

                    // Dispatch notification
                    let old_ip = format!("v4: {:?}, v6: {:?}", cached.ipv4, cached.ipv6);
                    let new_ip = format!("v4: {:?}, v6: {:?}", v4_str, v6_str);
                    self.notifier
                        .dispatch(NotificationEvent::Change {
                            task_name,
                            old_ip: &old_ip,
                            new_ip: &new_ip,
                        })
                        .await;

                    self.notifier
                        .dispatch(NotificationEvent::Recovery {
                            task_name,
                            current_ip: &new_ip,
                        })
                        .await;
                }
                Ok(())
            }
            Err(e) => {
                let err_msg = e.to_string();
                tracing::error!(
                    task = %task_name,
                    error = %err_msg,
                    "Failed to execute DDNS update request"
                );
                if !self.dry_run {
                    self.notifier
                        .dispatch(NotificationEvent::Failure {
                            task_name,
                            error_message: &err_msg,
                        })
                        .await;
                }
                Err(e)
            }
        }
    }

    /// Asynchronous loop executing periodically until cancellation token is triggered.
    pub async fn run_loop(&self, token: CancellationToken) {
        let task_name = self.config.name.clone();
        tracing::info!(
            task = %task_name,
            interval_secs = self.interval.as_secs(),
            "Starting periodic task loop"
        );

        loop {
            if token.is_cancelled() {
                tracing::info!(task = %task_name, "Task cancellation token set, terminating loop");
                break;
            }

            if let Err(e) = self.run_once().await {
                tracing::warn!(task = %task_name, error = %e, "Task execution encountered an error in loop");
            }

            tokio::select! {
                _ = token.cancelled() => {
                    tracing::info!(task = %task_name, "Task received cancellation during sleep, shutting down immediately");
                    break;
                }
                _ = tokio::time::sleep(self.interval) => {}
            }
        }

        tracing::info!(task = %task_name, "Task loop exited cleanly");
    }
}
