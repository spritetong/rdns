//! Configuration data models.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Root configuration entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub global: GlobalConfig,

    #[serde(default)]
    pub notification: Option<NotificationConfig>,

    pub tasks: Vec<TaskConfig>,
}

/// Global settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    #[serde(default = "default_interval")]
    pub interval: u64,

    #[serde(default = "default_timeout")]
    pub timeout: u64,

    #[serde(default = "default_shutdown_timeout")]
    pub shutdown_timeout: u64,

    pub worker_threads: Option<usize>,

    #[serde(default = "default_log_level")]
    pub log_level: String,

    pub proxy: Option<String>,
}

fn default_interval() -> u64 {
    300
}

fn default_timeout() -> u64 {
    10
}

fn default_shutdown_timeout() -> u64 {
    10
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            interval: default_interval(),
            timeout: default_timeout(),
            shutdown_timeout: default_shutdown_timeout(),
            worker_threads: None,
            log_level: default_log_level(),
            proxy: None,
        }
    }
}

/// Task-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskConfig {
    pub name: String,

    pub interval: Option<u64>,

    pub force_update_interval: Option<u64>,

    pub domain: Option<String>,

    pub ipv4: Option<IpStrategyConfig>,

    pub ipv6: Option<IpStrategyConfig>,

    pub request: RequestConfig,
}

/// IP resolution strategy for a single protocol family.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpStrategyConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// "remote" or "interface"
    pub source: String,

    /// URLs for remote resolver
    #[serde(default)]
    pub urls: Vec<String>,

    /// Network interface name pattern (e.g. "eth0", "enp.*")
    pub interface: Option<String>,

    /// IPv6 prefix filter (e.g. "240e:")
    pub ipv6_prefix: Option<String>,

    /// Allow private/internal addresses (default false)
    #[serde(default)]
    pub allow_private: bool,
}

fn default_true() -> bool {
    true
}

/// HTTP request template configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestConfig {
    #[serde(default = "default_method")]
    pub method: String,

    pub url: String,

    #[serde(default)]
    pub headers: HashMap<String, String>,

    pub body: Option<String>,

    #[serde(default)]
    pub success_regex: Option<String>,

    #[serde(default)]
    pub success_contains: Vec<String>,

    #[serde(default)]
    pub tls_insecure: bool,

    pub proxy: Option<String>,
}

fn default_method() -> String {
    "GET".to_string()
}

/// Generic notification settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationConfig {
    pub on_change: Option<WebhookHookConfig>,
    pub on_failure: Option<WebhookHookConfig>,
    pub on_recovery: Option<WebhookHookConfig>,
}

/// A single webhook notification hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookHookConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default = "default_post")]
    pub method: String,

    pub url: String,

    #[serde(default)]
    pub headers: HashMap<String, String>,

    pub body: Option<String>,
}

fn default_post() -> String {
    "POST".to_string()
}
