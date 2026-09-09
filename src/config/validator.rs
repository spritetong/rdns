//! Validation rules for configuration entities.

use crate::config::model::Config;
use crate::error::ConfigError;
use regex::Regex;
use std::collections::HashSet;

/// Validate structural and business rules of the parsed configuration.
pub fn validate_config(config: &Config) -> Result<(), ConfigError> {
    if config.tasks.is_empty() {
        return Err(ConfigError::Validation {
            field: "tasks".to_string(),
            message: "Configuration must contain at least one task".to_string(),
        });
    }

    let mut names = HashSet::new();
    for task in &config.tasks {
        if task.name.trim().is_empty() {
            return Err(ConfigError::Validation {
                field: "tasks[].name".to_string(),
                message: "Task name cannot be empty".to_string(),
            });
        }
        if !names.insert(&task.name) {
            return Err(ConfigError::Validation {
                field: "tasks[].name".to_string(),
                message: format!("Duplicate task name '{}' found", task.name),
            });
        }

        let v4_enabled = task.ipv4.as_ref().map(|s| s.enabled).unwrap_or(false);
        let v6_enabled = task.ipv6.as_ref().map(|s| s.enabled).unwrap_or(false);

        if !v4_enabled && !v6_enabled {
            return Err(ConfigError::Validation {
                field: format!("tasks[{}].ip", task.name),
                message: "At least one of 'ipv4' or 'ipv6' must be enabled".to_string(),
            });
        }

        // Validate IPv4 config
        if let Some(ref v4) = task.ipv4
            && v4.enabled
        {
            match v4.source.as_str() {
                "remote" => {
                    if v4.urls.is_empty() {
                        return Err(ConfigError::Validation {
                            field: format!("tasks[{}].ipv4.urls", task.name),
                            message: "Remote IPv4 source requires at least one URL".to_string(),
                        });
                    }
                }
                "interface" => {
                    if v4.interface.as_deref().unwrap_or("").is_empty() {
                        return Err(ConfigError::Validation {
                            field: format!("tasks[{}].ipv4.interface", task.name),
                            message: "Interface IPv4 source requires an interface name".to_string(),
                        });
                    }
                }
                other => {
                    return Err(ConfigError::Validation {
                        field: format!("tasks[{}].ipv4.source", task.name),
                        message: format!(
                            "Unsupported IP source '{}', must be 'remote' or 'interface'",
                            other
                        ),
                    });
                }
            }
        }

        // Validate IPv6 config
        if let Some(ref v6) = task.ipv6
            && v6.enabled
        {
            match v6.source.as_str() {
                "remote" => {
                    if v6.urls.is_empty() {
                        return Err(ConfigError::Validation {
                            field: format!("tasks[{}].ipv6.urls", task.name),
                            message: "Remote IPv6 source requires at least one URL".to_string(),
                        });
                    }
                }
                "interface" => {
                    if v6.interface.as_deref().unwrap_or("").is_empty() {
                        return Err(ConfigError::Validation {
                            field: format!("tasks[{}].ipv6.interface", task.name),
                            message: "Interface IPv6 source requires an interface name".to_string(),
                        });
                    }
                }
                other => {
                    return Err(ConfigError::Validation {
                        field: format!("tasks[{}].ipv6.source", task.name),
                        message: format!(
                            "Unsupported IP source '{}', must be 'remote' or 'interface'",
                            other
                        ),
                    });
                }
            }
        }

        // Validate request URL
        if task.request.url.trim().is_empty() {
            return Err(ConfigError::Validation {
                field: format!("tasks[{}].request.url", task.name),
                message: "Request URL cannot be empty".to_string(),
            });
        }

        // Validate success_regex if present
        if let Some(ref reg) = task.request.success_regex {
            Regex::new(reg).map_err(|e| ConfigError::Validation {
                field: format!("tasks[{}].request.success_regex", task.name),
                message: format!("Invalid regex pattern '{}': {}", reg, e),
            })?;
        }
    }

    Ok(())
}
