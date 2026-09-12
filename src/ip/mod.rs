// Copyright (c) 2026 Sprite Tong <spritetong@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later
// rdns is licensed under the GNU GPL v3.0 or later.

//! IP address detection engine.

mod dns;
mod interface;
mod remote;

pub use dns::DnsResolver;
pub use interface::InterfaceIpFetcher;
pub use remote::RemoteIpFetcher;

use crate::config::IpStrategyConfig;
use crate::error::IpFetchError;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

/// Unified IP resolver instance configured for a specific network interface profile.
pub struct InterfaceIpResolver {
    v4_strategy: Option<IpStrategyConfig>,
    v6_strategy: Option<IpStrategyConfig>,
    remote_fetcher: RemoteIpFetcher,
    v4_interface_fetcher: Option<InterfaceIpFetcher>,
    v6_interface_fetcher: Option<InterfaceIpFetcher>,
}

impl InterfaceIpResolver {
    pub fn new(
        v4_strategy: Option<IpStrategyConfig>,
        v6_strategy: Option<IpStrategyConfig>,
        timeout: Duration,
    ) -> Self {
        let v4_urls = v4_strategy
            .as_ref()
            .filter(|s| s.enabled && s.source == "remote")
            .map(|s| s.urls.clone())
            .unwrap_or_default();

        let v6_urls = v6_strategy
            .as_ref()
            .filter(|s| s.enabled && s.source == "remote")
            .map(|s| s.urls.clone())
            .unwrap_or_default();

        let remote_fetcher = RemoteIpFetcher::new(v4_urls, v6_urls, timeout);

        let v4_interface_fetcher = if let Some(ref strat) = v4_strategy {
            if strat.enabled && strat.source == "interface" {
                Some(InterfaceIpFetcher::new(
                    strat.interface.clone(),
                    None,
                    None,
                    None,
                    false,
                    strat.allow_private,
                    false,
                ))
            } else {
                None
            }
        } else {
            None
        };

        let v6_interface_fetcher = if let Some(ref strat) = v6_strategy {
            if strat.enabled && strat.source == "interface" {
                Some(InterfaceIpFetcher::new(
                    None,
                    strat.interface.clone(),
                    strat.ipv6_prefix.clone(),
                    strat.ipv6_regex.clone(),
                    strat.prefer_slaac,
                    false,
                    strat.allow_private,
                ))
            } else {
                None
            }
        } else {
            None
        };

        Self {
            v4_strategy,
            v6_strategy,
            remote_fetcher,
            v4_interface_fetcher,
            v6_interface_fetcher,
        }
    }

    /// Resolve both IPv4 and IPv6 concurrently, supporting graceful single-stack degradation.
    pub async fn resolve(&self) -> Result<(Option<Ipv4Addr>, Option<Ipv6Addr>), IpFetchError> {
        let (v4_res, v6_res) = tokio::join!(self.resolve_ipv4(), self.resolve_ipv6());

        let v4_enabled = self
            .v4_strategy
            .as_ref()
            .map(|s| s.enabled)
            .unwrap_or(false);
        let v6_enabled = self
            .v6_strategy
            .as_ref()
            .map(|s| s.enabled)
            .unwrap_or(false);

        match (v4_res, v6_res) {
            (Ok(v4), Ok(v6)) => Ok((v4, v6)),
            (Ok(v4), Err(e)) => {
                if v4_enabled && v4.is_some() {
                    tracing::warn!(
                        "IPv6 resolution failed ({}), continuing with IPv4 single-stack",
                        e
                    );
                    Ok((v4, None))
                } else {
                    Err(e)
                }
            }
            (Err(e), Ok(v6)) => {
                if v6_enabled && v6.is_some() {
                    tracing::warn!(
                        "IPv4 resolution failed ({}), continuing with IPv6 single-stack",
                        e
                    );
                    Ok((None, v6))
                } else {
                    Err(e)
                }
            }
            (Err(e4), Err(e6)) => {
                tracing::error!(
                    "Both IPv4 and IPv6 resolution failed (IPv4: {}, IPv6: {})",
                    e4,
                    e6
                );
                Err(e4)
            }
        }
    }

    pub async fn resolve_ipv4(&self) -> Result<Option<Ipv4Addr>, IpFetchError> {
        let strat = match self.v4_strategy {
            Some(ref s) if s.enabled => s,
            _ => return Ok(None),
        };

        match strat.source.as_str() {
            "remote" => {
                let ip = self.remote_fetcher.fetch_ipv4().await?;
                Ok(Some(ip))
            }
            "interface" => {
                let fetcher = self.v4_interface_fetcher.as_ref().ok_or_else(|| {
                    IpFetchError::InterfaceNotFound {
                        name: strat.interface.clone().unwrap_or_default(),
                    }
                })?;
                let ip = fetcher.fetch_ipv4()?;
                Ok(Some(ip))
            }
            _ => Err(IpFetchError::AllSourcesExhausted),
        }
    }

    pub async fn resolve_ipv6(&self) -> Result<Option<Ipv6Addr>, IpFetchError> {
        let strat = match self.v6_strategy {
            Some(ref s) if s.enabled => s,
            _ => return Ok(None),
        };

        match strat.source.as_str() {
            "remote" => {
                let ip = self.remote_fetcher.fetch_ipv6().await?;
                Ok(Some(ip))
            }
            "interface" => {
                let fetcher = self.v6_interface_fetcher.as_ref().ok_or_else(|| {
                    IpFetchError::InterfaceNotFound {
                        name: strat.interface.clone().unwrap_or_default(),
                    }
                })?;
                let ip = fetcher.fetch_ipv6()?;
                Ok(Some(ip))
            }
            _ => Err(IpFetchError::AllSourcesExhausted),
        }
    }
}
