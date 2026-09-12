// Copyright (c) 2026 Sprite Tong <spritetong@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later
// rdns is licensed under the GNU GPL v3.0 or later.

//! Secure placeholder template replacement.

use crate::error::TemplateError;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default)]
pub struct TemplateContext<'a> {
    pub ipv4: Option<&'a str>,
    pub ipv6: Option<&'a str>,
    pub domain: Option<&'a str>,
    pub timestamp: Option<u64>,
    pub args: Option<&'a std::collections::HashMap<String, String>>,
}

impl<'a> TemplateContext<'a> {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Render a template string by replacing placeholders like `{{ipv4}}` or custom named parameters `{{password}}`.
/// If a placeholder is present in the template but missing in the context,
/// returns `Err(TemplateError::MissingSlot)`.
pub fn render_template(template: &str, ctx: &TemplateContext) -> Result<String, TemplateError> {
    let mut output = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(start_idx) = rest.find("{{") {
        output.push_str(&rest[..start_idx]);
        let after_start = &rest[start_idx + 2..];
        if let Some(end_idx) = after_start.find("}}") {
            let slot = after_start[..end_idx].trim();
            match slot {
                "ipv4" => {
                    let val = ctx.ipv4.ok_or_else(|| TemplateError::MissingSlot {
                        slot: "ipv4".to_string(),
                    })?;
                    output.push_str(val);
                }
                "ipv6" => {
                    let val = ctx.ipv6.ok_or_else(|| TemplateError::MissingSlot {
                        slot: "ipv6".to_string(),
                    })?;
                    output.push_str(val);
                }
                "domain" => {
                    let val = ctx.domain.ok_or_else(|| TemplateError::MissingSlot {
                        slot: "domain".to_string(),
                    })?;
                    output.push_str(val);
                }
                "timestamp" => {
                    let ts = ctx.timestamp.unwrap_or_else(|| {
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs()
                    });
                    output.push_str(&ts.to_string());
                }
                other => {
                    if let Some(val) = ctx.args.and_then(|a| a.get(other)) {
                        output.push_str(val);
                    } else {
                        return Err(TemplateError::MissingSlot {
                            slot: other.to_string(),
                        });
                    }
                }
            }
            rest = &after_start[end_idx + 2..];
        } else {
            // Unclosed {{, push literal
            output.push_str("{{");
            rest = after_start;
        }
    }
    output.push_str(rest);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_template_rendering_success() {
        let ctx = TemplateContext {
            ipv4: Some("1.2.3.4"),
            ipv6: Some("2001:db8::1"),
            domain: Some("example.com"),
            timestamp: Some(1234567890),
            args: None,
        };

        let tmpl =
            "https://api.example.com/update?ip={{ipv4}}&v6={{ipv6}}&d={{domain}}&t={{timestamp}}";
        let res = render_template(tmpl, &ctx).expect("rendering should succeed");
        assert_eq!(
            res,
            "https://api.example.com/update?ip=1.2.3.4&v6=2001:db8::1&d=example.com&t=1234567890"
        );
    }

    #[test]
    fn test_template_custom_args_success() {
        let mut args = HashMap::new();
        args.insert("password".to_string(), "my_secret_pass".to_string());
        args.insert("zone_id".to_string(), "abc123xyz".to_string());

        let ctx = TemplateContext {
            ipv4: Some("1.2.3.4"),
            ipv6: None,
            domain: Some("example.com"),
            timestamp: None,
            args: Some(&args),
        };

        let tmpl = "https://api.example.com/update?domain={{domain}}&ip={{ipv4}}&pass={{password}}&zone={{zone_id}}";
        let res = render_template(tmpl, &ctx).expect("rendering should succeed");
        assert_eq!(
            res,
            "https://api.example.com/update?domain=example.com&ip=1.2.3.4&pass=my_secret_pass&zone=abc123xyz"
        );
    }

    #[test]
    fn test_template_custom_args_missing() {
        let args = HashMap::new();
        let ctx = TemplateContext {
            ipv4: Some("1.2.3.4"),
            ipv6: None,
            domain: Some("example.com"),
            timestamp: None,
            args: Some(&args),
        };

        let tmpl = "https://api.example.com/update?pass={{password}}";
        let err = render_template(tmpl, &ctx).unwrap_err();
        match err {
            TemplateError::MissingSlot { slot } => assert_eq!(slot, "password"),
        }
    }

    #[test]
    fn test_template_missing_slot() {
        let ctx = TemplateContext {
            ipv4: None,
            ipv6: Some("2001:db8::1"),
            domain: Some("example.com"),
            timestamp: None,
            args: None,
        };

        let tmpl = "https://api.example.com/update?ip={{ipv4}}";
        let err = render_template(tmpl, &ctx).unwrap_err();
        match err {
            TemplateError::MissingSlot { slot } => assert_eq!(slot, "ipv4"),
        }
    }
}
