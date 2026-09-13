// Copyright (c) 2026 Sprite Tong <spritetong@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later
// rdns is licensed under the GNU GPL v3.0 or later.

//! Remote HTTP-based IP discovery with protocol stack binding and fallback.

use crate::error::IpFetchError;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

pub struct RemoteIpFetcher {
    v4_urls: Vec<String>,
    v6_urls: Vec<String>,
    v4_client: reqwest::Client,
    v6_client: reqwest::Client,
    fallback_client: reqwest::Client,
}

impl RemoteIpFetcher {
    pub fn new(v4_urls: Vec<String>, v6_urls: Vec<String>, timeout: Duration) -> Self {
        let v4_client = reqwest::Client::builder()
            .timeout(timeout)
            .local_address(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let v6_client = reqwest::Client::builder()
            .timeout(timeout)
            .local_address(IpAddr::V6(Ipv6Addr::UNSPECIFIED))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let fallback_client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            v4_urls,
            v6_urls,
            v4_client,
            v6_client,
            fallback_client,
        }
    }

    pub async fn fetch_ipv4(&self) -> Result<Ipv4Addr, IpFetchError> {
        for url in &self.v4_urls {
            // First attempt with v4-bound client (0.0.0.0)
            let resp = match self.v4_client.get(url).send().await {
                Ok(r) => Ok(r),
                Err(_) => self.fallback_client.get(url).send().await,
            };

            if let Ok(resp) = resp
                && resp.status().is_success()
                && let Some(text) = read_ip_text(resp).await
                && let Some(IpAddr::V4(ip)) = parse_ip_from_response(&text)
            {
                return Ok(ip);
            }
            tracing::debug!("Failed to query IPv4 from '{}', trying next fallback", url);
        }
        Err(IpFetchError::AllSourcesExhausted)
    }

    pub async fn fetch_ipv6(&self) -> Result<Ipv6Addr, IpFetchError> {
        for url in &self.v6_urls {
            // First attempt with v6-bound client (::)
            let resp = match self.v6_client.get(url).send().await {
                Ok(r) => Ok(r),
                Err(_) => self.fallback_client.get(url).send().await,
            };

            if let Ok(resp) = resp
                && resp.status().is_success()
                && let Some(text) = read_ip_text(resp).await
                && let Some(IpAddr::V6(ip)) = parse_ip_from_response(&text)
            {
                return Ok(ip);
            }
            tracing::debug!("Failed to query IPv6 from '{}', trying next fallback", url);
        }
        Err(IpFetchError::AllSourcesExhausted)
    }
}

const MAX_IP_RESP_BYTES: usize = 4096;

async fn read_ip_text(mut resp: reqwest::Response) -> Option<String> {
    let mut buf = Vec::new();
    while let Ok(Some(chunk)) = resp.chunk().await {
        if buf.len().saturating_add(chunk.len()) > MAX_IP_RESP_BYTES {
            return None;
        }
        buf.extend_from_slice(&chunk);
    }
    String::from_utf8(buf).ok()
}

/// Parse an IP address from arbitrary remote response text.
/// Automatically adapts to:
/// - Plain text IP (e.g. `1.2.3.4` or `240e:3a1::1`)
/// - Quoted IP string (e.g. `"1.2.3.4"`)
/// - JSON format (e.g. `{"ip": "1.2.3.4"}`, `{"query": "..."}`, `{"origin": "..."}`)
/// - Embedded IPv4 pattern fallback
pub fn parse_ip_from_response(text: &str) -> Option<IpAddr> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 1. Direct parse (plain text IP)
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        return Some(ip);
    }

    // 2. Strip quotes if any (e.g. "1.2.3.4" or '1.2.3.4')
    let unquoted = trimmed.trim_matches(|c| c == '"' || c == '\'').trim();
    if let Ok(ip) = unquoted.parse::<IpAddr>() {
        return Some(ip);
    }

    // 3. JSON format: {"ip": "...", "query": "...", "origin": "...", "client_ip": "...", "address": "...", "myip": "..."}
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed)
        && let Some(obj) = val.as_object()
    {
        for key in ["ip", "query", "client_ip", "origin", "address", "myip"] {
            if let Some(s) = obj.get(key).and_then(|v| v.as_str())
                && let Ok(ip) = s.trim().parse::<IpAddr>()
            {
                return Some(ip);
            }
        }
    }

    // 4. Regex fallback: search for IPv4 pattern in case of wrapped text
    if let Ok(re) = regex::Regex::new(r"\b(?:[0-9]{1,3}\.){3}[0-9]{1,3}\b")
        && let Some(m) = re.find(trimmed)
        && let Ok(ip) = m.as_str().parse::<Ipv4Addr>()
    {
        return Some(IpAddr::V4(ip));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ip_from_response_plain() {
        assert_eq!(
            parse_ip_from_response("1.2.3.4"),
            Some(IpAddr::V4("1.2.3.4".parse().unwrap()))
        );
        assert_eq!(
            parse_ip_from_response("  2001:db8::1 \n"),
            Some(IpAddr::V6("2001:db8::1".parse().unwrap()))
        );
    }

    #[test]
    fn test_parse_ip_from_response_quoted() {
        assert_eq!(
            parse_ip_from_response("\"1.2.3.4\""),
            Some(IpAddr::V4("1.2.3.4".parse().unwrap()))
        );
        assert_eq!(
            parse_ip_from_response("'240e:3a1::1'"),
            Some(IpAddr::V6("240e:3a1::1".parse().unwrap()))
        );
    }

    #[test]
    fn test_parse_ip_from_response_json() {
        // ipify style
        assert_eq!(
            parse_ip_from_response(r#"{"ip": "203.0.113.195"}"#),
            Some(IpAddr::V4("203.0.113.195".parse().unwrap()))
        );
        // ip-api style
        assert_eq!(
            parse_ip_from_response(r#"{"query": "240e:3a1::1", "status": "success"}"#),
            Some(IpAddr::V6("240e:3a1::1".parse().unwrap()))
        );
        // httpbin style
        assert_eq!(
            parse_ip_from_response(r#"{"origin": "198.51.100.10"}"#),
            Some(IpAddr::V4("198.51.100.10".parse().unwrap()))
        );
    }

    #[test]
    fn test_parse_ip_from_response_invalid() {
        assert_eq!(parse_ip_from_response(""), None);
        assert_eq!(parse_ip_from_response("   "), None);
        assert_eq!(parse_ip_from_response("Internal Server Error"), None);
        assert_eq!(parse_ip_from_response(r#"{"error": "not found"}"#), None);
    }
}
