//! Scheduler service managing multiple DDNS tasks.

mod task;

pub use task::TaskScheduler;

use crate::config::Config;
use crate::engine::HttpEngine;
use crate::error::RdnsError;
use crate::lifecycle::LifecycleManager;
use crate::notification::NotificationDispatcher;
use crate::persistence::StateStore;
use std::sync::Arc;
use std::time::Duration;

pub struct SchedulerService {
    schedulers: Vec<Arc<TaskScheduler>>,
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
        let timeout = Duration::from_secs(config.global.timeout);

        let mut schedulers = Vec::new();
        for task_cfg in &config.tasks {
            let s = TaskScheduler::new(
                task_cfg.clone(),
                default_interval,
                timeout,
                engine.clone(),
                state_store.clone(),
                notifier.clone(),
                dry_run,
            );
            schedulers.push(Arc::new(s));
        }

        Ok(Self {
            schedulers,
            lifecycle,
        })
    }

    /// Run all tasks once (e.g. for --once or --dry-run) and return the overall result.
    pub async fn run_once(&self) -> Result<(), RdnsError> {
        let mut has_error = false;
        for s in &self.schedulers {
            if let Err(e) = s.run_once().await {
                tracing::error!(task = %s.name(), error = %e, "Task failed in run_once");
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

    /// Spawn all tasks into the TaskManager and wait for OS shutdown signal.
    pub async fn run_daemon(&self) {
        for s in &self.schedulers {
            let task_clone = Arc::clone(s);
            let child_token = self.lifecycle.child_token();
            let task_name = s.name().to_string();

            self.lifecycle.task_manager().spawn(task_name, async move {
                task_clone.run_loop(child_token).await;
            });
        }

        tracing::info!("All tasks spawned, daemon running. Awaiting OS shutdown signal...");
        self.lifecycle.wait_and_shutdown().await;
        tracing::info!("RDNS daemon stopped cleanly");
    }
}
