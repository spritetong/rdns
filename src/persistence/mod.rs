//! State management and atomic persistence storage.

mod atomic_file;

use atomic_file::write_atomic;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// Cached historical state for a single task.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskState {
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
    pub last_success_time: Option<u64>,
}

/// Persistence write request, optionally carrying an acknowledgment to support
/// flush-and-wait semantics on shutdown and single-run modes.
struct WriteMsg {
    ack: Option<oneshot::Sender<()>>,
}

#[derive(Clone)]
pub struct StateStore {
    path: PathBuf,
    states: Arc<RwLock<HashMap<String, TaskState>>>,
    tx: Option<mpsc::UnboundedSender<WriteMsg>>,
    write_enabled: bool,
}

impl StateStore {
    /// Create a StateStore with disk persistence writing enabled.
    #[allow(dead_code)]
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self::new_with_write_flag(path, true)
    }

    /// Create a StateStore with explicit control over disk persistence writing.
    ///
    /// When `write_enabled` is false, in-memory state is maintained across task runs
    /// for in-process idempotency, but no files are created or modified on disk.
    pub fn new_with_write_flag<P: AsRef<Path>>(path: P, write_enabled: bool) -> Self {
        let path_buf = path.as_ref().to_path_buf();
        let states = Arc::new(RwLock::new(HashMap::new()));

        let tx = if write_enabled && tokio::runtime::Handle::try_current().is_ok() {
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
            write_enabled,
        };
        store.load();
        store
    }

    /// Whether writing state to disk is enabled.
    #[allow(dead_code)]
    pub fn is_write_enabled(&self) -> bool {
        self.write_enabled
    }

    /// Background actor draining and debouncing write requests.
    async fn run_writer(
        path: PathBuf,
        states: Arc<RwLock<HashMap<String, TaskState>>>,
        mut rx: mpsc::UnboundedReceiver<WriteMsg>,
    ) {
        while let Some(head) = rx.recv().await {
            let mut acks = Vec::new();
            if let Some(ack) = head.ack {
                acks.push(ack);
            }

            // Debounce rapid bursts from concurrent tasks
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            while let Ok(next) = rx.try_recv() {
                if let Some(ack) = next.ack {
                    acks.push(ack);
                }
            }

            let path_clone = path.clone();
            let states_clone = Arc::clone(&states);
            let _ = tokio::task::spawn_blocking(move || {
                Self::save_snapshot(&path_clone, &states_clone);
            })
            .await;

            // Acknowledge all flushed requests once the snapshot is on disk
            for ack in acks {
                let _ = ack.send(());
            }
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

        if !self.write_enabled {
            return;
        }

        if let Some(ref tx) = self.tx {
            if tx.send(WriteMsg { ack: None }).is_err() {
                // Fallback to synchronous save if background channel is closed
                self.save();
            }
        } else {
            self.save();
        }
    }

    /// Persist all pending in-memory state synchronously and wait for completion.
    ///
    /// Ensures durability of state updates before process exit in `--once`/daemon
    /// shutdown paths, where detached background tasks would otherwise be dropped.
    pub async fn flush(&self) {
        if !self.write_enabled {
            return;
        }

        let Some(tx) = &self.tx else {
            self.save();
            return;
        };

        let (ack_tx, ack_rx) = oneshot::channel();
        if tx.send(WriteMsg { ack: Some(ack_tx) }).is_err() {
            self.save();
            return;
        }
        let _ = ack_rx.await;
    }

    /// Save state in-memory snapshot to disk atomically.
    pub fn save(&self) {
        if !self.write_enabled {
            return;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify `--once`-style exit durability: in-memory updates are persisted and
    /// acknowledged even though the detached background writer is dropped on runtime exit.
    #[tokio::test]
    async fn test_flush_persists_pending_updates_before_exit() {
        let dir = std::env::temp_dir().join(format!("rdns_state_flush_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");

        let store = StateStore::new(&path);
        store.update("task-a", Some("1.2.3.4".into()), None, 1700000000);
        store.flush().await;

        let content = std::fs::read_to_string(&path).expect("state.json must exist after flush");
        let map: HashMap<String, TaskState> = serde_json::from_str(&content).unwrap();
        assert_eq!(map["task-a"].ipv4.as_deref(), Some("1.2.3.4"));
        assert!(map["task-a"].last_success_time.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Idempotent persistence across runs: a store reloaded from disk observes
    /// the previously flushed state.
    #[tokio::test]
    async fn test_reload_observes_flushed_state() {
        let dir = std::env::temp_dir().join(format!("rdns_state_reload_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");

        {
            let store = StateStore::new(&path);
            store.update("task-a", Some("198.51.100.7".into()), None, 1700000100);
            store.flush().await;
        }

        let reloaded = StateStore::new(&path);
        let state = reloaded.get("task-a").unwrap();
        assert_eq!(state.ipv4.as_deref(), Some("198.51.100.7"));
        assert_eq!(state.last_success_time, Some(1700000100));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// When write persistence is disabled, in-memory state is maintained for in-process
    /// deduplication, but no file is created or written to disk.
    #[tokio::test]
    async fn test_disabled_write_state_keeps_memory_without_disk_file() {
        let dir = std::env::temp_dir().join(format!("rdns_state_disabled_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");

        let store = StateStore::new_with_write_flag(&path, false);
        assert!(!store.is_write_enabled());

        store.update("task-b", Some("203.0.113.1".into()), None, 1700000200);
        // Verify in-memory state is immediately accessible
        let mem_state = store
            .get("task-b")
            .expect("in-memory state must be present");
        assert_eq!(mem_state.ipv4.as_deref(), Some("203.0.113.1"));

        store.flush().await;
        // Verify no file was written to disk
        assert!(
            !path.exists(),
            "state.json must not exist when write is disabled"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
