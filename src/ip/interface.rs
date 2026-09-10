//! Local network interface IP reader and intelligent address filter.

use crate::error::IpFetchError;
pub use ifaddrsx::is_eui64_slaac;
use ifaddrsx::{get_interfaces, is_link_local_ipv6, is_unique_local_ipv6};
use regex::Regex;
use std::net::{Ipv4Addr, Ipv6Addr};

pub struct InterfaceIpFetcher {
    v4_interface_pattern: Option<String>,
    v6_interface_pattern: Option<String>,
    v6_prefix: Option<String>,
    v6_regex: Option<String>,
    prefer_slaac: bool,
    allow_private_v4: bool,
    allow_private_v6: bool,
}

impl InterfaceIpFetcher {
    pub fn new(
        v4_interface_pattern: Option<String>,
        v6_interface_pattern: Option<String>,
        v6_prefix: Option<String>,
        v6_regex: Option<String>,
        prefer_slaac: bool,
        allow_private_v4: bool,
        allow_private_v6: bool,
    ) -> Self {
        Self {
            v4_interface_pattern,
            v6_interface_pattern,
            v6_prefix,
            v6_regex,
            prefer_slaac,
            allow_private_v4,
            allow_private_v6,
        }
    }

    pub fn fetch_ipv4(&self) -> Result<Ipv4Addr, IpFetchError> {
        let pattern_str = self.v4_interface_pattern.as_deref().unwrap_or(".*");
        let regex = Regex::new(pattern_str).map_err(|e| IpFetchError::InterfaceNotFound {
            name: format!("Invalid interface pattern '{}': {}", pattern_str, e),
        })?;

        let interfaces = get_interfaces(true).map_err(IpFetchError::Io)?;
        let mut matched_interface = false;

        for iface in interfaces {
            if regex.is_match(&iface.name) || regex.is_match(iface.friendly_name()) {
                matched_interface = true;
                for ip in iface.ipv4_addrs() {
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

        let v6_filter_regex = if let Some(ref r) = self.v6_regex {
            Some(Regex::new(r).map_err(|e| IpFetchError::InterfaceNotFound {
                name: format!("Invalid ipv6_regex '{}': {}", r, e),
            })?)
        } else {
            None
        };

        let interfaces = get_interfaces(true).map_err(IpFetchError::Io)?;
        let mut matched_interface = false;
        let mut candidates = Vec::new();

        for iface in interfaces {
            if regex.is_match(&iface.name) || regex.is_match(iface.friendly_name()) {
                matched_interface = true;
                for ip in iface.ipv6_addrs() {
                    if ip.is_loopback() || ip.is_multicast() {
                        continue;
                    }
                    // Filter Link-Local (fe80::/10)
                    if is_link_local_ipv6(&ip) {
                        continue;
                    }
                    // Filter ULA (fc00::/7)
                    if !self.allow_private_v6 && is_unique_local_ipv6(&ip) {
                        continue;
                    }
                    // Check prefix if specified
                    if let Some(ref prefix) = self.v6_prefix {
                        let ip_str = ip.to_string();
                        if !ip_str.starts_with(prefix) {
                            continue;
                        }
                    }
                    // Check regex filter if specified
                    if let Some(ref reg) = v6_filter_regex {
                        let ip_str = ip.to_string();
                        if !reg.is_match(&ip_str) {
                            continue;
                        }
                    }
                    candidates.push(ip);
                }
            }
        }

        if !matched_interface {
            return Err(IpFetchError::InterfaceNotFound {
                name: pattern_str.to_string(),
            });
        }

        if candidates.is_empty() {
            return Err(IpFetchError::NoPublicIpFound {
                name: pattern_str.to_string(),
            });
        }

        // Prioritize SLAAC (EUI-64) address if prefer_slaac is enabled
        if self.prefer_slaac
            && let Some(&slaac_ip) = candidates.iter().find(|ip| is_eui64_slaac(ip))
        {
            return Ok(slaac_ip);
        }

        Ok(candidates[0])
    }
}

fn is_cgnat(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (octets[1] >= 64 && octets[1] <= 127)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_print_all_interfaces() {
        if let Ok(interfaces) = ifaddrsx::get_interfaces(false) {
            for iface in interfaces {
                println!(
                    "IFACE: '{}' (friendly: '{}'), IPs: {:?}",
                    iface.name,
                    iface.friendly_name(),
                    iface.ips
                );
            }
        }
    }

    #[test]
    fn test_is_eui64_slaac() {
        let slaac: Ipv6Addr = "240e:3a1:ec9:e531:dabb:c1ff:fe67:6221".parse().unwrap();
        assert!(is_eui64_slaac(&slaac));

        let dhcp: Ipv6Addr = "240e:3a1:ec9:e531::737".parse().unwrap();
        assert!(!is_eui64_slaac(&dhcp));

        let loopback: Ipv6Addr = "::1".parse().unwrap();
        assert!(!is_eui64_slaac(&loopback));
    }
}
