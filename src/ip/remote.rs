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
                && let Ok(text) = resp.text().await
                && let Ok(ip) = text.trim().parse::<Ipv4Addr>()
            {
                return Ok(ip);
            }
            tracing::debug!(
                url = %url,
                "Failed to query IPv4 from remote source, trying next fallback"
            );
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
                && let Ok(text) = resp.text().await
                && let Ok(ip) = text.trim().parse::<Ipv6Addr>()
            {
                return Ok(ip);
            }
            tracing::debug!(
                url = %url,
                "Failed to query IPv6 from remote source, trying next fallback"
            );
        }
        Err(IpFetchError::AllSourcesExhausted)
    }
}
