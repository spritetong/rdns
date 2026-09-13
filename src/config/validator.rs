// Copyright (c) 2026 Sprite Tong <spritetong@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later
// rdns is licensed under the GNU GPL v3.0 or later.

//! Validation rules for configuration entities.

use crate::config::model::Config;
use crate::error::ConfigError;
use regex::Regex;
use std::collections::HashSet;

/// Validate structural and business rules of the parsed configuration,
/// resolving and filling default templates for tasks with predefined providers.
pub fn validate_config(config: &mut Config) -> Result<(), ConfigError> {
    if config.interfaces.is_empty() {
        return Err(ConfigError::Validation {
            field: "interfaces".to_string(),
            message: "Configuration must contain at least one interface in 'interfaces'"
                .to_string(),
        });
    }

    if config.tasks.is_empty() {
        return Err(ConfigError::Validation {
            field: "tasks".to_string(),
            message: "Configuration must contain at least one task in 'tasks'".to_string(),
        });
    }

    if config.global.interval == 0 {
        return Err(ConfigError::Validation {
            field: "global.interval".to_string(),
            message: "Interval must be greater than 0".to_string(),
        });
    }

    if config.global.retry_interval == 0 {
        return Err(ConfigError::Validation {
            field: "global.retry_interval".to_string(),
            message: "Retry interval must be greater than 0".to_string(),
        });
    }

    if config.global.timeout == 0 {
        return Err(ConfigError::Validation {
            field: "global.timeout".to_string(),
            message: "Timeout must be greater than 0".to_string(),
        });
    }

    if config.global.shutdown_timeout == 0 {
        return Err(ConfigError::Validation {
            field: "global.shutdown_timeout".to_string(),
            message: "Shutdown timeout must be greater than 0".to_string(),
        });
    }

    if config.global.worker_threads == Some(0) {
        return Err(ConfigError::Validation {
            field: "global.worker_threads".to_string(),
            message: "Worker threads must be greater than 0".to_string(),
        });
    }

    let valid_levels = ["trace", "debug", "info", "warn", "error", "off"];
    if !valid_levels.contains(&config.global.log_level.to_lowercase().as_str()) {
        return Err(ConfigError::Validation {
            field: "global.log_level".to_string(),
            message: format!(
                "Invalid log level '{}'; must be one of: {}",
                config.global.log_level,
                valid_levels.join(", ")
            ),
        });
    }

    if let Some(ref dns) = config.global.dns_server {
        validate_dns_server(dns, "global.dns_server")?;
    }

    if let Some(ref cacerts) = config.global.cacerts {
        if cacerts.trim().is_empty() {
            return Err(ConfigError::Validation {
                field: "global.cacerts".to_string(),
                message: "CA certificates file path cannot be empty".to_string(),
            });
        }
        if !std::path::Path::new(cacerts).exists() {
            return Err(ConfigError::Validation {
                field: "global.cacerts".to_string(),
                message: format!("CA certificates file '{}' does not exist", cacerts),
            });
        }
    }

    let mut iface_names = HashSet::new();
    for iface in &config.interfaces {
        if iface.name.trim().is_empty() {
            return Err(ConfigError::Validation {
                field: "interfaces[].name".to_string(),
                message: "Interface name cannot be empty".to_string(),
            });
        }
        if !iface_names.insert(iface.name.clone()) {
            return Err(ConfigError::Validation {
                field: "interfaces[].name".to_string(),
                message: format!("Duplicate interface name '{}' found", iface.name),
            });
        }

        if iface.interval == Some(0) {
            return Err(ConfigError::Validation {
                field: format!("interfaces[{}].interval", iface.name),
                message: "Interval must be greater than 0".to_string(),
            });
        }

        if iface.retry_interval == Some(0) {
            return Err(ConfigError::Validation {
                field: format!("interfaces[{}].retry_interval", iface.name),
                message: "Retry interval must be greater than 0".to_string(),
            });
        }

        if let Some(ref dns) = iface.dns_server {
            validate_dns_server(dns, &format!("interfaces[{}].dns_server", iface.name))?;
        }

        let v4_enabled = iface.ipv4.as_ref().map(|s| s.enabled).unwrap_or(false);
        let v6_enabled = iface.ipv6.as_ref().map(|s| s.enabled).unwrap_or(false);

        if !v4_enabled && !v6_enabled {
            return Err(ConfigError::Validation {
                field: format!("interfaces[{}].ip", iface.name),
                message: "At least one of 'ipv4' or 'ipv6' must be enabled for interface"
                    .to_string(),
            });
        }

        // Validate IPv4 config
        if let Some(ref v4) = iface.ipv4
            && v4.enabled
        {
            match v4.source.as_str() {
                "remote" => {
                    if v4.urls.is_empty() {
                        return Err(ConfigError::Validation {
                            field: format!("interfaces[{}].ipv4.urls", iface.name),
                            message: "Remote IPv4 source requires at least one URL".to_string(),
                        });
                    }
                }
                "interface" => {
                    let iface_pattern = v4.interface.as_deref().unwrap_or("");
                    if iface_pattern.is_empty() {
                        return Err(ConfigError::Validation {
                            field: format!("interfaces[{}].ipv4.interface", iface.name),
                            message: "Interface IPv4 source requires an interface name or pattern"
                                .to_string(),
                        });
                    }
                    if let Err(e) = Regex::new(iface_pattern) {
                        return Err(ConfigError::Validation {
                            field: format!("interfaces[{}].ipv4.interface", iface.name),
                            message: format!(
                                "Invalid interface regex pattern '{}': {}",
                                iface_pattern, e
                            ),
                        });
                    }
                }
                other => {
                    return Err(ConfigError::Validation {
                        field: format!("interfaces[{}].ipv4.source", iface.name),
                        message: format!(
                            "Unsupported IP source '{}', must be 'remote' or 'interface'",
                            other
                        ),
                    });
                }
            }
        }

        // Validate IPv6 config
        if let Some(ref v6) = iface.ipv6
            && v6.enabled
        {
            match v6.source.as_str() {
                "remote" => {
                    if v6.urls.is_empty() {
                        return Err(ConfigError::Validation {
                            field: format!("interfaces[{}].ipv6.urls", iface.name),
                            message: "Remote IPv6 source requires at least one URL".to_string(),
                        });
                    }
                }
                "interface" => {
                    let iface_pattern = v6.interface.as_deref().unwrap_or("");
                    if iface_pattern.is_empty() {
                        return Err(ConfigError::Validation {
                            field: format!("interfaces[{}].ipv6.interface", iface.name),
                            message: "Interface IPv6 source requires an interface name or pattern"
                                .to_string(),
                        });
                    }
                    if let Err(e) = Regex::new(iface_pattern) {
                        return Err(ConfigError::Validation {
                            field: format!("interfaces[{}].ipv6.interface", iface.name),
                            message: format!(
                                "Invalid interface regex pattern '{}': {}",
                                iface_pattern, e
                            ),
                        });
                    }
                    if let Some(ref r) = v6.ipv6_regex
                        && let Err(e) = Regex::new(r)
                    {
                        return Err(ConfigError::Validation {
                            field: format!("interfaces[{}].ipv6.ipv6_regex", iface.name),
                            message: format!("Invalid ipv6_regex pattern '{}': {}", r, e),
                        });
                    }
                }
                other => {
                    return Err(ConfigError::Validation {
                        field: format!("interfaces[{}].ipv6.source", iface.name),
                        message: format!(
                            "Unsupported IP source '{}', must be 'remote' or 'interface'",
                            other
                        ),
                    });
                }
            }
        }
    }

    let mut task_names = HashSet::new();
    for task in &mut config.tasks {
        if task.name.trim().is_empty() {
            return Err(ConfigError::Validation {
                field: "tasks[].name".to_string(),
                message: "Task name cannot be empty".to_string(),
            });
        }
        if !task_names.insert(task.name.clone()) {
            return Err(ConfigError::Validation {
                field: "tasks[].name".to_string(),
                message: format!("Duplicate task name '{}' found", task.name),
            });
        }

        if task.force_update_interval == Some(0) {
            return Err(ConfigError::Validation {
                field: format!("tasks[{}].force_update_interval", task.name),
                message: "Force update interval must be greater than 0".to_string(),
            });
        }

        // Validate interface reference
        if let Some(ref target_iface) = task.interface
            && !iface_names.contains(target_iface)
        {
            return Err(ConfigError::Validation {
                field: format!("tasks[{}].interface", task.name),
                message: format!(
                    "Referenced interface '{}' is not defined in 'interfaces'",
                    target_iface
                ),
            });
        }

        // Validate provider and populate default request if needed
        if let Some(ref provider_name) = task.provider {
            let p = crate::provider::get_provider(provider_name).ok_or_else(|| {
                ConfigError::Validation {
                    field: format!("tasks[{}].provider", task.name),
                    message: format!(
                        "Unknown provider '{}'. Supported providers: {}",
                        provider_name,
                        crate::provider::supported_providers_str()
                    ),
                }
            })?;

            if p.requires_domain && task.domain.as_deref().unwrap_or("").trim().is_empty() {
                return Err(ConfigError::Validation {
                    field: format!("tasks[{}].domain", task.name),
                    message: format!(
                        "Provider '{}' requires 'domain' to be specified on task '{}'",
                        p.name, task.name
                    ),
                });
            }

            for required_arg in p.required_args {
                let is_valid = task
                    .args
                    .get(*required_arg)
                    .and_then(|v| v.as_ref())
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false);
                if !is_valid {
                    return Err(ConfigError::Validation {
                        field: format!("tasks[{}].args.{}", task.name, required_arg),
                        message: format!(
                            "Provider '{}' requires parameter '{}' in task '{}' args",
                            p.name, required_arg, task.name
                        ),
                    });
                }
            }

            // Cloudflare specific validation for active stack record IDs
            if p.name == "cloudflare" {
                let iface_name = task
                    .interface
                    .as_deref()
                    .unwrap_or(config.interfaces.first().map(|i| i.name.as_str()).unwrap_or(""));
                let target_iface = config.interfaces.iter().find(|i| i.name == iface_name);
                let iface_v4_enabled = target_iface
                    .and_then(|i| i.ipv4.as_ref())
                    .map(|s| s.enabled)
                    .unwrap_or(false);
                let iface_v6_enabled = target_iface
                    .and_then(|i| i.ipv6.as_ref())
                    .map(|s| s.enabled)
                    .unwrap_or(false);

                let v4_active = is_task_protocol_active(task.args.get("ipv4"), iface_v4_enabled);
                let v6_active = is_task_protocol_active(task.args.get("ipv6"), iface_v6_enabled);

                if !v4_active && !v6_active {
                    return Err(ConfigError::Validation {
                        field: format!("tasks[{}].args", task.name),
                        message: format!(
                            "Task '{}' disables both IPv4 and IPv6; at least one must be enabled",
                            task.name
                        ),
                    });
                }

                let has_custom_body = task
                    .request
                    .as_ref()
                    .and_then(|r| r.body.as_ref())
                    .is_some();
                if !has_custom_body {
                    if v4_active {
                        let has_v4_rec = task
                            .args
                            .get("record_id_v4")
                            .and_then(|v| v.as_ref())
                            .map(|s| !s.trim().is_empty())
                            .unwrap_or(false);
                        if !has_v4_rec {
                            return Err(ConfigError::Validation {
                                field: format!("tasks[{}].args.record_id_v4", task.name),
                                message: format!(
                                    "Provider 'cloudflare' requires parameter 'record_id_v4' in task '{}' args when IPv4 is active",
                                    task.name
                                ),
                            });
                        }
                    }

                    if v6_active {
                        let has_v6_rec = task
                            .args
                            .get("record_id_v6")
                            .and_then(|v| v.as_ref())
                            .map(|s| !s.trim().is_empty())
                            .unwrap_or(false);
                        if !has_v6_rec {
                            return Err(ConfigError::Validation {
                                field: format!("tasks[{}].args.record_id_v6", task.name),
                                message: format!(
                                    "Provider 'cloudflare' requires parameter 'record_id_v6' in task '{}' args when IPv6 is active",
                                    task.name
                                ),
                            });
                        }
                    }
                }
            }

            // Populate default request template, merging user request overrides if present
            let mut effective_req = p.default_request();
            if let Some(user_req) = task.request.take() {
                effective_req.merge(user_req);
            }
            task.request = Some(effective_req);
        } else if let Some(ref mut user_req) = task.request {
            user_req.fill_defaults();
        }

        let req = match task.request.as_ref() {
            Some(r) => r,
            None => {
                return Err(ConfigError::Validation {
                    field: format!("tasks[{}]", task.name),
                    message: "Task must specify either 'provider' or 'request'".to_string(),
                });
            }
        };

        // Validate request URL
        if req.url().trim().is_empty() {
            return Err(ConfigError::Validation {
                field: format!("tasks[{}].request.url", task.name),
                message: "Request URL cannot be empty".to_string(),
            });
        }

        // Validate success_regex if present
        if let Some(ref reg) = req.success_regex {
            Regex::new(reg).map_err(|e| ConfigError::Validation {
                field: format!("tasks[{}].request.success_regex", task.name),
                message: format!("Invalid regex pattern '{}': {}", reg, e),
            })?;
        }

        // Validate cacerts file if present
        if let Some(ref cacerts) = req.cacerts {
            if cacerts.trim().is_empty() {
                return Err(ConfigError::Validation {
                    field: format!("tasks[{}].request.cacerts", task.name),
                    message: "CA certificates file path cannot be empty".to_string(),
                });
            }
            if !std::path::Path::new(cacerts).exists() {
                return Err(ConfigError::Validation {
                    field: format!("tasks[{}].request.cacerts", task.name),
                    message: format!("CA certificates file '{}' does not exist", cacerts),
                });
            }
        }
    }

    Ok(())
}

fn is_task_protocol_active(
    task_arg: Option<&Option<String>>,
    iface_protocol_enabled: bool,
) -> bool {
    match task_arg {
        Some(None) => false,
        Some(Some(val)) => {
            let trimmed = val.trim();
            !(trimmed.is_empty()
                || trimmed.eq_ignore_ascii_case("none")
                || trimmed.eq_ignore_ascii_case("false")
                || trimmed.eq_ignore_ascii_case("null")
                || trimmed == "~")
        }
        None => iface_protocol_enabled,
    }
}

fn validate_dns_server(val: &str, field: &str) -> Result<(), ConfigError> {
    if val.trim().is_empty() {
        return Err(ConfigError::Validation {
            field: field.to_string(),
            message: "dns_server cannot be empty".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::*;

    fn test_request(url: &str) -> RequestConfig {
        RequestConfig {
            url: Some(url.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_retry_interval_zero_rejected() {
        let mut config = Config {
            global: GlobalConfig {
                retry_interval: 0,
                ..Default::default()
            },
            interfaces: vec![InterfaceConfig {
                name: "test".to_string(),
                interval: Some(300),
                retry_interval: None,
                dns_server: None,
                ipv4: Some(IpStrategyConfig {
                    enabled: true,
                    source: "remote".to_string(),
                    urls: vec!["http://127.0.0.1".to_string()],
                    interface: None,
                    ipv6_prefix: None,
                    prefer_slaac: false,
                    ipv6_regex: None,
                    allow_private: false,
                }),
                ipv6: None,
            }],
            notification: None,
            tasks: vec![TaskConfig {
                name: "t1".to_string(),
                interface: None,
                force_update_interval: None,
                domain: None,
                provider: None,
                args: Default::default(),
                request: Some(test_request("http://127.0.0.1")),
            }],
        };

        let err = validate_config(&mut config).unwrap_err();
        assert!(
            err.to_string()
                .contains("Retry interval must be greater than 0")
        );
    }

    #[test]
    fn test_provider_fills_default_request() {
        let mut args = std::collections::HashMap::new();
        args.insert("password".to_string(), Some("my_pass".to_string()));

        let mut config = Config {
            global: GlobalConfig::default(),
            interfaces: vec![InterfaceConfig {
                name: "test".to_string(),
                interval: Some(300),
                retry_interval: None,
                dns_server: None,
                ipv4: Some(IpStrategyConfig {
                    enabled: true,
                    source: "remote".to_string(),
                    urls: vec!["http://127.0.0.1".to_string()],
                    interface: None,
                    ipv6_prefix: None,
                    prefer_slaac: false,
                    ipv6_regex: None,
                    allow_private: false,
                }),
                ipv6: None,
            }],
            notification: None,
            tasks: vec![TaskConfig {
                name: "dynu_task".to_string(),
                interface: None,
                force_update_interval: None,
                domain: Some("test.freeddns.org".to_string()),
                provider: Some("dynu".to_string()),
                args,
                request: None,
            }],
        };

        validate_config(&mut config).expect("validation should succeed and fill default request");
        let task = &config.tasks[0];
        let req = task.request.as_ref().expect("request must be populated");
        assert_eq!(req.method(), "GET");
        assert!(req.url().contains("api.dynu.com"));
        assert!(req.url().contains("{{password}}"));
        assert_eq!(req.success_regex.as_deref(), Some("^(good|nochg)"));
    }

    #[test]
    fn test_provider_with_request_overrides_proxy_and_tls_insecure() {
        let mut args = std::collections::HashMap::new();
        args.insert("password".to_string(), Some("my_pass".to_string()));

        let mut config = Config {
            global: GlobalConfig::default(),
            interfaces: vec![InterfaceConfig {
                name: "test".to_string(),
                interval: Some(300),
                retry_interval: None,
                dns_server: None,
                ipv4: Some(IpStrategyConfig {
                    enabled: true,
                    source: "remote".to_string(),
                    urls: vec!["http://127.0.0.1".to_string()],
                    interface: None,
                    ipv6_prefix: None,
                    prefer_slaac: false,
                    ipv6_regex: None,
                    allow_private: false,
                }),
                ipv6: None,
            }],
            notification: None,
            tasks: vec![TaskConfig {
                name: "dynu_task".to_string(),
                interface: None,
                force_update_interval: None,
                domain: Some("test.freeddns.org".to_string()),
                provider: Some("dynu".to_string()),
                args,
                request: Some(RequestConfig {
                    proxy: Some("http://127.0.0.1:7890".to_string()),
                    tls_insecure: Some(true),
                    ..Default::default()
                }),
            }],
        };

        validate_config(&mut config).expect("validation should succeed and merge request");
        let task = &config.tasks[0];
        let req = task.request.as_ref().expect("request must be populated");
        assert_eq!(req.method(), "GET");
        assert!(req.url().contains("api.dynu.com"));
        assert!(req.url().contains("{{password}}"));
        assert_eq!(req.success_regex.as_deref(), Some("^(good|nochg)"));
        assert_eq!(req.proxy.as_deref(), Some("http://127.0.0.1:7890"));
        assert!(req.tls_insecure());
    }

    #[test]
    fn test_provider_with_header_merging() {
        let mut args = std::collections::HashMap::new();
        args.insert("password".to_string(), Some("my_pass".to_string()));

        let mut custom_headers = std::collections::HashMap::new();
        custom_headers.insert("X-Custom-Header".to_string(), "custom_val".to_string());

        let mut config = Config {
            global: GlobalConfig::default(),
            interfaces: vec![InterfaceConfig {
                name: "test".to_string(),
                interval: Some(300),
                retry_interval: None,
                dns_server: None,
                ipv4: Some(IpStrategyConfig {
                    enabled: true,
                    source: "remote".to_string(),
                    urls: vec!["http://127.0.0.1".to_string()],
                    interface: None,
                    ipv6_prefix: None,
                    prefer_slaac: false,
                    ipv6_regex: None,
                    allow_private: false,
                }),
                ipv6: None,
            }],
            notification: None,
            tasks: vec![TaskConfig {
                name: "dynu_task".to_string(),
                interface: None,
                force_update_interval: None,
                domain: Some("test.freeddns.org".to_string()),
                provider: Some("dynu".to_string()),
                args,
                request: Some(RequestConfig {
                    headers: custom_headers,
                    ..Default::default()
                }),
            }],
        };

        validate_config(&mut config).expect("validation should succeed");
        let task = &config.tasks[0];
        let req = task.request.as_ref().expect("request must be populated");
        assert_eq!(
            req.headers.get("X-Custom-Header").map(|s| s.as_str()),
            Some("custom_val")
        );
    }

    #[test]
    fn test_provider_missing_required_args() {
        let mut config = Config {
            global: GlobalConfig::default(),
            interfaces: vec![InterfaceConfig {
                name: "test".to_string(),
                interval: Some(300),
                retry_interval: None,
                dns_server: None,
                ipv4: Some(IpStrategyConfig {
                    enabled: true,
                    source: "remote".to_string(),
                    urls: vec!["http://127.0.0.1".to_string()],
                    interface: None,
                    ipv6_prefix: None,
                    prefer_slaac: false,
                    ipv6_regex: None,
                    allow_private: false,
                }),
                ipv6: None,
            }],
            notification: None,
            tasks: vec![TaskConfig {
                name: "dynu_task".to_string(),
                interface: None,
                force_update_interval: None,
                domain: Some("test.freeddns.org".to_string()),
                provider: Some("dynu".to_string()),
                args: Default::default(), // Missing "password"
                request: None,
            }],
        };

        let err = validate_config(&mut config).unwrap_err();
        assert!(err.to_string().contains("requires parameter 'password'"));
    }

    #[test]
    fn test_task_without_provider_and_request() {
        let mut config = Config {
            global: GlobalConfig::default(),
            interfaces: vec![InterfaceConfig {
                name: "test".to_string(),
                interval: Some(300),
                retry_interval: None,
                dns_server: None,
                ipv4: Some(IpStrategyConfig {
                    enabled: true,
                    source: "remote".to_string(),
                    urls: vec!["http://127.0.0.1".to_string()],
                    interface: None,
                    ipv6_prefix: None,
                    prefer_slaac: false,
                    ipv6_regex: None,
                    allow_private: false,
                }),
                ipv6: None,
            }],
            notification: None,
            tasks: vec![TaskConfig {
                name: "empty_task".to_string(),
                interface: None,
                force_update_interval: None,
                domain: None,
                provider: None,
                args: Default::default(),
                request: None,
            }],
        };

        let err = validate_config(&mut config).unwrap_err();
        assert!(
            err.to_string()
                .contains("must specify either 'provider' or 'request'")
        );
    }

    #[test]
    fn test_zero_timeout_rejected() {
        let mut config = Config {
            global: GlobalConfig {
                timeout: 0,
                ..Default::default()
            },
            interfaces: vec![InterfaceConfig {
                name: "test".to_string(),
                interval: Some(300),
                retry_interval: None,
                dns_server: None,
                ipv4: Some(IpStrategyConfig {
                    enabled: true,
                    source: "remote".to_string(),
                    urls: vec!["http://127.0.0.1".to_string()],
                    interface: None,
                    ipv6_prefix: None,
                    prefer_slaac: false,
                    ipv6_regex: None,
                    allow_private: false,
                }),
                ipv6: None,
            }],
            notification: None,
            tasks: vec![TaskConfig {
                name: "t1".to_string(),
                interface: None,
                force_update_interval: None,
                domain: None,
                provider: None,
                args: Default::default(),
                request: Some(test_request("http://127.0.0.1")),
            }],
        };

        let err = validate_config(&mut config).unwrap_err();
        assert!(err.to_string().contains("Timeout must be greater than 0"));
    }

    #[test]
    fn test_zero_shutdown_timeout_rejected() {
        let mut config = Config {
            global: GlobalConfig {
                shutdown_timeout: 0,
                ..Default::default()
            },
            interfaces: vec![InterfaceConfig {
                name: "test".to_string(),
                interval: Some(300),
                retry_interval: None,
                dns_server: None,
                ipv4: Some(IpStrategyConfig {
                    enabled: true,
                    source: "remote".to_string(),
                    urls: vec!["http://127.0.0.1".to_string()],
                    interface: None,
                    ipv6_prefix: None,
                    prefer_slaac: false,
                    ipv6_regex: None,
                    allow_private: false,
                }),
                ipv6: None,
            }],
            notification: None,
            tasks: vec![TaskConfig {
                name: "t1".to_string(),
                interface: None,
                force_update_interval: None,
                domain: None,
                provider: None,
                args: Default::default(),
                request: Some(test_request("http://127.0.0.1")),
            }],
        };

        let err = validate_config(&mut config).unwrap_err();
        assert!(
            err.to_string()
                .contains("Shutdown timeout must be greater than 0")
        );
    }

    #[test]
    fn test_zero_worker_threads_rejected() {
        let mut config = Config {
            global: GlobalConfig {
                worker_threads: Some(0),
                ..Default::default()
            },
            interfaces: vec![InterfaceConfig {
                name: "test".to_string(),
                interval: Some(300),
                retry_interval: None,
                dns_server: None,
                ipv4: Some(IpStrategyConfig {
                    enabled: true,
                    source: "remote".to_string(),
                    urls: vec!["http://127.0.0.1".to_string()],
                    interface: None,
                    ipv6_prefix: None,
                    prefer_slaac: false,
                    ipv6_regex: None,
                    allow_private: false,
                }),
                ipv6: None,
            }],
            notification: None,
            tasks: vec![TaskConfig {
                name: "t1".to_string(),
                interface: None,
                force_update_interval: None,
                domain: None,
                provider: None,
                args: Default::default(),
                request: Some(test_request("http://127.0.0.1")),
            }],
        };

        let err = validate_config(&mut config).unwrap_err();
        assert!(
            err.to_string()
                .contains("Worker threads must be greater than 0")
        );
    }

    #[test]
    fn test_zero_force_update_interval_rejected() {
        let mut config = Config {
            global: GlobalConfig::default(),
            interfaces: vec![InterfaceConfig {
                name: "test".to_string(),
                interval: Some(300),
                retry_interval: None,
                dns_server: None,
                ipv4: Some(IpStrategyConfig {
                    enabled: true,
                    source: "remote".to_string(),
                    urls: vec!["http://127.0.0.1".to_string()],
                    interface: None,
                    ipv6_prefix: None,
                    prefer_slaac: false,
                    ipv6_regex: None,
                    allow_private: false,
                }),
                ipv6: None,
            }],
            notification: None,
            tasks: vec![TaskConfig {
                name: "t1".to_string(),
                interface: None,
                force_update_interval: Some(0),
                domain: None,
                provider: None,
                args: Default::default(),
                request: Some(test_request("http://127.0.0.1")),
            }],
        };

        let err = validate_config(&mut config).unwrap_err();
        assert!(
            err.to_string()
                .contains("Force update interval must be greater than 0")
        );
    }

    #[test]
    fn test_invalid_interface_regex_rejected() {
        let mut config = Config {
            global: GlobalConfig::default(),
            interfaces: vec![InterfaceConfig {
                name: "test-bad-regex".to_string(),
                interval: Some(300),
                retry_interval: None,
                dns_server: None,
                ipv4: Some(IpStrategyConfig {
                    enabled: true,
                    source: "interface".to_string(),
                    urls: vec![],
                    interface: Some("[unclosed-bracket".to_string()),
                    ipv6_prefix: None,
                    prefer_slaac: false,
                    ipv6_regex: None,
                    allow_private: false,
                }),
                ipv6: None,
            }],
            notification: None,
            tasks: vec![TaskConfig {
                name: "t1".to_string(),
                interface: None,
                force_update_interval: None,
                domain: None,
                provider: None,
                args: Default::default(),
                request: Some(test_request("http://127.0.0.1")),
            }],
        };

        let err = validate_config(&mut config).unwrap_err();
        assert!(err.to_string().contains("Invalid interface regex pattern"));
    }

    #[test]
    fn test_invalid_log_level_rejected() {
        let mut config = Config {
            global: crate::config::model::GlobalConfig {
                log_level: "unsupported_level".to_string(),
                ..Default::default()
            },
            interfaces: vec![InterfaceConfig {
                name: "eth0".to_string(),
                interval: None,
                retry_interval: None,
                dns_server: None,
                ipv4: None,
                ipv6: None,
            }],
            notification: None,
            tasks: vec![TaskConfig {
                name: "t1".to_string(),
                interface: None,
                force_update_interval: None,
                domain: None,
                provider: None,
                args: Default::default(),
                request: Some(test_request("http://127.0.0.1")),
            }],
        };

        let err = validate_config(&mut config).unwrap_err();
        assert!(err.to_string().contains("global.log_level"));
    }

    #[test]
    fn test_cacerts_validation() {
        let mut config = Config {
            global: crate::config::model::GlobalConfig {
                cacerts: Some("non_existent_certs_file.pem".to_string()),
                ..Default::default()
            },
            interfaces: vec![InterfaceConfig {
                name: "eth0".to_string(),
                interval: None,
                retry_interval: None,
                dns_server: None,
                ipv4: None,
                ipv6: None,
            }],
            notification: None,
            tasks: vec![TaskConfig {
                name: "t1".to_string(),
                interface: None,
                force_update_interval: None,
                domain: None,
                provider: None,
                args: Default::default(),
                request: Some(test_request("http://127.0.0.1")),
            }],
        };

        let err = validate_config(&mut config).unwrap_err();
        assert!(err.to_string().contains("does not exist"));

        config.global.cacerts = Some("   ".to_string());
        let err2 = validate_config(&mut config).unwrap_err();
        assert!(err2.to_string().contains("cannot be empty"));
    }

    #[test]
    fn test_cloudflare_validation_dualstack_and_singlestack() {
        let make_cf_config = |args: std::collections::HashMap<String, Option<String>>| Config {
            global: GlobalConfig::default(),
            interfaces: vec![InterfaceConfig {
                name: "dual_iface".to_string(),
                interval: Some(300),
                retry_interval: None,
                dns_server: None,
                ipv4: Some(IpStrategyConfig {
                    enabled: true,
                    source: "remote".to_string(),
                    urls: vec!["http://127.0.0.1".to_string()],
                    interface: None,
                    ipv6_prefix: None,
                    prefer_slaac: false,
                    ipv6_regex: None,
                    allow_private: false,
                }),
                ipv6: Some(IpStrategyConfig {
                    enabled: true,
                    source: "remote".to_string(),
                    urls: vec!["http://[::1]".to_string()],
                    interface: None,
                    ipv6_prefix: None,
                    prefer_slaac: false,
                    ipv6_regex: None,
                    allow_private: false,
                }),
            }],
            notification: None,
            tasks: vec![TaskConfig {
                name: "cf_task".to_string(),
                interface: Some("dual_iface".to_string()),
                force_update_interval: None,
                domain: Some("sub.example.com".to_string()),
                provider: Some("cloudflare".to_string()),
                args,
                request: None,
            }],
        };

        // 1. Dual-stack missing record_id_v4
        let mut args1 = std::collections::HashMap::new();
        args1.insert("token".to_string(), Some("cf_tok".to_string()));
        args1.insert("zone_id".to_string(), Some("cf_zone".to_string()));
        args1.insert("record_id_v6".to_string(), Some("v6_rec".to_string()));
        let mut cfg1 = make_cf_config(args1);
        let err1 = validate_config(&mut cfg1).unwrap_err();
        assert!(err1.to_string().contains("requires parameter 'record_id_v4'"));

        // 2. Dual-stack missing record_id_v6
        let mut args2 = std::collections::HashMap::new();
        args2.insert("token".to_string(), Some("cf_tok".to_string()));
        args2.insert("zone_id".to_string(), Some("cf_zone".to_string()));
        args2.insert("record_id_v4".to_string(), Some("v4_rec".to_string()));
        let mut cfg2 = make_cf_config(args2);
        let err2 = validate_config(&mut cfg2).unwrap_err();
        assert!(err2.to_string().contains("requires parameter 'record_id_v6'"));

        // 3. Dual-stack with both succeeds
        let mut args3 = std::collections::HashMap::new();
        args3.insert("token".to_string(), Some("cf_tok".to_string()));
        args3.insert("zone_id".to_string(), Some("cf_zone".to_string()));
        args3.insert("record_id_v4".to_string(), Some("v4_rec".to_string()));
        args3.insert("record_id_v6".to_string(), Some("v6_rec".to_string()));
        let mut cfg3 = make_cf_config(args3);
        assert!(validate_config(&mut cfg3).is_ok());

        // 4. IPv4-only (ipv6 suppressed via null): requires only record_id_v4
        let mut args4 = std::collections::HashMap::new();
        args4.insert("token".to_string(), Some("cf_tok".to_string()));
        args4.insert("zone_id".to_string(), Some("cf_zone".to_string()));
        args4.insert("record_id_v4".to_string(), Some("v4_rec".to_string()));
        args4.insert("ipv6".to_string(), None); // Suppress IPv6
        let mut cfg4 = make_cf_config(args4);
        assert!(validate_config(&mut cfg4).is_ok());

        // 5. IPv6-only (ipv4 suppressed via null): requires only record_id_v6
        let mut args5 = std::collections::HashMap::new();
        args5.insert("token".to_string(), Some("cf_tok".to_string()));
        args5.insert("zone_id".to_string(), Some("cf_zone".to_string()));
        args5.insert("record_id_v6".to_string(), Some("v6_rec".to_string()));
        args5.insert("ipv4".to_string(), None); // Suppress IPv4
        let mut cfg5 = make_cf_config(args5);
        assert!(validate_config(&mut cfg5).is_ok());

        // 6. Both suppressed fails
        let mut args6 = std::collections::HashMap::new();
        args6.insert("token".to_string(), Some("cf_tok".to_string()));
        args6.insert("zone_id".to_string(), Some("cf_zone".to_string()));
        args6.insert("ipv4".to_string(), None);
        args6.insert("ipv6".to_string(), None);
        let mut cfg6 = make_cf_config(args6);
        let err6 = validate_config(&mut cfg6).unwrap_err();
        assert!(err6.to_string().contains("disables both IPv4 and IPv6"));
    }
}
