//! Asynchronous task manager retaining JoinHandles and handling shutdown drains.

use parking_lot::Mutex;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

type TrackedTask = (String, JoinHandle<()>);
type HandleList = Arc<Mutex<Vec<TrackedTask>>>;

#[derive(Clone, Default)]
pub struct TaskManager {
    handles: HandleList,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            handles: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Spawn a named asynchronous task and track its JoinHandle.
    pub fn spawn<F>(&self, name: String, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let handle = tokio::spawn(future);
        self.handles.lock().push((name, handle));
    }

    /// Gracefully drain all running tasks within the specified timeout duration.
    /// If tasks do not finish within the timeout, force abort remaining handles.
    pub async fn shutdown_all(&self, timeout_duration: Duration) {
        let mut handles = {
            let mut guard = self.handles.lock();
            std::mem::take(&mut *guard)
        };

        if handles.is_empty() {
            return;
        }

        tracing::info!(
            "Waiting up to {}s for {} active tasks to drain",
            timeout_duration.as_secs(),
            handles.len()
        );

        let drain_future = async {
            for (name, handle) in handles.iter_mut() {
                if let Err(e) = handle.await {
                    tracing::warn!("[{}] Task terminated with error during join: {}", name, e);
                }
            }
        };

        if tokio::time::timeout(timeout_duration, drain_future)
            .await
            .is_err()
        {
            tracing::warn!(
                "Shutdown timed out after {}s, aborting remaining tasks",
                timeout_duration.as_secs()
            );
            for (name, handle) in &handles {
                if !handle.is_finished() {
                    tracing::warn!("[{}] Forcibly aborting task", name);
                    handle.abort();
                }
            }
        } else {
            tracing::info!("All active tasks drained cleanly");
        }
    }
}
