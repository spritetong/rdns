//! Scheduler service managing multiple interface IP tasks and their bound Webhooks.

mod task;

pub use task::{InterfaceScheduler, TaskExecutor};

use crate::config::Config;
use crate::engine::HttpEngine;
use crate::error::RdnsError;
use crate::lifecycle::{LifecycleAction, LifecycleManager};
use crate::notification::NotificationDispatcher;
use crate::persistence::StateStore;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

pub struct SchedulerService {
    interfaces: Vec<Arc<InterfaceScheduler>>,
    lifecycle: Arc<LifecycleManager>,
}

impl SchedulerService {
    pub fn new(
        config: &Config,
        state_store: StateStore,
        lifecycle: Arc<LifecycleManager>,
        dry_run: bool,
    ) -> Result<Self, RdnsError> {
        let engine = HttpEngine::new(&config.global)?;
        let notifier = NotificationDispatcher::new(config.notification.clone(), engine.clone());
        let default_interval = config.global.interval;
        let default_retry_interval = config.global.retry_interval;
        let timeout = Duration::from_secs(config.global.timeout);

        // Group tasks by interface name.
        // If task does not specify an interface, default to the first interface in config.interfaces.
        let default_iface_name = config
            .interfaces
            .first()
            .map(|i| i.name.as_str())
            .ok_or_else(|| {
                RdnsError::Assertion("Configuration contains no network interfaces".to_string())
            })?;
        let mut tasks_by_iface: HashMap<String, Vec<Arc<TaskExecutor>>> = HashMap::new();
        let dns_resolver = crate::ip::DnsResolver::new();

        for task_cfg in &config.tasks {
            let iface_name = task_cfg.interface.as_deref().unwrap_or(default_iface_name);
            let iface_cfg = config.interfaces.iter().find(|i| i.name == iface_name);
            let dns_server = iface_cfg
                .and_then(|i| i.dns_server.clone())
                .or_else(|| config.global.dns_server.clone());
            let normal_interval_secs = iface_cfg
                .and_then(|i| i.interval)
                .unwrap_or(default_interval);

            let task_ctx = task::TaskContext {
                engine: engine.clone(),
                state_store: state_store.clone(),
                notifier: notifier.clone(),
                dns_resolver: dns_resolver.clone(),
                dns_server,
                timeout,
                normal_interval_secs,
                dry_run,
            };
            let executor = Arc::new(TaskExecutor::new(task_cfg.clone(), task_ctx));
            tasks_by_iface
                .entry(iface_name.to_string())
                .or_default()
                .push(executor);
        }

        let mut interfaces = Vec::new();
        for iface_cfg in &config.interfaces {
            let bound_tasks = tasks_by_iface.remove(&iface_cfg.name).unwrap_or_default();
            let s = InterfaceScheduler::new(
                iface_cfg.clone(),
                bound_tasks,
                default_interval,
                default_retry_interval,
                timeout,
            );
            interfaces.push(Arc::new(s));
        }

        Ok(Self {
            interfaces,
            lifecycle,
        })
    }

    /// Run all interface IP resolutions and their bound tasks once (e.g. for --once or --dry-run).
    pub async fn run_once(&self) -> Result<(), RdnsError> {
        let mut has_error = false;
        for s in &self.interfaces {
            if let Err(e) = s.run_once().await {
                tracing::error!("[{}] Interface failed in run_once: {}", s.name(), e);
                has_error = true;
            }
        }

        if has_error {
            Err(RdnsError::Assertion(
                "One or more tasks failed during run_once".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    /// Spawn each interface IP task into the TaskManager and wait for OS signal.
    pub async fn run_daemon(&self) -> LifecycleAction {
        for s in &self.interfaces {
            let iface_clone = Arc::clone(s);
            let child_token = self.lifecycle.child_token();
            let task_name = format!("iface-{}", s.name());

            self.lifecycle.task_manager().spawn(task_name, async move {
                iface_clone.run_loop(child_token).await;
            });
        }

        tracing::info!("Starting main loop");
        let action = self.lifecycle.wait_and_drain().await;
        match action {
            LifecycleAction::Shutdown => tracing::info!("RDNS daemon stopped cleanly"),
            LifecycleAction::Reload => tracing::info!("RDNS daemon drained for reload"),
        }
        action
    }
}
