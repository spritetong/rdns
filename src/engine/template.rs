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
    pub args: Option<&'a std::collections::HashMap<String, Option<String>>>,
}

impl<'a> TemplateContext<'a> {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Retrieve the resolved string value of a named slot/variable if present and non-empty.
    pub fn get_value(&self, var: &str) -> Option<std::borrow::Cow<'a, str>> {
        match var {
            "ipv4" => self
                .ipv4
                .filter(|s| !s.trim().is_empty())
                .map(std::borrow::Cow::Borrowed),
            "ipv6" => self
                .ipv6
                .filter(|s| !s.trim().is_empty())
                .map(std::borrow::Cow::Borrowed),
            "domain" => self
                .domain
                .filter(|s| !s.trim().is_empty())
                .map(std::borrow::Cow::Borrowed),
            "timestamp" => self
                .timestamp
                .map(|t| std::borrow::Cow::Owned(t.to_string())),
            other => self
                .args
                .and_then(|a| a.get(other))
                .and_then(|v| v.as_deref())
                .filter(|s| !s.trim().is_empty())
                .map(std::borrow::Cow::Borrowed),
        }
    }
}

/// Render a template string by replacing placeholders like `{{ipv4}}` or custom named parameters `{{password}}`.
/// Also supports conditional patterns: `{{?var:THEN}}` or `{{?var:THEN|ELSE}}`, where `{}` or `{var}` in `THEN`
/// will be replaced with the variable's value.
/// If a standard placeholder is present in the template but missing in the context,
/// returns `Err(TemplateError::MissingSlot)`.
pub fn render_template(template: &str, ctx: &TemplateContext) -> Result<String, TemplateError> {
    let mut output = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(start_idx) = rest.find("{{") {
        output.push_str(&rest[..start_idx]);
        let after_start = &rest[start_idx + 2..];

        // Match closing }} while properly accounting for inner braces like {} or {var}
        let mut depth = 2usize;
        let mut match_end = None;

        for (idx, ch) in after_start.char_indices() {
            if ch == '{' {
                depth += 1;
            } else if ch == '}' {
                depth -= 1;
                if depth == 0 {
                    match_end = Some(idx);
                    break;
                }
            }
        }

        if let Some(end_idx) = match_end {
            let slot = after_start[..end_idx - 1].trim();
            if let Some(cond_expr) = slot.strip_prefix('?') {
                // Conditional pattern: {{?var:THEN_PATTERN}} or {{?var:THEN_PATTERN|ELSE_PATTERN}}
                let (var_name, pattern) = if let Some((v, p)) = cond_expr.split_once(':') {
                    (v.trim(), p)
                } else {
                    return Err(TemplateError::MissingSlot {
                        slot: slot.to_string(),
                    });
                };

                let (then_part, else_part) = match pattern.split_once('|') {
                    Some((t, e)) => (t, e),
                    None => (pattern, ""),
                };

                if let Some(val) = ctx.get_value(var_name) {
                    let intermediate = then_part
                        .replace("{}", &val)
                        .replace(&format!("{{{}}}", var_name), &val);
                    let rendered_then = if intermediate.contains("{{") {
                        render_template(&intermediate, ctx)?
                    } else {
                        intermediate
                    };
                    output.push_str(&rendered_then);
                } else {
                    let rendered_else = if else_part.contains("{{") {
                        render_template(else_part, ctx)?
                    } else {
                        else_part.to_string()
                    };
                    output.push_str(&rendered_else);
                }
            } else {
                match slot {
                    "ipv4" => {
                        let val = ctx.ipv4.filter(|s| !s.trim().is_empty()).ok_or_else(|| {
                            TemplateError::MissingSlot {
                                slot: "ipv4".to_string(),
                            }
                        })?;
                        output.push_str(val);
                    }
                    "ipv6" => {
                        let val = ctx.ipv6.filter(|s| !s.trim().is_empty()).ok_or_else(|| {
                            TemplateError::MissingSlot {
                                slot: "ipv6".to_string(),
                            }
                        })?;
                        output.push_str(val);
                    }
                    "domain" => {
                        let val = ctx.domain.filter(|s| !s.trim().is_empty()).ok_or_else(|| {
                            TemplateError::MissingSlot {
                                slot: "domain".to_string(),
                            }
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
                        if let Some(val) = ctx
                            .args
                            .and_then(|a| a.get(other))
                            .and_then(|v| v.as_deref())
                            .filter(|s| !s.trim().is_empty())
                        {
                            output.push_str(val);
                        } else {
                            return Err(TemplateError::MissingSlot {
                                slot: other.to_string(),
                            });
                        }
                    }
                }
            }
            rest = &after_start[end_idx + 1..];
        } else {
            // Unclosed {{, push literal
            output.push_str("{{");
            rest = after_start;
        }
    }
    output.push_str(rest);

    let trimmed = output.trim();
    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        output = sanitize_json_commas(&output);
    }

    Ok(output)
}

/// Quote-aware JSON comma normalizer.
/// Automatically cleans leading, trailing, and consecutive invalid commas resulting
/// from conditional block omission in JSON templates:
/// - Leading commas after `[` or `{` (e.g. `[ , item ]` -> `[ item ]`)
/// - Trailing commas before `]` or `}` (e.g. `[ item , ]` -> `[ item ]`)
/// - Consecutive commas (e.g. `[ item1 , , item2 ]` -> `[ item1 , item2 ]`)
/// - Commas in empty containers (e.g. `[ , ]` -> `[]`, `{ , }` -> `{}`)
///
/// Commas and brackets inside string literals (e.g. `"hello, ] world"`) are strictly preserved.
pub fn sanitize_json_commas(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut in_string = false;
    let mut escape = false;
    let mut pending_comma = false;
    let mut pending_whitespace = String::new();
    let mut can_accept_comma = false;

    for c in input.chars() {
        if in_string {
            output.push(c);
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
                can_accept_comma = true;
            }
        } else {
            match c {
                '"' => {
                    if pending_comma {
                        output.push(',');
                        pending_comma = false;
                    }
                    output.push_str(&pending_whitespace);
                    pending_whitespace.clear();
                    output.push('"');
                    in_string = true;
                    can_accept_comma = false;
                }
                ' ' | '\t' | '\n' | '\r' => {
                    pending_whitespace.push(c);
                }
                ',' => {
                    if can_accept_comma {
                        pending_comma = true;
                        can_accept_comma = false;
                        pending_whitespace.clear();
                    } else {
                        // Duplicate comma or leading comma; discard it
                        pending_whitespace.clear();
                    }
                }
                ']' | '}' => {
                    // Trailing comma before closer is dropped!
                    pending_comma = false;
                    output.push_str(&pending_whitespace);
                    pending_whitespace.clear();
                    output.push(c);
                    can_accept_comma = true;
                }
                '[' | '{' => {
                    if pending_comma {
                        output.push(',');
                        pending_comma = false;
                    }
                    output.push_str(&pending_whitespace);
                    pending_whitespace.clear();
                    output.push(c);
                    can_accept_comma = false;
                }
                ':' => {
                    if pending_comma {
                        pending_comma = false;
                    }
                    output.push_str(&pending_whitespace);
                    pending_whitespace.clear();
                    output.push(':');
                    can_accept_comma = false;
                }
                other => {
                    if pending_comma {
                        output.push(',');
                        pending_comma = false;
                    }
                    output.push_str(&pending_whitespace);
                    pending_whitespace.clear();
                    output.push(other);
                    can_accept_comma = true;
                }
            }
        }
    }

    output.push_str(&pending_whitespace);
    output
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
        args.insert("password".to_string(), Some("my_secret_pass".to_string()));
        args.insert("zone_id".to_string(), Some("abc123xyz".to_string()));

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
        let args: HashMap<String, Option<String>> = HashMap::new();
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

    #[test]
    fn test_conditional_template_dual_stack() {
        let ctx = TemplateContext {
            ipv4: Some("1.2.3.4"),
            ipv6: Some("240e:3a1::1"),
            domain: Some("test.dynv6.net"),
            timestamp: None,
            args: None,
        };

        // dynv6 style
        let tmpl = "https://dynv6.com/api/update?hostname={{domain}}{{?ipv4:&ipv4={}}}{{?ipv6:&ipv6={}}}";
        let res = render_template(tmpl, &ctx).unwrap();
        assert_eq!(
            res,
            "https://dynv6.com/api/update?hostname=test.dynv6.net&ipv4=1.2.3.4&ipv6=240e:3a1::1"
        );

        // dynu style with else branch
        let dynu_tmpl = "https://api.dynu.com/nic/update?hostname={{domain}}{{?ipv4:&myip={}|&myip=no}}{{?ipv6:&myipv6={}|&myipv6=no}}";
        let res_dynu = render_template(dynu_tmpl, &ctx).unwrap();
        assert_eq!(
            res_dynu,
            "https://api.dynu.com/nic/update?hostname=test.dynv6.net&myip=1.2.3.4&myipv6=240e:3a1::1"
        );
    }

    #[test]
    fn test_conditional_template_ipv4_only() {
        let ctx = TemplateContext {
            ipv4: Some("1.2.3.4"),
            ipv6: None,
            domain: Some("test.dynv6.net"),
            timestamp: None,
            args: None,
        };

        // dynv6 style: IPv6 omitted completely
        let tmpl = "https://dynv6.com/api/update?hostname={{domain}}{{?ipv4:&ipv4={}}}{{?ipv6:&ipv6={}}}";
        let res = render_template(tmpl, &ctx).unwrap();
        assert_eq!(
            res,
            "https://dynv6.com/api/update?hostname=test.dynv6.net&ipv4=1.2.3.4"
        );

        // dynu style: IPv6 defaults to &myipv6=no
        let dynu_tmpl = "https://api.dynu.com/nic/update?hostname={{domain}}{{?ipv4:&myip={}|&myip=no}}{{?ipv6:&myipv6={}|&myipv6=no}}";
        let res_dynu = render_template(dynu_tmpl, &ctx).unwrap();
        assert_eq!(
            res_dynu,
            "https://api.dynu.com/nic/update?hostname=test.dynv6.net&myip=1.2.3.4&myipv6=no"
        );
    }

    #[test]
    fn test_conditional_template_ipv6_only() {
        let ctx = TemplateContext {
            ipv4: None,
            ipv6: Some("240e:3a1::1"),
            domain: Some("test.dynv6.net"),
            timestamp: None,
            args: None,
        };

        // dynv6 style: IPv4 omitted completely
        let tmpl = "https://dynv6.com/api/update?hostname={{domain}}{{?ipv4:&ipv4={}}}{{?ipv6:&ipv6={}}}";
        let res = render_template(tmpl, &ctx).unwrap();
        assert_eq!(
            res,
            "https://dynv6.com/api/update?hostname=test.dynv6.net&ipv6=240e:3a1::1"
        );

        // dynu style: IPv4 defaults to &myip=no
        let dynu_tmpl = "https://api.dynu.com/nic/update?hostname={{domain}}{{?ipv4:&myip={}|&myip=no}}{{?ipv6:&myipv6={}|&myipv6=no}}";
        let res_dynu = render_template(dynu_tmpl, &ctx).unwrap();
        assert_eq!(
            res_dynu,
            "https://api.dynu.com/nic/update?hostname=test.dynv6.net&myip=no&myipv6=240e:3a1::1"
        );
    }

    #[test]
    fn test_conditional_template_custom_args() {
        let mut args = HashMap::new();
        args.insert("token".to_string(), Some("secret_token".to_string()));
        args.insert("opt_flag".to_string(), None); // null / absent

        let ctx = TemplateContext {
            ipv4: Some("1.2.3.4"),
            ipv6: None,
            domain: Some("example.com"),
            timestamp: None,
            args: Some(&args),
        };

        let tmpl = "https://example.com/update?token={{token}}{{?opt_flag:&flag={opt_flag}|&flag=disabled}}";
        let res = render_template(tmpl, &ctx).unwrap();
        assert_eq!(
            res,
            "https://example.com/update?token=secret_token&flag=disabled"
        );
    }

    #[test]
    fn test_sanitize_json_commas() {
        // Leading comma in array
        assert_eq!(sanitize_json_commas("[ , 1, 2 ]"), "[ 1, 2 ]");
        // Trailing comma in array
        assert_eq!(sanitize_json_commas("[ 1, 2 , ]"), "[ 1, 2 ]");
        // Duplicate commas in array
        assert_eq!(sanitize_json_commas("[ 1, , 2 ]"), "[ 1, 2 ]");
        // Multiple consecutive commas
        assert_eq!(sanitize_json_commas("[ 1 , , , 2 ]"), "[ 1, 2 ]");
        // Trailing comma in object
        assert_eq!(sanitize_json_commas(r#"{"a": 1, }"#), r#"{"a": 1 }"#);
        // Leading comma in object
        assert_eq!(sanitize_json_commas(r#"{ , "a": 1}"#), r#"{ "a": 1}"#);
        // Empty container with commas
        assert_eq!(sanitize_json_commas("[,]"), "[]");
        assert_eq!(sanitize_json_commas("[ , ]"), "[ ]");
        assert_eq!(sanitize_json_commas("[ , , ]"), "[ ]");
        assert_eq!(sanitize_json_commas("{ , }"), "{ }");
        // String immunity: commas, brackets, braces inside quotes are preserved untouched
        let with_str = r#"{"text": "hello, ] }, world", "count": 2, }"#;
        assert_eq!(
            sanitize_json_commas(with_str),
            r#"{"text": "hello, ] }, world", "count": 2 }"#
        );
        // Escaped quotes inside strings
        let with_escapes = r#"{"text": "quote \" , ] test", }"#;
        assert_eq!(
            sanitize_json_commas(with_escapes),
            r#"{"text": "quote \" , ] test" }"#
        );
    }

    #[test]
    fn test_conditional_json_array_cloudflare_batch() {
        let tmpl = r#"{"patches":[{{?ipv4:{"id":"{{record_id_v4}}","content":"{}"}}},{{?ipv6:{"id":"{{record_id_v6}}","content":"{}"}}}]}"#;

        let mut args = HashMap::new();
        args.insert("record_id_v4".to_string(), Some("rec_v4_123".to_string()));
        args.insert("record_id_v6".to_string(), Some("rec_v6_456".to_string()));

        // 1. Dual-stack
        let ctx_dual = TemplateContext {
            ipv4: Some("1.2.3.4"),
            ipv6: Some("240e:3a1::1"),
            domain: Some("sub.example.com"),
            timestamp: None,
            args: Some(&args),
        };
        let res_dual = render_template(tmpl, &ctx_dual).unwrap();
        assert_eq!(
            res_dual,
            r#"{"patches":[{"id":"rec_v4_123","content":"1.2.3.4"},{"id":"rec_v6_456","content":"240e:3a1::1"}]}"#
        );

        // 2. IPv4 only (IPv6 block omitted, trailing comma cleaned)
        let ctx_v4 = TemplateContext {
            ipv4: Some("1.2.3.4"),
            ipv6: None,
            domain: Some("sub.example.com"),
            timestamp: None,
            args: Some(&args),
        };
        let res_v4 = render_template(tmpl, &ctx_v4).unwrap();
        assert_eq!(
            res_v4,
            r#"{"patches":[{"id":"rec_v4_123","content":"1.2.3.4"}]}"#
        );

        // 3. IPv6 only (IPv4 block omitted, leading comma cleaned)
        let ctx_v6 = TemplateContext {
            ipv4: None,
            ipv6: Some("240e:3a1::1"),
            domain: Some("sub.example.com"),
            timestamp: None,
            args: Some(&args),
        };
        let res_v6 = render_template(tmpl, &ctx_v6).unwrap();
        assert_eq!(
            res_v6,
            r#"{"patches":[{"id":"rec_v6_456","content":"240e:3a1::1"}]}"#
        );

        // 4. Neither (both omitted, empty array)
        let ctx_none = TemplateContext {
            ipv4: None,
            ipv6: None,
            domain: Some("sub.example.com"),
            timestamp: None,
            args: Some(&args),
        };
        let res_none = render_template(tmpl, &ctx_none).unwrap();
        assert_eq!(res_none, r#"{"patches":[]}"#);
    }
}

