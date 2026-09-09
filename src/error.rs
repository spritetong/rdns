//! Error types for the RDNS application.

use thiserror::Error;

/// Top-level error type for RDNS operations.
#[derive(Error, Debug)]
pub enum RdnsError {
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("IP detection failed for task '{task}': {source}")]
    IpFetch { task: String, source: IpFetchError },

    #[error("Template rendering error: {0}")]
    Template(#[from] TemplateError),

    #[error("HTTP request error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Response verification assertion failed: {0}")]
    Assertion(String),

    #[error("Persistence I/O error: {0}")]
    Persistence(#[from] std::io::Error),

    #[error("Task was cancelled by shutdown token")]
    #[allow(dead_code)]
    Cancelled,
}

/// Errors occurring during configuration loading, parsing, and validation.
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read configuration file '{path}': {source}")]
    ReadFile {
        path: String,
        source: std::io::Error,
    },

    #[error("YAML deserialization error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("Environment variable expansion error: {0}")]
    EnvLookup(String),

    #[error("Invalid configuration field '{field}': {message}")]
    Validation { field: String, message: String },
}

/// Errors occurring during IP address discovery.
#[derive(Error, Debug)]
pub enum IpFetchError {
    #[error("All remote query sources exhausted without obtaining an IP")]
    AllSourcesExhausted,

    #[error("Network interface '{name}' was not found on this system")]
    InterfaceNotFound { name: String },

    #[error("No valid public IP found on interface '{name}'")]
    NoPublicIpFound { name: String },

    #[error("I/O error during IP lookup: {0}")]
    Io(#[from] std::io::Error),
}

/// Errors occurring during URL/Body template interpolation.
#[derive(Error, Debug)]
pub enum TemplateError {
    #[error("Template placeholder '{{{{{slot}}}}}' required but not available in context")]
    MissingSlot { slot: String },
}
