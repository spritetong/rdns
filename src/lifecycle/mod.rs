//! Lifecycle and graceful shutdown management.

mod signal;
mod task_manager;

pub use signal::SignalListener;
pub use task_manager::TaskManager;
use tokio_util::sync::CancellationToken;

/// Lifecycle coordinator managing root cancellation and task draining.
pub struct LifecycleManager {
    root_token: CancellationToken,
    task_manager: TaskManager,
    shutdown_timeout: std::time::Duration,
}

impl LifecycleManager {
    pub fn new(shutdown_timeout_secs: u64) -> Self {
        Self {
            root_token: CancellationToken::new(),
            task_manager: TaskManager::new(),
            shutdown_timeout: std::time::Duration::from_secs(shutdown_timeout_secs),
        }
    }

    /// Access the root cancellation token.
    #[allow(dead_code)]
    pub fn root_token(&self) -> &CancellationToken {
        &self.root_token
    }

    /// Create a child cancellation token for a submodule or task.
    pub fn child_token(&self) -> CancellationToken {
        self.root_token.child_token()
    }

    /// Access the task manager for spawning and tracking tasks.
    pub fn task_manager(&self) -> &TaskManager {
        &self.task_manager
    }

    /// Wait for OS shutdown signal and coordinate graceful task termination.
    pub async fn wait_and_shutdown(&self) {
        match SignalListener::wait_shutdown_signal().await {
            Ok(sig) => {
                tracing::info!(
                    signal = %sig,
                    "Received OS termination signal, initiating shutdown sequence"
                );
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "Failed to listen for OS signals, initiating shutdown"
                );
            }
        }

        // Cancel all child tasks immediately
        self.root_token.cancel();

        // Drain all tasks with timeout
        self.task_manager.shutdown_all(self.shutdown_timeout).await;
    }
}
