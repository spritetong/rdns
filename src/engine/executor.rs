//! Request execution, templating, and dry-run previewing.

use crate::config::RequestConfig;
use crate::engine::template::{TemplateContext, render_template};
use crate::engine::verifier::ResponseVerifier;
use crate::error::RdnsError;
use reqwest::Method;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::str::FromStr;

pub struct RequestExecutor {
    client: reqwest::Client,
}

impl RequestExecutor {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub async fn execute(
        &self,
        task_name: &str,
        req_cfg: &RequestConfig,
        ctx: &TemplateContext<'_>,
        dry_run: bool,
    ) -> Result<(), RdnsError> {
        // 1. Render URL
        let rendered_url = render_template(&req_cfg.url, ctx)?;

        // 2. Render Headers
        let mut headers = HeaderMap::new();
        for (k, v) in &req_cfg.headers {
            let rendered_v = render_template(v, ctx)?;
            let h_name = HeaderName::from_str(k)
                .map_err(|e| RdnsError::Assertion(format!("Invalid header name '{}': {}", k, e)))?;
            let h_val = HeaderValue::from_str(&rendered_v).map_err(|e| {
                RdnsError::Assertion(format!("Invalid header value for '{}': {}", k, e))
            })?;
            headers.insert(h_name, h_val);
        }

        // 3. Render Body if present
        let rendered_body = if let Some(ref body_tmpl) = req_cfg.body {
            Some(render_template(body_tmpl, ctx)?)
        } else {
            None
        };

        // 4. Handle Dry-Run Mode
        if dry_run {
            let masked_url = mask_sensitive_params(&rendered_url);
            println!("\n========== [DRY-RUN PREVIEW: {}] ==========", task_name);
            println!("Method: {}", req_cfg.method.to_uppercase());
            println!("URL:    {}", masked_url);
            println!("Headers:");
            for (k, v) in &headers {
                let val_str = v.to_str().unwrap_or("<binary>");
                let masked_val = if k.as_str().eq_ignore_ascii_case("authorization") {
                    mask_secret(val_str)
                } else {
                    val_str.to_string()
                };
                println!("  {}: {}", k, masked_val);
            }
            if let Some(ref body) = rendered_body {
                println!("Body:\n{}", body.trim());
            }
            if let Some(ref reg) = req_cfg.success_regex {
                println!("Assertion Regex:    {}", reg);
            }
            if !req_cfg.success_contains.is_empty() {
                println!("Assertion Contains: {:?}", req_cfg.success_contains);
            }
            println!("================================================\n");
            return Ok(());
        }

        // 5. Execute Live Request
        let method = Method::from_str(&req_cfg.method.to_uppercase()).map_err(|e| {
            RdnsError::Assertion(format!("Invalid HTTP method '{}': {}", req_cfg.method, e))
        })?;

        let mut req = self.client.request(method, &rendered_url).headers(headers);

        if let Some(ref body) = rendered_body {
            req = req.body(body.clone());
        }

        let resp = req.send().await?;
        let status = resp.status();
        let resp_body = resp.text().await.unwrap_or_default();

        // 6. Verify Response
        let verifier =
            ResponseVerifier::new(req_cfg.success_regex.as_deref(), &req_cfg.success_contains)?;
        verifier.verify(status, &resp_body)?;

        tracing::info!(
            task = %task_name,
            status = %status,
            "HTTP Webhook request executed and verified successfully"
        );

        Ok(())
    }
}

fn mask_sensitive_params(url: &str) -> String {
    let sensitive_keys = ["password", "token", "secret", "key", "pass", "auth"];
    let mut masked = url.to_string();
    for key in sensitive_keys {
        let pattern = format!("{}=", key);
        let mut search_from = 0;
        while let Some(pos) = masked[search_from..].to_lowercase().find(&pattern) {
            let actual_pos = search_from + pos + pattern.len();
            let end_pos = masked[actual_pos..]
                .find('&')
                .map(|p| actual_pos + p)
                .unwrap_or(masked.len());
            let val = &masked[actual_pos..end_pos];
            let secret_mask = mask_secret(val);
            masked.replace_range(actual_pos..end_pos, &secret_mask);
            search_from = actual_pos + secret_mask.len();
        }
    }
    masked
}

fn mask_secret(val: &str) -> String {
    if val.len() <= 4 {
        "****".to_string()
    } else {
        format!("{}****", &val[..2])
    }
}
