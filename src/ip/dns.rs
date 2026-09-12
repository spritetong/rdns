// Copyright (c) 2026 Sprite Tong <spritetong@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later
// rdns is licensed under the GNU GPL v3.0 or later.

//! DNS resolver for verifying cloud domain records using hickory-resolver.

use crate::error::IpFetchError;
use hickory_resolver::Resolver;
use hickory_resolver::config::{NameServerConfig, ResolverConfig, ResolverOpts};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

/// DNS query client powered by `hickory-resolver`.
#[derive(Clone, Default)]
pub struct DnsResolver;

impl DnsResolver {
    pub fn new() -> Self {
        Self
    }

    /// Resolve A and AAAA records for a domain.
    /// If `dns_server` is None, uses the OS default configuration.
    /// If `dns_server` is Some, uses hickory-resolver to query the specified nameserver over UDP/TCP.
    pub async fn resolve(
        &self,
        domain: &str,
        dns_server: Option<&str>,
        timeout: Duration,
    ) -> Result<(Option<Ipv4Addr>, Option<Ipv6Addr>), IpFetchError> {
        let domain = domain.trim().trim_end_matches('.');
        if domain.is_empty() {
            return Ok((None, None));
        }

        let mut opts = ResolverOpts::default();
        opts.timeout = timeout;
        opts.attempts = 2;

        let resolver = match dns_server {
            Some(srv) => {
                let socket_addr =
                    parse_dns_server_addr(srv)
                        .await
                        .map_err(|e| IpFetchError::DnsLookup {
                            domain: domain.to_string(),
                            message: format!("Invalid DNS server '{}': {}", srv, e),
                        })?;

                let mut name_server = NameServerConfig::udp_and_tcp(socket_addr.ip());
                for conn in &mut name_server.connections {
                    conn.port = socket_addr.port();
                }
                let config = ResolverConfig::from_name_servers(vec![name_server]);
                let builder =
                    Resolver::builder_with_config(config, TokioRuntimeProvider::default());
                builder
                    .with_options(opts)
                    .build()
                    .map_err(|e| IpFetchError::DnsLookup {
                        domain: domain.to_string(),
                        message: format!("Failed to create DNS resolver: {}", e),
                    })?
            }
            None => {
                let builder = Resolver::builder_tokio().unwrap_or_else(|_| {
                    Resolver::builder_with_config(
                        ResolverConfig::default(),
                        TokioRuntimeProvider::default(),
                    )
                });
                builder
                    .with_options(opts)
                    .build()
                    .map_err(|e| IpFetchError::DnsLookup {
                        domain: domain.to_string(),
                        message: format!("Failed to create DNS resolver: {}", e),
                    })?
            }
        };

        match resolver.lookup_ip(domain).await {
            Ok(lookup) => {
                let mut v4 = None;
                let mut v6 = None;
                for ip in lookup.iter() {
                    match ip {
                        IpAddr::V4(addr) if v4.is_none() => v4 = Some(addr),
                        IpAddr::V6(addr) if v6.is_none() => v6 = Some(addr),
                        _ => {}
                    }
                }
                Ok((v4, v6))
            }
            Err(e) => {
                let err_str = e.to_string();
                // If no records or NXDOMAIN, return None instead of failing the task
                if err_str.contains("no record found")
                    || err_str.contains("NXDomain")
                    || err_str.contains("NoRecordsFound")
                {
                    Ok((None, None))
                } else {
                    Err(IpFetchError::DnsLookup {
                        domain: domain.to_string(),
                        message: err_str,
                    })
                }
            }
        }
    }
}

/// Parse a DNS server string into a SocketAddr (defaulting port to 53).
async fn parse_dns_server_addr(srv: &str) -> Result<SocketAddr, String> {
    let srv = srv.trim();
    if srv.is_empty() {
        return Err("Empty DNS server".to_string());
    }

    if let Ok(addr) = srv.parse::<SocketAddr>() {
        return Ok(addr);
    }

    if let Ok(ip) = srv.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, 53));
    }

    let target = if srv.contains(':') {
        srv.to_string()
    } else {
        format!("{}:53", srv)
    };

    let mut addrs = tokio::net::lookup_host(&target)
        .await
        .map_err(|e| format!("Could not resolve DNS server '{}': {}", srv, e))?;

    addrs
        .next()
        .ok_or_else(|| format!("No IP found for DNS server '{}'", srv))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_empty_domain() {
        let resolver = DnsResolver::new();
        let res = resolver
            .resolve("", None, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(res, (None, None));
    }

    #[tokio::test]
    async fn test_parse_dns_server_addr() {
        let addr = parse_dns_server_addr("8.8.8.8").await.unwrap();
        assert_eq!(addr, "8.8.8.8:53".parse().unwrap());

        let addr = parse_dns_server_addr("1.1.1.1:5353").await.unwrap();
        assert_eq!(addr, "1.1.1.1:5353".parse().unwrap());
    }
}
