//! Local network interface IP reader and intelligent address filter.

use crate::error::IpFetchError;
use regex::Regex;
use std::net::{Ipv4Addr, Ipv6Addr};

pub struct InterfaceIpFetcher {
    v4_interface_pattern: Option<String>,
    v6_interface_pattern: Option<String>,
    v6_prefix: Option<String>,
    allow_private_v4: bool,
    allow_private_v6: bool,
}

impl InterfaceIpFetcher {
    pub fn new(
        v4_interface_pattern: Option<String>,
        v6_interface_pattern: Option<String>,
        v6_prefix: Option<String>,
        allow_private_v4: bool,
        allow_private_v6: bool,
    ) -> Self {
        Self {
            v4_interface_pattern,
            v6_interface_pattern,
            v6_prefix,
            allow_private_v4,
            allow_private_v6,
        }
    }

    pub fn fetch_ipv4(&self) -> Result<Ipv4Addr, IpFetchError> {
        let pattern_str = self.v4_interface_pattern.as_deref().unwrap_or(".*");
        let regex = Regex::new(pattern_str).map_err(|e| IpFetchError::InterfaceNotFound {
            name: format!("Invalid interface pattern '{}': {}", pattern_str, e),
        })?;

        let if_addrs = get_if_addrs::get_if_addrs().map_err(IpFetchError::Io)?;
        let mut matched_interface = false;

        for iface in if_addrs {
            if regex.is_match(&iface.name) {
                matched_interface = true;
                if let std::net::IpAddr::V4(ip) = iface.addr.ip() {
                    if ip.is_loopback() {
                        continue;
                    }
                    if !self.allow_private_v4
                        && (ip.is_private() || ip.is_link_local() || is_cgnat(ip))
                    {
                        continue;
                    }
                    return Ok(ip);
                }
            }
        }

        if !matched_interface {
            Err(IpFetchError::InterfaceNotFound {
                name: pattern_str.to_string(),
            })
        } else {
            Err(IpFetchError::NoPublicIpFound {
                name: pattern_str.to_string(),
            })
        }
    }

    pub fn fetch_ipv6(&self) -> Result<Ipv6Addr, IpFetchError> {
        let pattern_str = self.v6_interface_pattern.as_deref().unwrap_or(".*");
        let regex = Regex::new(pattern_str).map_err(|e| IpFetchError::InterfaceNotFound {
            name: format!("Invalid interface pattern '{}': {}", pattern_str, e),
        })?;

        let if_addrs = get_if_addrs::get_if_addrs().map_err(IpFetchError::Io)?;
        let mut matched_interface = false;

        for iface in if_addrs {
            if regex.is_match(&iface.name) {
                matched_interface = true;
                if let std::net::IpAddr::V6(ip) = iface.addr.ip() {
                    if ip.is_loopback() || ip.is_multicast() {
                        continue;
                    }
                    // Filter Link-Local (fe80::/10)
                    let segments = ip.segments();
                    if (segments[0] & 0xffc0) == 0xfe80 {
                        continue;
                    }
                    // Filter ULA (fc00::/7, starts with 0xfc or 0xfd)
                    if !self.allow_private_v6 && (segments[0] & 0xfe00) == 0xfc00 {
                        continue;
                    }
                    // Check prefix if specified
                    if let Some(ref prefix) = self.v6_prefix {
                        let ip_str = ip.to_string();
                        if !ip_str.starts_with(prefix) {
                            continue;
                        }
                    }
                    return Ok(ip);
                }
            }
        }

        if !matched_interface {
            Err(IpFetchError::InterfaceNotFound {
                name: pattern_str.to_string(),
            })
        } else {
            Err(IpFetchError::NoPublicIpFound {
                name: pattern_str.to_string(),
            })
        }
    }
}

fn is_cgnat(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (octets[1] >= 64 && octets[1] <= 127)
}
