//! State management and atomic persistence storage.

mod atomic_file;

use atomic_file::write_atomic;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Cached historical state for a single task.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskState {
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
    pub last_success_time: Option<u64>,
}

#[derive(Clone)]
pub struct StateStore {
    path: PathBuf,
    states: Arc<RwLock<HashMap<String, TaskState>>>,
    tx: Option<mpsc::UnboundedSender<()>>,
}

impl StateStore {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        let path_buf = path.as_ref().to_path_buf();
        let states = Arc::new(RwLock::new(HashMap::new()));

        let tx = if tokio::runtime::Handle::try_current().is_ok() {
            let (tx, rx) = mpsc::unbounded_channel();
            let bg_path = path_buf.clone();
            let bg_states = Arc::clone(&states);
            tokio::spawn(Self::run_writer(bg_path, bg_states, rx));
            Some(tx)
        } else {
            None
        };

        let store = Self {
            path: path_buf,
            states,
            tx,
        };
        store.load();
        store
    }

    /// Background actor draining and debouncing write requests.
    async fn run_writer(
        path: PathBuf,
        states: Arc<RwLock<HashMap<String, TaskState>>>,
        mut rx: mpsc::UnboundedReceiver<()>,
    ) {
        while rx.recv().await.is_some() {
            // Debounce rapid bursts from concurrent tasks
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            while rx.try_recv().is_ok() {}

            let path_clone = path.clone();
            let states_clone = Arc::clone(&states);
            let _ = tokio::task::spawn_blocking(move || {
                Self::save_snapshot(&path_clone, &states_clone);
            })
            .await;
        }
    }

    /// Load existing state from JSON file if available.
    pub fn load(&self) {
        if self.path.exists()
            && let Ok(content) = std::fs::read_to_string(&self.path)
            && let Ok(map) = serde_json::from_str::<HashMap<String, TaskState>>(&content)
        {
            *self.states.write() = map;
            tracing::info!(
                path = %self.path.display(),
                "Loaded previous IP state records"
            );
        }
    }

    /// Retrieve the current cached state for a task.
    pub fn get(&self, task_name: &str) -> Option<TaskState> {
        self.states.read().get(task_name).cloned()
    }

    /// Update state for a task and notify background worker for asynchronous disk persistence.
    pub fn update(
        &self,
        task_name: &str,
        ipv4: Option<String>,
        ipv6: Option<String>,
        timestamp: u64,
    ) {
        {
            let mut map = self.states.write();
            map.insert(
                task_name.to_string(),
                TaskState {
                    ipv4,
                    ipv6,
                    last_success_time: Some(timestamp),
                },
            );
        }

        if let Some(ref tx) = self.tx {
            if tx.send(()).is_err() {
                // Fallback to synchronous save if background channel is closed
                self.save();
            }
        } else {
            self.save();
        }
    }

    /// Save state in-memory snapshot to disk atomically.
    pub fn save(&self) {
        Self::save_snapshot(&self.path, &self.states);
    }

    fn save_snapshot(path: &Path, states: &RwLock<HashMap<String, TaskState>>) {
        let json_bytes = {
            let map = states.read();
            match serde_json::to_vec_pretty(&*map) {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to serialize state to JSON");
                    return;
                }
            }
        };

        if let Err(e) = write_atomic(path, &json_bytes) {
            tracing::error!(path = %path.display(), error = %e, "Failed to atomically save state file");
        } else {
            tracing::debug!(path = %path.display(), "State file atomically updated");
        }
    }
}
