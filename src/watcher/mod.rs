// Copyright (c) 2026 Sprite Tong <spritetong@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later
// rdns is licensed under the GNU GPL v3.0 or later.

//! Network change monitoring powered by `netwatcher`.
//!
//! Provides event-driven network change detection to eliminate empty polling,
//! with virtual adapter noise filtering, robust debounce coalescing, and
//! Modern Standby DNS readiness verification.

use crate::ip::DnsResolver;
use netwatcher::async_adapter::Tokio;
use netwatcher::Update;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// Determine whether a network interface name corresponds to a virtual, container,
/// loopback, or VPN adapter that should be filtered out to eliminate spurious wakeups.
pub fn is_virtual_or_ignored_adapter(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("vethernet")
        || lower.contains("loopback")
        || lower.contains("virtual")
        || lower.contains("wsl")
        || lower.contains("docker")
        || lower.contains("vmnet")
        || lower.contains("tap")
        || lower.contains("tun")
}

/// Determine whether an IP address is a valid, active, non-link-local address.
pub fn is_valid_active_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => !v4.is_loopback() && !v4.is_link_local(),
        IpAddr::V6(v6) => !v6.is_loopback() && !is_link_local_v6(v6),
    }
}

fn is_link_local_v6(v6: &std::net::Ipv6Addr) -> bool {
    let segments = v6.segments();
    (segments[0] & 0xffc0) == 0xfe80
}

/// Verify if an interface possesses at least one valid, active, non-link-local IP.
pub fn has_valid_active_ip(iface: &netwatcher::Interface) -> bool {
    iface.ips.iter().any(|rec| is_valid_active_ip(&rec.ip))
}

/// Inspect an update from `netwatcher` and determine if it represents a meaningful
/// physical or routable network change, filtering out virtual adapter noise.
pub fn is_meaningful_network_update(
    update: &Update,
    configured_interfaces: &[String],
) -> bool {
    if update.is_initial {
        return false;
    }

    let is_ignored = |name: &str| -> bool {
        // If explicitly configured in user config (by name, substring, or regex), do not ignore
        if configured_interfaces.iter().any(|c| {
            let trimmed = c.trim();
            if trimmed.is_empty() || trimmed == ".*" {
                return false;
            }
            trimmed.eq_ignore_ascii_case(name)
                || name.to_ascii_lowercase().contains(&trimmed.to_ascii_lowercase())
                || regex::Regex::new(trimmed).map(|r| r.is_match(name)).unwrap_or(false)
        }) {
            return false;
        }
        is_virtual_or_ignored_adapter(name)
    };

    // Check newly added interfaces
    for iface in update.diff.added.values() {
        if !is_ignored(&iface.name) && has_valid_active_ip(iface) {
            return true;
        }
    }

    // Check removed interfaces
    for iface in update.diff.removed.values() {
        if !is_ignored(&iface.name) {
            return true;
        }
    }

    // Check modified interfaces
    for (index, diff) in &update.diff.modified {
        if let Some(iface) = update.interfaces.get(index) {
            if is_ignored(&iface.name) {
                continue;
            }
            let has_added_active = diff
                .addrs_added
                .iter()
                .any(|rec| is_valid_active_ip(&rec.ip));
            let has_removed_active = diff
                .addrs_removed
                .iter()
                .any(|rec| is_valid_active_ip(&rec.ip));
            if has_added_active || has_removed_active || diff.name_changed {
                return true;
            }
        }
    }

    false
}

/// Probe DNS availability to handle Modern Standby / Sleep wakeup.
///
/// When a device wakes from sleep, network interfaces may report an IP before the
/// system DNS service (`Dnscache` on Windows or local resolver) is fully operational.
/// This lightweight probe queries targets with short retries before dispatching tasks.
pub async fn wait_for_dns_readiness(
    dns_resolver: &DnsResolver,
    dns_server: Option<&str>,
    probe_targets: &[String],
    max_wait: Duration,
) -> bool {
    let start = Instant::now();
    let default_fallbacks = ["one.one.one.one", "dns.google"];
    let mut attempt = 0;

    while start.elapsed() < max_wait {
        attempt += 1;
        let target = if !probe_targets.is_empty() {
            &probe_targets[(attempt - 1) % probe_targets.len()]
        } else {
            default_fallbacks[(attempt - 1) % default_fallbacks.len()]
        };

        let query_timeout = Duration::from_millis(500);
        match dns_resolver.resolve(target, dns_server, query_timeout).await {
            Ok(_) => {
                tracing::debug!(
                    "[netwatcher] DNS readiness verified via '{}' on attempt {}",
                    target,
                    attempt
                );
                return true;
            }
            Err(e) => {
                tracing::debug!(
                    "[netwatcher] DNS probe attempt {} for '{}' failed ({}), retrying...",
                    attempt,
                    target,
                    e
                );
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    }

    tracing::warn!(
        "[netwatcher] DNS readiness probe timed out after {}s; proceeding with update",
        max_wait.as_secs()
    );
    false
}

/// Background network interface change watcher.
pub struct NetworkWatcher {
    notifier: Arc<Notify>,
    debounce_duration: Duration,
    configured_interfaces: Vec<String>,
    probe_targets: Vec<String>,
    dns_resolver: DnsResolver,
    dns_server: Option<String>,
}

impl NetworkWatcher {
    pub fn new(
        notifier: Arc<Notify>,
        debounce_ms: u64,
        configured_interfaces: Vec<String>,
        probe_targets: Vec<String>,
        dns_resolver: DnsResolver,
        dns_server: Option<String>,
    ) -> Self {
        Self {
            notifier,
            debounce_duration: Duration::from_millis(debounce_ms),
            configured_interfaces,
            probe_targets,
            dns_resolver,
            dns_server,
        }
    }

    /// Run the watcher loop until cancellation.
    pub async fn run_loop(&self, token: CancellationToken) {
        let mut watch = match netwatcher::watch_interfaces_async::<Tokio>() {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(
                    "[netwatcher] Network watcher unavailable ({}); falling back to timer-based polling",
                    e
                );
                return;
            }
        };

        tracing::info!(
            "[netwatcher] Network change watcher started (debounce: {}ms)",
            self.debounce_duration.as_millis()
        );

        let min_cooldown = Duration::from_secs(5);
        let mut last_notify_time = Instant::now() - min_cooldown;

        loop {
            // 1. Wait for next interface update or cancellation
            let update = tokio::select! {
                _ = token.cancelled() => break,
                upd = watch.changed() => upd,
            };

            // 2. Filter virtual adapters and non-actionable diffs
            if !is_meaningful_network_update(&update, &self.configured_interfaces) {
                continue;
            }

            tracing::info!(
                "[netwatcher] Network change detected, waiting {}ms to settle",
                self.debounce_duration.as_millis()
            );

            // 3. Robust Debounce Window: settle intermediate micro-events (link up -> 169.254 -> gateway -> real IP)
            let debounce_deadline = tokio::time::sleep(self.debounce_duration);
            tokio::pin!(debounce_deadline);

            let mut cancelled = false;
            loop {
                tokio::select! {
                    _ = token.cancelled() => {
                        cancelled = true;
                        break;
                    }
                    _ = &mut debounce_deadline => {
                        // Settle period finished without cancellation
                        break;
                    }
                    _ = watch.changed() => {
                        // Absorb subsequent micro-events during debounce window
                    }
                }
            }

            if cancelled {
                break;
            }

            // 4. Rate-limiting cooldown: avoid triggering multiple times in short succession
            let now = Instant::now();
            if now.duration_since(last_notify_time) < min_cooldown {
                let remaining = min_cooldown - now.duration_since(last_notify_time);
                tokio::select! {
                    _ = token.cancelled() => break,
                    _ = tokio::time::sleep(remaining) => {}
                }
            }

            // 5. Modern Standby / Wake protection: Probe DNS readiness before triggering tasks
            wait_for_dns_readiness(
                &self.dns_resolver,
                self.dns_server.as_deref(),
                &self.probe_targets,
                Duration::from_secs(3),
            )
            .await;

            // 6. Notify all interface schedulers to execute immediately
            last_notify_time = Instant::now();
            self.notifier.notify_waiters();
        }

        tracing::info!("[netwatcher] Network change watcher terminated cleanly");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_virtual_adapter_filtering() {
        assert!(is_virtual_or_ignored_adapter("vEthernet (WSL)"));
        assert!(is_virtual_or_ignored_adapter("vEthernet (Default Switch)"));
        assert!(is_virtual_or_ignored_adapter("VirtualBox Host-Only Ethernet Adapter"));
        assert!(is_virtual_or_ignored_adapter("VMware Network Adapter VMnet1"));
        assert!(is_virtual_or_ignored_adapter("Loopback Pseudo-Interface 1"));
        assert!(is_virtual_or_ignored_adapter("docker0"));
        assert!(is_virtual_or_ignored_adapter("tap-windows6"));
        assert!(is_virtual_or_ignored_adapter("tun0"));

        assert!(!is_virtual_or_ignored_adapter("Ethernet"));
        assert!(!is_virtual_or_ignored_adapter("Wi-Fi"));
        assert!(!is_virtual_or_ignored_adapter("eth0"));
        assert!(!is_virtual_or_ignored_adapter("wlan0"));
        assert!(!is_virtual_or_ignored_adapter("enp3s0"));
    }

    #[test]
    fn test_valid_active_ip() {
        let v4_loopback: IpAddr = "127.0.0.1".parse().unwrap();
        let v4_link_local: IpAddr = "169.254.10.20".parse().unwrap();
        let v4_public: IpAddr = "198.51.100.1".parse().unwrap();
        let v4_private: IpAddr = "192.168.1.100".parse().unwrap();

        assert!(!is_valid_active_ip(&v4_loopback));
        assert!(!is_valid_active_ip(&v4_link_local));
        assert!(is_valid_active_ip(&v4_public));
        assert!(is_valid_active_ip(&v4_private));

        let v6_loopback: IpAddr = "::1".parse().unwrap();
        let v6_link_local: IpAddr = "fe80::1ff:fe23:4567:890a".parse().unwrap();
        let v6_global: IpAddr = "2001:db8::1".parse().unwrap();

        assert!(!is_valid_active_ip(&v6_loopback));
        assert!(!is_valid_active_ip(&v6_link_local));
        assert!(is_valid_active_ip(&v6_global));
    }

    #[test]
    fn test_configured_virtual_adapter_whitelist() {
        let configured = vec!["tun0".to_string(), "vEthernet".to_string()];
        let is_ignored = |name: &str| -> bool {
            if configured.iter().any(|c| {
                let trimmed = c.trim();
                if trimmed.is_empty() || trimmed == ".*" {
                    return false;
                }
                trimmed.eq_ignore_ascii_case(name)
                    || name.to_ascii_lowercase().contains(&trimmed.to_ascii_lowercase())
                    || regex::Regex::new(trimmed).map(|r| r.is_match(name)).unwrap_or(false)
            }) {
                return false;
            }
            is_virtual_or_ignored_adapter(name)
        };

        // Explicitly configured virtual adapters should NOT be ignored
        assert!(!is_ignored("tun0"));
        assert!(!is_ignored("vEthernet (WSL)"));

        // Unconfigured virtual adapters SHOULD be ignored
        assert!(is_ignored("docker0"));
        assert!(is_ignored("tap-windows6"));
        assert!(is_ignored("VMware Network Adapter VMnet1"));

        // Physical adapters are never ignored regardless of config
        assert!(!is_ignored("Ethernet"));
        assert!(!is_ignored("Wi-Fi"));
    }
}
