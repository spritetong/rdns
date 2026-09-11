//! Generic webhook notification system.

mod events;

pub use events::NotificationEvent;

use crate::config::{NotificationConfig, WebhookHookConfig};
use crate::engine::HttpEngine;
use reqwest::Method;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct NotificationDispatcher {
    config: Option<NotificationConfig>,
    engine: HttpEngine,
    failure_counts: Arc<parking_lot::Mutex<HashMap<String, u32>>>,
}

impl NotificationDispatcher {
    pub fn new(config: Option<NotificationConfig>, engine: HttpEngine) -> Self {
        Self {
            config,
            engine,
            failure_counts: Arc::new(parking_lot::Mutex::new(HashMap::new())),
        }
    }

    pub async fn dispatch(&self, event: NotificationEvent<'_>) {
        let cfg = match self.config {
            Some(ref c) => c,
            None => return,
        };

        match event {
            NotificationEvent::Change {
                task_name,
                old_ip,
                new_ip,
            } => {
                // Reset failure count on change
                self.failure_counts.lock().remove(task_name);

                if let Some(ref hook) = cfg.on_change
                    && hook.enabled
                {
                    let mut extra = HashMap::new();
                    extra.insert("task_name", task_name);
                    extra.insert("status", "changed");
                    extra.insert("old_ip", old_ip);
                    extra.insert("new_ip", new_ip);
                    self.send_hook(hook, &extra).await;
                }
            }
            NotificationEvent::Failure {
                task_name,
                error_message,
            } => {
                let is_first_failure = {
                    let mut guard = self.failure_counts.lock();
                    let count = guard.entry(task_name.to_string()).or_insert(0);
                    *count = count.saturating_add(1);
                    *count == 1
                };

                // Alert on first failure, suppress consecutive
                if is_first_failure
                    && let Some(ref hook) = cfg.on_failure
                    && hook.enabled
                {
                    let mut extra = HashMap::new();
                    extra.insert("task_name", task_name);
                    extra.insert("status", "failed");
                    extra.insert("error_message", error_message);
                    self.send_hook(hook, &extra).await;
                }
            }
            NotificationEvent::Recovery {
                task_name,
                current_ip,
            } => {
                let had_previous_failures = {
                    let mut guard = self.failure_counts.lock();
                    guard.remove(task_name).unwrap_or(0) > 0
                };

                if had_previous_failures
                    && let Some(ref hook) = cfg.on_recovery
                    && hook.enabled
                {
                    let mut extra = HashMap::new();
                    extra.insert("task_name", task_name);
                    extra.insert("status", "recovered");
                    extra.insert("new_ip", current_ip);
                    self.send_hook(hook, &extra).await;
                }
            }
        }
    }

    async fn send_hook(&self, hook: &WebhookHookConfig, params: &HashMap<&str, &str>) {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string();

        let render_str = |s: &str| -> String {
            let mut res = s.to_string();
            for (k, v) in params {
                res = res.replace(&format!("{{{{{}}}}}", k), v);
            }
            res.replace("{{timestamp}}", &ts)
        };

        let rendered_url = render_str(&hook.url);
        let mut headers = HeaderMap::new();
        for (k, v) in &hook.headers {
            if let (Ok(h_name), Ok(h_val)) = (
                HeaderName::from_str(k),
                HeaderValue::from_str(&render_str(v)),
            ) {
                headers.insert(h_name, h_val);
            }
        }

        let rendered_body = hook.body.as_deref().map(render_str);

        let method = Method::from_str(&hook.method.to_uppercase()).unwrap_or(Method::POST);
        let mut req = self
            .engine
            .executor()
            .client()
            .request(method, &rendered_url)
            .headers(headers);
        if let Some(body) = rendered_body {
            req = req.body(body);
        }

        match req.send().await {
            Ok(resp) => {
                tracing::info!(
                    "Webhook notification sent successfully (HTTP {})",
                    resp.status()
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to send webhook notification: {}",
                    e
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GlobalConfig;

    #[tokio::test]
    async fn test_failure_recovery_lifecycle() {
        let global = GlobalConfig::default();
        let engine = HttpEngine::new(&global).unwrap();
        let dispatcher = NotificationDispatcher::new(Some(NotificationConfig::default()), engine);

        // 1. Initial state: 0 failures
        assert_eq!(dispatcher.failure_counts.lock().get("task1"), None);

        // 2. First failure
        dispatcher
            .dispatch(NotificationEvent::Failure {
                task_name: "task1",
                error_message: "timeout",
            })
            .await;
        assert_eq!(
            dispatcher.failure_counts.lock().get("task1").copied(),
            Some(1)
        );

        // 3. Second failure: count increments
        dispatcher
            .dispatch(NotificationEvent::Failure {
                task_name: "task1",
                error_message: "timeout again",
            })
            .await;
        assert_eq!(
            dispatcher.failure_counts.lock().get("task1").copied(),
            Some(2)
        );

        // 4. Recovery: consumes previous failure count (> 0) and removes the key
        dispatcher
            .dispatch(NotificationEvent::Recovery {
                task_name: "task1",
                current_ip: "1.2.3.4",
            })
            .await;
        assert_eq!(dispatcher.failure_counts.lock().get("task1"), None);

        // 5. Subsequent recovery with no failures: does not trigger recovery again
        dispatcher
            .dispatch(NotificationEvent::Recovery {
                task_name: "task1",
                current_ip: "1.2.3.4",
            })
            .await;
        assert_eq!(dispatcher.failure_counts.lock().get("task1"), None);
    }

    #[tokio::test]
    async fn test_failure_counter_saturating_add() {
        let global = GlobalConfig::default();
        let engine = HttpEngine::new(&global).unwrap();
        let dispatcher = NotificationDispatcher::new(Some(NotificationConfig::default()), engine);

        // Seed with u32::MAX
        dispatcher
            .failure_counts
            .lock()
            .insert("task_max".to_string(), u32::MAX);

        // Dispatch failure: must saturate at u32::MAX and not wrap around to 0
        dispatcher
            .dispatch(NotificationEvent::Failure {
                task_name: "task_max",
                error_message: "fail",
            })
            .await;
        assert_eq!(
            dispatcher.failure_counts.lock().get("task_max").copied(),
            Some(u32::MAX)
        );
    }
}
