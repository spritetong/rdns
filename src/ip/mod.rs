//! IP address detection engine.

mod interface;
mod remote;

pub use interface::InterfaceIpFetcher;
pub use remote::RemoteIpFetcher;

use crate::config::IpStrategyConfig;
use crate::error::IpFetchError;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

/// Unified IP resolver instance configured for a specific task.
pub struct TaskIpResolver {
    v4_strategy: Option<IpStrategyConfig>,
    v6_strategy: Option<IpStrategyConfig>,
    remote_fetcher: RemoteIpFetcher,
}

impl TaskIpResolver {
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

        Self {
            v4_strategy,
            v6_strategy,
            remote_fetcher,
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
                let fetcher = InterfaceIpFetcher::new(
                    strat.interface.clone(),
                    None,
                    None,
                    strat.allow_private,
                    false,
                );
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
                let fetcher = InterfaceIpFetcher::new(
                    None,
                    strat.interface.clone(),
                    strat.ipv6_prefix.clone(),
                    false,
                    strat.allow_private,
                );
                let ip = fetcher.fetch_ipv6()?;
                Ok(Some(ip))
            }
            _ => Err(IpFetchError::AllSourcesExhausted),
        }
    }
}
