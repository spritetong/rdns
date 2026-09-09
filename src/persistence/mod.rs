//! State management and atomic persistence storage.

mod atomic_file;

use atomic_file::write_atomic;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    states: Arc<Mutex<HashMap<String, TaskState>>>,
}

impl StateStore {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        let path_buf = path.as_ref().to_path_buf();
        let store = Self {
            path: path_buf,
            states: Arc::new(Mutex::new(HashMap::new())),
        };
        store.load();
        store
    }

    /// Load existing state from JSON file if available.
    pub fn load(&self) {
        if self.path.exists()
            && let Ok(content) = std::fs::read_to_string(&self.path)
            && let Ok(map) = serde_json::from_str::<HashMap<String, TaskState>>(&content)
        {
            *self.states.lock() = map;
            tracing::info!(
                path = %self.path.display(),
                "Loaded previous IP state records"
            );
        }
    }

    /// Retrieve the current cached state for a task.
    pub fn get(&self, task_name: &str) -> Option<TaskState> {
        self.states.lock().get(task_name).cloned()
    }

    /// Update state for a task and immediately flush to disk via atomic write.
    pub fn update(
        &self,
        task_name: &str,
        ipv4: Option<String>,
        ipv6: Option<String>,
        timestamp: u64,
    ) {
        {
            let mut map = self.states.lock();
            map.insert(
                task_name.to_string(),
                TaskState {
                    ipv4,
                    ipv6,
                    last_success_time: Some(timestamp),
                },
            );
        }
        self.save();
    }

    /// Save state in-memory snapshot to disk atomically.
    pub fn save(&self) {
        let json_bytes = {
            let map = self.states.lock();
            match serde_json::to_vec_pretty(&*map) {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to serialize state to JSON");
                    return;
                }
            }
        };

        if let Err(e) = write_atomic(&self.path, &json_bytes) {
            tracing::error!(path = %self.path.display(), error = %e, "Failed to atomically save state file");
        } else {
            tracing::debug!(path = %self.path.display(), "State file atomically updated");
        }
    }
}
