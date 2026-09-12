//! Request execution, templating, and dry-run previewing.

use crate::config::RequestConfig;
use crate::engine::template::{TemplateContext, render_template};
use crate::engine::verifier::ResponseVerifier;
use crate::error::RdnsError;
use reqwest::Method;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;

pub const MAX_RESP_BODY_SIZE: usize = 1024 * 1024; // 1 MiB

type ClientCacheKey = (Option<String>, bool, Option<String>);

pub struct RequestExecutor {
    default_client: reqwest::Client,
    default_proxy: Option<String>,
    default_cacerts: Option<String>,
    timeout: Duration,
    custom_clients: parking_lot::RwLock<HashMap<ClientCacheKey, reqwest::Client>>,
}

impl RequestExecutor {
    pub fn new(
        client: reqwest::Client,
        default_proxy: Option<String>,
        default_cacerts: Option<String>,
        timeout: Duration,
    ) -> Self {
        Self {
            default_client: client,
            default_proxy,
            default_cacerts,
            timeout,
            custom_clients: parking_lot::RwLock::new(HashMap::new()),
        }
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.default_client
    }

    fn get_or_create_custom_client(
        &self,
        task_name: &str,
        proxy: Option<&str>,
        tls_insecure: bool,
        cacerts: Option<&str>,
    ) -> Result<reqwest::Client, RdnsError> {
        let key = (
            proxy.map(str::to_string),
            tls_insecure,
            cacerts.map(str::to_string),
        );
        {
            let read_guard = self.custom_clients.read();
            if let Some(client) = read_guard.get(&key) {
                return Ok(client.clone());
            }
        }

        let mut write_guard = self.custom_clients.write();
        if let Some(client) = write_guard.get(&key) {
            return Ok(client.clone());
        }

        if tls_insecure {
            tracing::warn!(
                "[{}] TLS certificate verification is disabled (tls_insecure: true)",
                task_name
            );
        }

        let client = crate::engine::build_http_client(
            self.timeout,
            proxy,
            tls_insecure,
            cacerts,
        )?;

        write_guard.insert(key, client.clone());
        Ok(client)
    }

    pub async fn execute(
        &self,
        task_name: &str,
        req_cfg: &RequestConfig,
        ctx: &TemplateContext<'_>,
        dry_run: bool,
    ) -> Result<(), RdnsError> {
        // 1. Render URL
        let rendered_url = render_template(req_cfg.url(), ctx)?;

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

        let effective_proxy = req_cfg.proxy.as_deref().or(self.default_proxy.as_deref());
        let effective_cacerts = req_cfg
            .cacerts
            .as_deref()
            .or(self.default_cacerts.as_deref());

        // 4. Handle Dry-Run Mode
        if dry_run {
            let masked_url = mask_sensitive_params(&rendered_url);
            println!("\n========== [DRY-RUN PREVIEW: {}] ==========", task_name);
            println!("Method: {}", req_cfg.method().to_uppercase());
            println!("URL:    {}", masked_url);
            println!("Headers:");
            for (k, v) in &headers {
                let val_str = v.to_str().unwrap_or("<binary>");
                let masked_val = if is_sensitive_header(k.as_str()) {
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
            if !req_cfg.success_contains().is_empty() {
                println!("Assertion Contains: {:?}", req_cfg.success_contains());
            }
            if req_cfg.tls_insecure() {
                println!("TLS Insecure:       true (CERTIFICATE VALIDATION DISABLED)");
            }
            if let Some(c) = effective_cacerts
                && !c.trim().is_empty()
            {
                println!("CA Certs:           {}", c);
            }
            if let Some(p) = effective_proxy
                && !p.trim().is_empty()
            {
                println!("Proxy:              {}", p);
            }
            println!("================================================\n");
            return Ok(());
        }

        // 5. Select appropriate HTTP client (cached by proxy/tls_insecure/cacerts config)
        let is_default_proxy = effective_proxy == self.default_proxy.as_deref();
        let is_default_cacerts = effective_cacerts == self.default_cacerts.as_deref();
        let client = if !req_cfg.tls_insecure() && is_default_proxy && is_default_cacerts {
            self.default_client.clone()
        } else {
            self.get_or_create_custom_client(
                task_name,
                effective_proxy,
                req_cfg.tls_insecure(),
                effective_cacerts,
            )?
        };

        // 6. Execute Live Request
        let method = Method::from_str(&req_cfg.method().to_uppercase()).map_err(|e| {
            RdnsError::Assertion(format!("Invalid HTTP method '{}': {}", req_cfg.method(), e))
        })?;

        let mut req = client.request(method, &rendered_url).headers(headers);

        if let Some(ref body) = rendered_body {
            req = req.body(body.clone());
        }

        let resp = req.send().await?;
        let status = resp.status();
        let resp_body = read_bounded_text(resp, MAX_RESP_BODY_SIZE).await?;

        // 7. Verify Response
        let verifier =
            ResponseVerifier::new(req_cfg.success_regex.as_deref(), req_cfg.success_contains())?;
        verifier.verify(status, &resp_body)?;

        tracing::debug!(
            "[{}] DDNS provider response verified successfully (HTTP {})",
            task_name,
            status
        );

        Ok(())
    }
}

pub async fn read_bounded_text(
    mut resp: reqwest::Response,
    max_bytes: usize,
) -> Result<String, RdnsError> {
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(RdnsError::Http)? {
        if buf.len().saturating_add(chunk.len()) > max_bytes {
            return Err(RdnsError::Assertion(format!(
                "Response body size exceeded maximum limit of {} bytes",
                max_bytes
            )));
        }
        buf.extend_from_slice(&chunk);
    }
    String::from_utf8(buf)
        .map_err(|e| RdnsError::Assertion(format!("Response body is not valid UTF-8: {}", e)))
}

fn is_sensitive_header(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "authorization"
        || n.contains("token")
        || n.contains("secret")
        || n.contains("password")
        || n.contains("key")
        || n.contains("auth")
}

fn mask_sensitive_params(url: &str) -> String {
    let sensitive_keys = ["password", "token", "secret", "key", "pass", "auth"];
    let mut masked = url.to_string();
    for key in sensitive_keys {
        let pattern = format!("{}=", key);
        let mut search_from = 0;
        while let Some(pos) = find_key_ci(&masked[search_from..], &pattern) {
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

/// Find an ASCII-case-insensitive key in the original byte coordinate space.
/// Does not mutate the haystack or change UTF-8 character byte lengths.
fn find_key_ci(haystack: &str, key: &str) -> Option<usize> {
    let (hb, kb) = (haystack.as_bytes(), key.as_bytes());
    if kb.is_empty() || hb.len() < kb.len() {
        return None;
    }
    (0..=hb.len() - kb.len()).find(|&i| {
        kb.iter()
            .zip(&hb[i..])
            .all(|(k, h)| k.eq_ignore_ascii_case(h))
    })
}

fn mask_secret(val: &str) -> String {
    let char_count = val.chars().count();
    if char_count <= 4 {
        "****".to_string()
    } else {
        let prefix_bytes = val
            .char_indices()
            .nth(2)
            .map(|(idx, _)| idx)
            .unwrap_or(val.len());
        format!("{}****", &val[..prefix_bytes])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mask_secret_ascii_and_unicode() {
        // <= 4 chars returns "****"
        assert_eq!(mask_secret(""), "****");
        assert_eq!(mask_secret("abc"), "****");
        assert_eq!(mask_secret("abcd"), "****");
        assert_eq!(mask_secret("密码"), "****");
        assert_eq!(mask_secret("🔑🦀👍"), "****");

        // > 4 chars preserves first 2 Unicode characters, not bytes
        assert_eq!(mask_secret("abcdef"), "ab****");
        assert_eq!(mask_secret("密码保护123"), "密码****");
        assert_eq!(mask_secret("€12345"), "€1****");
        assert_eq!(mask_secret("🔑🦀rocket"), "🔑🦀****");
    }

    #[test]
    fn test_mask_sensitive_params_unicode_and_case() {
        let url =
            "https://example.com/api?user=admin&PASSWORD=€12345&TOKEN=中文密码999&other=normal";
        let masked = mask_sensitive_params(url);
        assert!(masked.contains("PASSWORD=€1****"));
        assert!(masked.contains("TOKEN=中文****"));
        assert!(masked.contains("other=normal"));

        // Special Unicode casing test: ensure non-ASCII characters in URL don't cause index mismatch
        let url2 = "https://example.com/api?domain=münchen.de&key=secret123&foo=bar";
        let masked2 = mask_sensitive_params(url2);
        assert_eq!(
            masked2,
            "https://example.com/api?domain=münchen.de&key=se****&foo=bar"
        );
    }

    #[test]
    fn test_is_sensitive_header() {
        assert!(is_sensitive_header("Authorization"));
        assert!(is_sensitive_header("X-Api-Key"));
        assert!(is_sensitive_header("X-Auth-Token"));
        assert!(is_sensitive_header("Client-Secret"));
        assert!(!is_sensitive_header("Content-Type"));
        assert!(!is_sensitive_header("User-Agent"));
    }

    #[tokio::test]
    async fn test_read_bounded_text_overflow() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let response = "HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\n01234567890123456789";
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });

        let client = reqwest::Client::new();
        let resp = client.get(format!("http://{}", addr)).send().await.unwrap();
        // Limit to 10 bytes: response body has 20 bytes -> must return RdnsError::Assertion
        let res = read_bounded_text(resp, 10).await;
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("exceeded maximum limit")
        );
    }

    #[tokio::test]
    async fn test_read_bounded_text_success() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let response =
                    "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello";
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });

        let client = reqwest::Client::new();
        let resp = client.get(format!("http://{}", addr)).send().await.unwrap();
        let res = read_bounded_text(resp, 100).await.unwrap();
        assert_eq!(res, "hello");
    }

    #[tokio::test]
    async fn test_request_executor_dry_run_with_global_proxy_fallback() {
        let client = reqwest::Client::new();
        let executor = RequestExecutor::new(
            client,
            Some("http://127.0.0.1:7890".to_string()),
            None,
            Duration::from_secs(10),
        );
        let req_cfg = RequestConfig {
            url: Some("https://example.com/update".to_string()),
            tls_insecure: Some(true),
            ..Default::default()
        };
        let ctx = TemplateContext {
            ipv4: Some("1.2.3.4"),
            ipv6: None,
            domain: Some("test.domain"),
            timestamp: Some(1234567890),
            args: None,
        };

        // dry-run executes cleanly and displays the effective proxy
        let res = executor.execute("test-task", &req_cfg, &ctx, true).await;
        assert!(res.is_ok());
    }

    #[test]
    fn test_request_executor_custom_client_caches_effective_proxy() {
        let client = reqwest::Client::new();
        let executor = RequestExecutor::new(
            client,
            Some("http://127.0.0.1:7890".to_string()),
            None,
            Duration::from_secs(10),
        );

        let _c1 = executor
            .get_or_create_custom_client("t1", Some("http://127.0.0.1:7890"), true, None)
            .expect("client creation succeeds");
        let _c2 = executor
            .get_or_create_custom_client("t2", Some("http://127.0.0.1:7890"), true, None)
            .expect("client reuse succeeds");

        assert_eq!(executor.custom_clients.read().len(), 1);
        let key = (Some("http://127.0.0.1:7890".to_string()), true, None);
        assert!(executor.custom_clients.read().contains_key(&key));
    }
}
