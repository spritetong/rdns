//! Local network interface IP reader and intelligent address filter.

use crate::error::IpFetchError;
use ifaddrsx::get_interfaces;
pub use ifaddrsx::{Ipv4AddrExt, Ipv6AddrExt};
use regex::Regex;
use std::net::{Ipv4Addr, Ipv6Addr};

pub struct InterfaceIpFetcher {
    v4_interface_pattern: String,
    v4_interface_regex: Regex,
    v6_interface_pattern: String,
    v6_interface_regex: Regex,
    v6_prefix: Option<String>,
    v6_regex: Option<Regex>,
    prefer_slaac: bool,
    allow_private_v4: bool,
    allow_private_v6: bool,
}

impl InterfaceIpFetcher {
    pub fn try_new(
        v4_interface_pattern: Option<String>,
        v6_interface_pattern: Option<String>,
        v6_prefix: Option<String>,
        v6_regex: Option<String>,
        prefer_slaac: bool,
        allow_private_v4: bool,
        allow_private_v6: bool,
    ) -> Result<Self, IpFetchError> {
        let v4_str = v4_interface_pattern.unwrap_or_else(|| ".*".to_string());
        let v4_reg = Regex::new(&v4_str).map_err(|e| IpFetchError::InterfaceNotFound {
            name: format!("Invalid interface pattern '{}': {}", v4_str, e),
        })?;

        let v6_str = v6_interface_pattern.unwrap_or_else(|| ".*".to_string());
        let v6_reg = Regex::new(&v6_str).map_err(|e| IpFetchError::InterfaceNotFound {
            name: format!("Invalid interface pattern '{}': {}", v6_str, e),
        })?;

        let v6_filter_regex = if let Some(ref r) = v6_regex {
            Some(Regex::new(r).map_err(|e| IpFetchError::InterfaceNotFound {
                name: format!("Invalid ipv6_regex '{}': {}", r, e),
            })?)
        } else {
            None
        };

        Ok(Self {
            v4_interface_pattern: v4_str,
            v4_interface_regex: v4_reg,
            v6_interface_pattern: v6_str,
            v6_interface_regex: v6_reg,
            v6_prefix,
            v6_regex: v6_filter_regex,
            prefer_slaac,
            allow_private_v4,
            allow_private_v6,
        })
    }

    pub fn new(
        v4_interface_pattern: Option<String>,
        v6_interface_pattern: Option<String>,
        v6_prefix: Option<String>,
        v6_regex: Option<String>,
        prefer_slaac: bool,
        allow_private_v4: bool,
        allow_private_v6: bool,
    ) -> Self {
        Self::try_new(
            v4_interface_pattern,
            v6_interface_pattern,
            v6_prefix,
            v6_regex,
            prefer_slaac,
            allow_private_v4,
            allow_private_v6,
        )
        .unwrap_or_else(|e| {
            tracing::error!(
                error = %e,
                "Invalid pattern in InterfaceIpFetcher::new, defaulting to fallback"
            );
            Self {
                v4_interface_pattern: ".*".to_string(),
                v4_interface_regex: Regex::new(".*").expect("valid regex"),
                v6_interface_pattern: ".*".to_string(),
                v6_interface_regex: Regex::new(".*").expect("valid regex"),
                v6_prefix: None,
                v6_regex: None,
                prefer_slaac,
                allow_private_v4,
                allow_private_v6,
            }
        })
    }

    pub fn fetch_ipv4(&self) -> Result<Ipv4Addr, IpFetchError> {
        let interfaces = get_interfaces(true).map_err(IpFetchError::Io)?;
        let mut matched_interface = false;

        for iface in interfaces {
            if self.v4_interface_regex.is_match(&iface.name)
                || self.v4_interface_regex.is_match(iface.friendly_name())
            {
                matched_interface = true;
                for ip in iface.ipv4_addrs() {
                    if ip.is_loopback() {
                        continue;
                    }
                    if !self.allow_private_v4
                        && (ip.is_private() || ip.is_link_local() || ip.is_cgnat())
                    {
                        continue;
                    }
                    return Ok(ip);
                }
            }
        }

        if !matched_interface {
            Err(IpFetchError::InterfaceNotFound {
                name: self.v4_interface_pattern.clone(),
            })
        } else {
            Err(IpFetchError::NoPublicIpFound {
                name: self.v4_interface_pattern.clone(),
            })
        }
    }

    pub fn fetch_ipv6(&self) -> Result<Ipv6Addr, IpFetchError> {
        let interfaces = get_interfaces(true).map_err(IpFetchError::Io)?;
        let mut matched_interface = false;
        let mut candidates = Vec::new();

        for iface in interfaces {
            if self.v6_interface_regex.is_match(&iface.name)
                || self.v6_interface_regex.is_match(iface.friendly_name())
            {
                matched_interface = true;
                for ip in iface.ipv6_addrs() {
                    if ip.is_loopback() || ip.is_multicast() {
                        continue;
                    }
                    // Filter Link-Local (fe80::/10)
                    if ip.is_link_local_ipv6() {
                        continue;
                    }
                    // Filter ULA (fc00::/7)
                    if !self.allow_private_v6 && ip.is_unique_local_ipv6() {
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
                    if let Some(ref reg) = self.v6_regex {
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
                name: self.v6_interface_pattern.clone(),
            });
        }

        if candidates.is_empty() {
            return Err(IpFetchError::NoPublicIpFound {
                name: self.v6_interface_pattern.clone(),
            });
        }

        // 1. Prioritize SLAAC (EUI-64) address if prefer_slaac is enabled
        if self.prefer_slaac
            && let Some(&slaac_ip) = candidates.iter().find(|ip| ip.is_eui64_slaac())
        {
            return Ok(slaac_ip);
        }

        // 2. Prioritize stable (non-temporary) IPv6 addresses (D4)
        if let Some(&stable_ip) = candidates.iter().find(|ip| !ip.is_rfc4941_temporary()) {
            return Ok(stable_ip);
        }

        // 3. Fallback to temporary address if no stable address is available
        tracing::warn!(
            interface = %self.v6_interface_pattern,
            "Only RFC 4941 temporary IPv6 address found; using as fallback"
        );
        Ok(candidates[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_print_all_interfaces() {
        if let Ok(interfaces) = ifaddrsx::get_interfaces(false) {
            for iface in interfaces {
                println!(
                    "IFACE: '{}' (friendly: '{}', is_up: {}, mac: {:?}), IPs: {:?}",
                    iface.name,
                    iface.friendly_name(),
                    iface.is_up(),
                    iface.mac_addr,
                    iface.ips
                );
            }
        }
    }

    #[test]
    fn test_is_eui64_slaac() {
        let slaac: Ipv6Addr = "240e:3a1:ec9:e531:dabb:c1ff:fe67:6221".parse().unwrap();
        assert!(slaac.is_eui64_slaac());

        let dhcp: Ipv6Addr = "240e:3a1:ec9:e531::737".parse().unwrap();
        assert!(!dhcp.is_eui64_slaac());

        let loopback: Ipv6Addr = "::1".parse().unwrap();
        assert!(!loopback.is_eui64_slaac());
    }

    #[test]
    fn test_is_rfc4941_temporary() {
        // SLAAC EUI-64 is stable, NOT temporary
        let slaac: Ipv6Addr = "240e:3a1:ec9:e531:dabb:c1ff:fe67:6221".parse().unwrap();
        assert!(!slaac.is_rfc4941_temporary());

        // DHCPv6 / static low suffix is stable, NOT temporary
        let dhcp: Ipv6Addr = "240e:3a1:ec9:e531::737".parse().unwrap();
        assert!(!dhcp.is_rfc4941_temporary());

        let static_ip: Ipv6Addr = "240e:3a1:ec9:e531::1".parse().unwrap();
        assert!(!static_ip.is_rfc4941_temporary());

        // RFC 4941 privacy temporary address: randomized 64-bit IID with u-bit=0
        // e.g. 0xe852 (1110 1000, bit 1 is 0)
        let temp_ip: Ipv6Addr = "240e:3a1:ec9:e531:e852:6a12:b37e:8914".parse().unwrap();
        assert!(temp_ip.is_rfc4941_temporary());
    }

    #[test]
    fn test_is_cgnat() {
        let cgnat_ip: Ipv4Addr = "100.64.0.1".parse().unwrap();
        assert!(cgnat_ip.is_cgnat());

        let public_ip: Ipv4Addr = "8.8.8.8".parse().unwrap();
        assert!(!public_ip.is_cgnat());

        let private_ip: Ipv4Addr = "192.168.1.1".parse().unwrap();
        assert!(!private_ip.is_cgnat());
    }

    #[test]
    fn test_eui64_mac_roundtrip() {
        let slaac: Ipv6Addr = "240e:3a1:ec9:e531:dabb:c1ff:fe67:6221".parse().unwrap();
        let mac = slaac.mac_from_eui64_slaac();
        assert!(mac.is_some());
        let mac = mac.unwrap();

        let prefix: Ipv6Addr = "240e:3a1:ec9:e531::".parse().unwrap();
        let synthesized = Ipv6Addr::from_mac_slaac(&prefix, &mac);
        assert_eq!(synthesized, slaac);
    }
}
