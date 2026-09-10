//! Configuration data models.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Root configuration entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub global: GlobalConfig,

    /// Named network interfaces/egress profiles.
    #[serde(default)]
    pub interfaces: Vec<InterfaceConfig>,

    #[serde(default)]
    pub notification: Option<NotificationConfig>,

    pub tasks: Vec<TaskConfig>,
}

/// Named network interface IP query profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceConfig {
    pub name: String,

    pub interval: Option<u64>,

    /// Optional shortened retry interval (seconds) when update fails and DNS is inconsistent
    pub retry_interval: Option<u64>,

    /// Optional DNS server for verifying cloud records (e.g. "8.8.8.8", "1.1.1.1:53")
    pub dns_server: Option<String>,

    pub ipv4: Option<IpStrategyConfig>,

    pub ipv6: Option<IpStrategyConfig>,
}

/// Global settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    #[serde(default = "default_interval")]
    pub interval: u64,

    /// Shortened retry interval in seconds when update fails and DNS is inconsistent (default: 60)
    #[serde(default = "default_retry_interval")]
    pub retry_interval: u64,

    #[serde(default = "default_timeout")]
    pub timeout: u64,

    #[serde(default = "default_shutdown_timeout")]
    pub shutdown_timeout: u64,

    pub worker_threads: Option<usize>,

    #[serde(default = "default_log_level")]
    pub log_level: String,

    pub proxy: Option<String>,

    /// Global default DNS server for verifying cloud records
    pub dns_server: Option<String>,
}

fn default_interval() -> u64 {
    300
}

fn default_retry_interval() -> u64 {
    60
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
            retry_interval: default_retry_interval(),
            timeout: default_timeout(),
            shutdown_timeout: default_shutdown_timeout(),
            worker_threads: None,
            log_level: default_log_level(),
            proxy: None,
            dns_server: None,
        }
    }
}

/// Task-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskConfig {
    pub name: String,

    /// Associated interface name. If omitted, defaults to the first configured interface.
    pub interface: Option<String>,

    pub force_update_interval: Option<u64>,

    pub domain: Option<String>,

    /// Predefined provider name (e.g. "dynu", "dynv6", "duckdns").
    /// If request is omitted, the provider's default request template is used.
    pub provider: Option<String>,

    /// Additional named parameters for URL, header, and body templates (e.g. password, token).
    /// Environment variables like ${DYNU_PASSWORD} are automatically expanded.
    #[serde(default)]
    pub args: HashMap<String, String>,

    /// HTTP request configuration. If omitted, filled from provider's default template.
    #[serde(default)]
    pub request: Option<RequestConfig>,
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

    /// Prefer SLAAC (EUI-64) IPv6 address if multiple exist on the interface (default true)
    #[serde(default = "default_true")]
    pub prefer_slaac: bool,

    /// Optional regex filter for IPv6 address (e.g. ".*6221$")
    pub ipv6_regex: Option<String>,

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
