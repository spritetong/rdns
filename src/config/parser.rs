//! YAML config parsing and environment variable expansion.

use crate::config::model::Config;
use crate::error::ConfigError;
use std::path::Path;

/// Expand `${VAR}` or `${VAR:-default}` patterns in the input text.
pub fn expand_env_vars(input: &str) -> Result<String, ConfigError> {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let mut expr = String::new();
            let mut closed = false;
            for ch in chars.by_ref() {
                if ch == '}' {
                    closed = true;
                    break;
                }
                expr.push(ch);
            }
            if !closed {
                return Err(ConfigError::Validation {
                    field: "config".to_string(),
                    message: "Unclosed '${' in configuration".to_string(),
                });
            }

            let value = if let Some((var_name, default_val)) = expr.split_once(":-") {
                std::env::var(var_name).unwrap_or_else(|_| default_val.to_string())
            } else {
                std::env::var(&expr).map_err(|_| {
                    ConfigError::EnvLookup(format!("Environment variable '{}' is not set", expr))
                })?
            };
            result.push_str(&value);
        } else {
            result.push(c);
        }
    }

    Ok(result)
}

/// Load and parse a configuration file from a filesystem path.
pub fn load_config<P: AsRef<Path>>(path: P) -> Result<Config, ConfigError> {
    let path_ref = path.as_ref();
    let content = std::fs::read_to_string(path_ref).map_err(|e| ConfigError::ReadFile {
        path: path_ref.display().to_string(),
        source: e,
    })?;

    let expanded = expand_env_vars(&content)?;
    let config: Config = serde_yaml::from_str(&expanded).map_err(ConfigError::Yaml)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_env_vars() {
        unsafe {
            std::env::set_var("TEST_RDNS_ENV", "my_secret_token");
        }

        let input = "Authorization: Bearer ${TEST_RDNS_ENV}";
        let res = expand_env_vars(input).expect("expansion should succeed");
        assert_eq!(res, "Authorization: Bearer my_secret_token");

        let default_input = "Pass: ${NON_EXISTING_VAR_123:-fallback_pass}";
        let res_default = expand_env_vars(default_input).expect("default expansion should succeed");
        assert_eq!(res_default, "Pass: fallback_pass");

        let missing_input = "Missing: ${NON_EXISTING_VAR_XYZ}";
        assert!(expand_env_vars(missing_input).is_err());
    }
}
