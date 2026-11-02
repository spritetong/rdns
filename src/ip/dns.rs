// Copyright (c) 2026 Sprite Tong <spritetong@gmail.com>
// SPDX-License-Identifier: MPL-2.0
// rdns is licensed under the Mozilla Public License Version 2.0.

//! DNS resolver for verifying cloud domain records using hickory-resolver.
//!
//! Supports wstunnel-style DNS resolver configuration:
//! - `dns://1.1.1.1` (UDP/TCP, default port 53)
//! - `dns+https://1.1.1.1?sni=cloudflare-dns.com` (DNS-over-HTTPS, default port 443)
//! - `dns+tls://8.8.8.8?sni=dns.google` (DNS-over-TLS, default port 853)
//! - `system://0.0.0.0` (Operating system native libc resolver)
//! - `8.8.8.8` (Bare IP or host, treated as dns:// for backward compatibility)

use crate::error::IpFetchError;
use hickory_resolver::Resolver;
use hickory_resolver::config::{NameServerConfig, ResolverConfig, ResolverOpts};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

/// DNS query client powered by `hickory-resolver`.
#[derive(Clone, Default)]
pub struct DnsResolver {
    resolver_cache: Arc<Mutex<HashMap<String, Arc<hickory_resolver::TokioResolver>>>>,
}

impl DnsResolver {
    pub fn new() -> Self {
        Self {
            resolver_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Resolve A and AAAA records for a domain.
    ///
    /// If `resolvers` is `None` or empty, or contains `system://...`, uses the OS default configuration.
    /// Otherwise, queries the specified upstream resolvers (supporting UDP/TCP, DoH, DoT).
    pub async fn resolve(
        &self,
        domain: &str,
        resolvers: Option<&[String]>,
        timeout: Duration,
    ) -> Result<(Option<Ipv4Addr>, Option<Ipv6Addr>), IpFetchError> {
        let domain = domain.trim().trim_end_matches('.');
        if domain.is_empty() {
            return Ok((None, None));
        }

        let resolver = self.get_or_create_resolver(resolvers, timeout).await?;

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

    async fn get_or_create_resolver(
        &self,
        resolvers: Option<&[String]>,
        timeout: Duration,
    ) -> Result<Arc<hickory_resolver::TokioResolver>, IpFetchError> {
        let cache_key = match resolvers {
            Some(list) if !list.is_empty() => format!("{}:{}", list.join(";"), timeout.as_millis()),
            _ => format!("system:{}", timeout.as_millis()),
        };

        {
            let cache = self.resolver_cache.lock();
            if let Some(r) = cache.get(&cache_key) {
                return Ok(Arc::clone(r));
            }
        }

        let mut opts = ResolverOpts::default();
        opts.timeout = timeout;
        opts.attempts = 2;

        let is_system = match resolvers {
            Some(list) if !list.is_empty() => list.iter().any(|r| is_system_resolver(r)),
            _ => true,
        };

        let resolver = if is_system {
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
                    domain: String::new(),
                    message: format!("Failed to create system DNS resolver: {}", e),
                })?
        } else {
            let list = resolvers.unwrap();
            let mut name_servers = Vec::new();
            for r in list {
                let ns = parse_resolver_item(r)
                    .await
                    .map_err(|e| IpFetchError::DnsLookup {
                        domain: String::new(),
                        message: format!("Invalid DNS resolver '{}': {}", r, e),
                    })?;
                name_servers.push(ns);
            }
            let config = ResolverConfig::from_name_servers(name_servers);
            let builder = Resolver::builder_with_config(config, TokioRuntimeProvider::default());
            builder
                .with_options(opts)
                .build()
                .map_err(|e| IpFetchError::DnsLookup {
                    domain: String::new(),
                    message: format!("Failed to create custom DNS resolver: {}", e),
                })?
        };

        let resolver = Arc::new(resolver);
        let mut cache = self.resolver_cache.lock();
        cache.insert(cache_key, Arc::clone(&resolver));
        Ok(resolver)
    }
}

/// Checks if a resolver string denotes the host OS system / libc resolver.
pub fn is_system_resolver(srv: &str) -> bool {
    let s = srv.trim();
    s.starts_with("system://") || s.eq_ignore_ascii_case("system")
}

/// Parse a resolver string into a Hickory `NameServerConfig`.
pub async fn parse_resolver_item(r: &str) -> Result<NameServerConfig, String> {
    let r = r.trim();
    if r.is_empty() {
        return Err("Empty DNS resolver".to_string());
    }

    if is_system_resolver(r) {
        return Err("system resolver must be handled at the resolver builder level".to_string());
    }

    // If bare IP or host[:port] without scheme, prefix with dns:// for backward compatibility
    let url_str = if !r.contains("://") {
        if let Ok(ip) = r.parse::<IpAddr>() {
            match ip {
                IpAddr::V4(_) => format!("dns://{}", ip),
                IpAddr::V6(_) => format!("dns://[{}]", ip),
            }
        } else if let Ok(sa) = r.parse::<SocketAddr>() {
            match sa.ip() {
                IpAddr::V4(_) => format!("dns://{}", sa),
                IpAddr::V6(v6) => format!("dns://[{}]:{}", v6, sa.port()),
            }
        } else {
            format!("dns://{}", r)
        }
    } else {
        r.to_string()
    };

    let parsed_url =
        url::Url::parse(&url_str).map_err(|e| format!("Invalid resolver URL '{}': {}", r, e))?;

    let host = parsed_url
        .host()
        .ok_or_else(|| format!("Missing host in DNS resolver URL '{}'", r))?;

    let (ip, default_sni) = match host {
        url::Host::Ipv4(v4) => (IpAddr::V4(v4), v4.to_string()),
        url::Host::Ipv6(v6) => (IpAddr::V6(v6), v6.to_string()),
        url::Host::Domain(domain) => {
            let dummy_port = parsed_url.port().unwrap_or(53);
            let mut addrs = tokio::net::lookup_host((domain, dummy_port))
                .await
                .map_err(|e| format!("Failed to resolve DNS resolver host '{}': {}", domain, e))?;
            let ip = addrs
                .next()
                .map(|sa| sa.ip())
                .ok_or_else(|| format!("No IP address found for DNS resolver host '{}'", domain))?;
            (ip, domain.to_string())
        }
    };

    match parsed_url.scheme() {
        "dns" => {
            let port = parsed_url.port().unwrap_or(53);
            let mut ns = NameServerConfig::udp_and_tcp(ip);
            for conn in &mut ns.connections {
                conn.port = port;
            }
            Ok(ns)
        }
        "dns+https" | "https" => {
            let port = parsed_url.port().unwrap_or(443);
            let sni = parsed_url
                .query_pairs()
                .find(|(k, _)| k == "sni")
                .map(|(_, v)| v.to_string())
                .unwrap_or(default_sni);

            let path = {
                let p = parsed_url.path();
                if p.is_empty() || p == "/" {
                    None
                } else {
                    Some(Arc::from(p))
                }
            };

            let mut ns = NameServerConfig::https(ip, Arc::from(sni.as_str()), path);
            for conn in &mut ns.connections {
                conn.port = port;
            }
            Ok(ns)
        }
        "dns+tls" | "tls" => {
            let port = parsed_url.port().unwrap_or(853);
            let sni = parsed_url
                .query_pairs()
                .find(|(k, _)| k == "sni")
                .map(|(_, v)| v.to_string())
                .unwrap_or(default_sni);

            let mut ns = NameServerConfig::tls(ip, Arc::from(sni.as_str()));
            for conn in &mut ns.connections {
                conn.port = port;
            }
            Ok(ns)
        }
        other => Err(format!(
            "Unsupported DNS resolver scheme '{}'. Supported: dns://, dns+https://, dns+tls://, system://0.0.0.0",
            other
        )),
    }
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
    async fn test_parse_udp_resolver() {
        let ns = parse_resolver_item("dns://1.1.1.1").await.unwrap();
        assert_eq!(ns.ip, "1.1.1.1".parse::<IpAddr>().unwrap());
        assert_eq!(ns.connections[0].port, 53);

        let ns = parse_resolver_item("dns://8.8.8.8:5353").await.unwrap();
        assert_eq!(ns.ip, "8.8.8.8".parse::<IpAddr>().unwrap());
        assert_eq!(ns.connections[0].port, 5353);
    }

    #[tokio::test]
    async fn test_parse_bare_ip_resolvers() {
        let ns = parse_resolver_item("8.8.8.8").await.unwrap();
        assert_eq!(ns.ip, "8.8.8.8".parse::<IpAddr>().unwrap());
        assert_eq!(ns.connections[0].port, 53);

        let ns = parse_resolver_item("1.1.1.1:5353").await.unwrap();
        assert_eq!(ns.ip, "1.1.1.1".parse::<IpAddr>().unwrap());
        assert_eq!(ns.connections[0].port, 5353);

        let ns = parse_resolver_item("2001:4860:4860::8888").await.unwrap();
        assert_eq!(ns.ip, "2001:4860:4860::8888".parse::<IpAddr>().unwrap());
    }

    #[tokio::test]
    async fn test_parse_doh_resolver() {
        let ns = parse_resolver_item("dns+https://1.1.1.1?sni=cloudflare-dns.com")
            .await
            .unwrap();
        assert_eq!(ns.ip, "1.1.1.1".parse::<IpAddr>().unwrap());
        assert_eq!(ns.connections[0].port, 443);

        let ns_custom_port =
            parse_resolver_item("dns+https://1.1.1.1:8443/custom-query?sni=cloudflare-dns.com")
                .await
                .unwrap();
        assert_eq!(ns_custom_port.connections[0].port, 8443);
    }

    #[tokio::test]
    async fn test_parse_dot_resolver() {
        let ns = parse_resolver_item("dns+tls://8.8.8.8?sni=dns.google")
            .await
            .unwrap();
        assert_eq!(ns.ip, "8.8.8.8".parse::<IpAddr>().unwrap());
        assert_eq!(ns.connections[0].port, 853);
    }

    #[test]
    fn test_system_resolver_check() {
        assert!(is_system_resolver("system://0.0.0.0"));
        assert!(is_system_resolver("system://"));
        assert!(is_system_resolver("system"));
        assert!(!is_system_resolver("dns://1.1.1.1"));
    }

    #[tokio::test]
    async fn test_unsupported_scheme_error() {
        let err = parse_resolver_item("ftp://1.1.1.1").await.unwrap_err();
        assert!(err.contains("Unsupported DNS resolver scheme 'ftp'"));
    }
}
