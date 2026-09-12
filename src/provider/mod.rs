// Copyright (c) 2026 Sprite Tong <spritetong@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later
// rdns is licensed under the GNU GPL v3.0 or later.

//! Predefined DDNS provider templates and registry.

use crate::config::RequestConfig;

/// Definition of a predefined DDNS service provider.
#[derive(Debug, Clone)]
pub struct Provider {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub website: &'static str,
    pub requires_domain: bool,
    pub required_args: &'static [&'static str],
    pub optional_args: &'static [&'static str],
    pub default_method: &'static str,
    pub default_url: &'static str,
    pub default_headers: &'static [(&'static str, &'static str)],
    pub default_body: Option<&'static str>,
    pub default_success_regex: Option<&'static str>,
    pub default_success_contains: &'static [&'static str],
    pub example_yaml: &'static str,
}

impl Provider {
    /// Instantiate default RequestConfig for this provider.
    pub fn default_request(&self) -> RequestConfig {
        RequestConfig {
            method: Some(self.default_method.to_string()),
            url: Some(self.default_url.to_string()),
            headers: self
                .default_headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: self.default_body.map(str::to_string),
            success_regex: self.default_success_regex.map(|s| s.to_string()),
            success_contains: Some(
                self.default_success_contains
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            ),
            tls_insecure: Some(false),
            proxy: None,
            cacerts: None,
        }
    }
}

/// Static registry of built-in providers.
pub static PROVIDERS: &[Provider] = &[
    Provider {
        name: "dynu",
        aliases: &[],
        description: "Dynu Systems DDNS (Dual-stack IPv4 & IPv6)",
        website: "https://www.dynu.com",
        requires_domain: true,
        required_args: &["password"],
        optional_args: &["username"],
        default_method: "GET",
        default_url: "https://api.dynu.com/nic/update?hostname={{domain}}&myip={{ipv4}}&myipv6={{ipv6}}&password={{password}}",
        default_headers: &[],
        default_body: None,
        default_success_regex: Some("^(good|nochg)"),
        default_success_contains: &[],
        example_yaml: r#"  - name: "dynu-dualstack"
    provider: "dynu"
    domain: "yourname.freeddns.org"
    args:
      password: "${DYNU_PASSWORD}""#,
    },
    Provider {
        name: "dynu-ipv4",
        aliases: &[],
        description: "Dynu Systems DDNS (IPv4 only)",
        website: "https://www.dynu.com",
        requires_domain: true,
        required_args: &["password"],
        optional_args: &["username"],
        default_method: "GET",
        default_url: "https://api.dynu.com/nic/update?hostname={{domain}}&myip={{ipv4}}&myipv6=no&password={{password}}",
        default_headers: &[],
        default_body: None,
        default_success_regex: Some("^(good|nochg)"),
        default_success_contains: &[],
        example_yaml: r#"  - name: "dynu-v4"
    provider: "dynu-ipv4"
    domain: "yourname.freeddns.org"
    args:
      password: "${DYNU_PASSWORD}""#,
    },
    Provider {
        name: "dynu-ipv6",
        aliases: &[],
        description: "Dynu Systems DDNS (IPv6 only)",
        website: "https://www.dynu.com",
        requires_domain: true,
        required_args: &["password"],
        optional_args: &["username"],
        default_method: "GET",
        default_url: "https://api.dynu.com/nic/update?hostname={{domain}}&myip=no&myipv6={{ipv6}}&password={{password}}",
        default_headers: &[],
        default_body: None,
        default_success_regex: Some("^(good|nochg)"),
        default_success_contains: &[],
        example_yaml: r#"  - name: "dynu-v6"
    provider: "dynu-ipv6"
    domain: "yourname.freeddns.org"
    args:
      password: "${DYNU_PASSWORD}""#,
    },
    Provider {
        name: "dynv6",
        aliases: &[],
        description: "dynv6 Free Dynamic DNS (Dual-stack IPv4 & IPv6)",
        website: "https://dynv6.com",
        requires_domain: true,
        required_args: &["token"],
        optional_args: &[],
        default_method: "GET",
        default_url: "https://dynv6.com/api/update?hostname={{domain}}&token={{token}}&ipv4={{ipv4}}&ipv6={{ipv6}}",
        default_headers: &[],
        default_body: None,
        default_success_regex: Some("(?i)^(addresses updated|addresses unchanged|unchanged)"),
        default_success_contains: &[],
        example_yaml: r#"  - name: "dynv6-dualstack"
    provider: "dynv6"
    domain: "yourname.dynv6.net"
    args:
      token: "${DYNV6_TOKEN}""#,
    },
    Provider {
        name: "dynv6-ipv4",
        aliases: &[],
        description: "dynv6 Free Dynamic DNS (IPv4 only)",
        website: "https://dynv6.com",
        requires_domain: true,
        required_args: &["token"],
        optional_args: &[],
        default_method: "GET",
        default_url: "https://dynv6.com/api/update?hostname={{domain}}&token={{token}}&ipv4={{ipv4}}",
        default_headers: &[],
        default_body: None,
        default_success_regex: Some("(?i)^(addresses updated|addresses unchanged|unchanged)"),
        default_success_contains: &[],
        example_yaml: r#"  - name: "dynv6-v4"
    provider: "dynv6-ipv4"
    domain: "yourname.dynv6.net"
    args:
      token: "${DYNV6_TOKEN}""#,
    },
    Provider {
        name: "dynv6-ipv6",
        aliases: &[],
        description: "dynv6 Free Dynamic DNS (IPv6 only)",
        website: "https://dynv6.com",
        requires_domain: true,
        required_args: &["token"],
        optional_args: &[],
        default_method: "GET",
        default_url: "https://dynv6.com/api/update?hostname={{domain}}&token={{token}}&ipv6={{ipv6}}",
        default_headers: &[],
        default_body: None,
        default_success_regex: Some("(?i)^(addresses updated|addresses unchanged|unchanged)"),
        default_success_contains: &[],
        example_yaml: r#"  - name: "dynv6-v6"
    provider: "dynv6-ipv6"
    domain: "yourname.dynv6.net"
    args:
      token: "${DYNV6_TOKEN}""#,
    },
    Provider {
        name: "duckdns",
        aliases: &[],
        description: "DuckDNS Free Dynamic DNS (Dual-stack IPv4 & IPv6)",
        website: "https://www.duckdns.org",
        requires_domain: true,
        required_args: &["token"],
        optional_args: &[],
        default_method: "GET",
        default_url: "https://www.duckdns.org/update?domains={{domain}}&token={{token}}&ip={{ipv4}}&ipv6={{ipv6}}",
        default_headers: &[],
        default_body: None,
        default_success_regex: Some("^OK"),
        default_success_contains: &["OK"],
        example_yaml: r#"  - name: "duckdns-dualstack"
    provider: "duckdns"
    domain: "yourdomain"
    args:
      token: "${DUCKDNS_TOKEN}""#,
    },
    Provider {
        name: "duckdns-ipv4",
        aliases: &[],
        description: "DuckDNS Free Dynamic DNS (IPv4 only)",
        website: "https://www.duckdns.org",
        requires_domain: true,
        required_args: &["token"],
        optional_args: &[],
        default_method: "GET",
        default_url: "https://www.duckdns.org/update?domains={{domain}}&token={{token}}&ip={{ipv4}}",
        default_headers: &[],
        default_body: None,
        default_success_regex: Some("^OK"),
        default_success_contains: &["OK"],
        example_yaml: r#"  - name: "duckdns-v4"
    provider: "duckdns-ipv4"
    domain: "yourdomain"
    args:
      token: "${DUCKDNS_TOKEN}""#,
    },
    Provider {
        name: "duckdns-ipv6",
        aliases: &[],
        description: "DuckDNS Free Dynamic DNS (IPv6 only)",
        website: "https://www.duckdns.org",
        requires_domain: true,
        required_args: &["token"],
        optional_args: &[],
        default_method: "GET",
        default_url: "https://www.duckdns.org/update?domains={{domain}}&token={{token}}&ipv6={{ipv6}}",
        default_headers: &[],
        default_body: None,
        default_success_regex: Some("^OK"),
        default_success_contains: &["OK"],
        example_yaml: r#"  - name: "duckdns-v6"
    provider: "duckdns-ipv6"
    domain: "yourdomain"
    args:
      token: "${DUCKDNS_TOKEN}""#,
    },
    Provider {
        name: "he",
        aliases: &["hurricane-electric"],
        description: "Hurricane Electric Dynamic DNS (IPv4)",
        website: "https://dns.he.net",
        requires_domain: true,
        required_args: &["password"],
        optional_args: &[],
        default_method: "GET",
        default_url: "https://dyn.dns.he.net/nic/update?hostname={{domain}}&password={{password}}&myip={{ipv4}}",
        default_headers: &[],
        default_body: None,
        default_success_regex: Some("^(good|nochg)"),
        default_success_contains: &[],
        example_yaml: r#"  - name: "he-v4"
    provider: "he"
    domain: "yourname.dyn.he.net"
    args:
      password: "${HE_PASSWORD}""#,
    },
    Provider {
        name: "noip",
        aliases: &[],
        description: "No-IP Dynamic Update Client HTTP API (IPv4)",
        website: "https://www.noip.com",
        requires_domain: true,
        required_args: &["username", "password"],
        optional_args: &[],
        default_method: "GET",
        default_url: "https://dynupdate.no-ip.com/nic/update?hostname={{domain}}&myip={{ipv4}}&username={{username}}&password={{password}}",
        default_headers: &[],
        default_body: None,
        default_success_regex: Some("^(good|nochg)"),
        default_success_contains: &[],
        example_yaml: r#"  - name: "noip-v4"
    provider: "noip"
    domain: "yourname.ddns.net"
    args:
      username: "${NOIP_USER}"
      password: "${NOIP_PASSWORD}""#,
    },
    Provider {
        name: "cloudflare",
        aliases: &["cloudflare-dualstack", "cf-dualstack", "cf"],
        description: "Cloudflare DNS API v4 (Dual-stack IPv4 & IPv6 batch update)",
        website: "https://dash.cloudflare.com",
        requires_domain: true,
        required_args: &["token", "zone_id", "record_id_v4", "record_id_v6"],
        optional_args: &[],
        default_method: "POST",
        default_url: "https://api.cloudflare.com/client/v4/zones/{{zone_id}}/dns_records/batch",
        default_headers: &[
            ("Authorization", "Bearer {{token}}"),
            ("Content-Type", "application/json"),
        ],
        default_body: Some(r#"{"patches":[{"id":"{{record_id_v4}}","content":"{{ipv4}}"},{"id":"{{record_id_v6}}","content":"{{ipv6}}"}]}"#),
        default_success_regex: Some(r#""success"\s*:\s*true"#),
        default_success_contains: &[],
        example_yaml: r#"  - name: "cloudflare-dualstack"
    provider: "cloudflare"
    domain: "sub.example.com"
    args:
      token: "${CF_API_TOKEN}"
      zone_id: "${CF_ZONE_ID}"
      record_id_v4: "${CF_RECORD_ID_V4}"
      record_id_v6: "${CF_RECORD_ID_V6}""#,
    },
    Provider {
        name: "cloudflare-v4",
        aliases: &["cf-v4", "cloudflare-ipv4"],
        description: "Cloudflare DNS API v4 (IPv4 A record)",
        website: "https://dash.cloudflare.com",
        requires_domain: true,
        required_args: &["token", "zone_id", "record_id"],
        optional_args: &[],
        default_method: "PATCH",
        default_url: "https://api.cloudflare.com/client/v4/zones/{{zone_id}}/dns_records/{{record_id}}",
        default_headers: &[
            ("Authorization", "Bearer {{token}}"),
            ("Content-Type", "application/json"),
        ],
        default_body: Some(r#"{"content":"{{ipv4}}"}"#),
        default_success_regex: Some(r#""success"\s*:\s*true"#),
        default_success_contains: &[],
        example_yaml: r#"  - name: "cloudflare-v4"
    provider: "cloudflare-v4"
    domain: "sub.example.com"
    args:
      token: "${CF_API_TOKEN}"
      zone_id: "${CF_ZONE_ID}"
      record_id: "${CF_RECORD_ID}""#,
    },
    Provider {
        name: "cloudflare-v6",
        aliases: &["cf-v6", "cloudflare-ipv6"],
        description: "Cloudflare DNS API v4 (IPv6 AAAA record)",
        website: "https://dash.cloudflare.com",
        requires_domain: true,
        required_args: &["token", "zone_id", "record_id"],
        optional_args: &[],
        default_method: "PATCH",
        default_url: "https://api.cloudflare.com/client/v4/zones/{{zone_id}}/dns_records/{{record_id}}",
        default_headers: &[
            ("Authorization", "Bearer {{token}}"),
            ("Content-Type", "application/json"),
        ],
        default_body: Some(r#"{"content":"{{ipv6}}"}"#),
        default_success_regex: Some(r#""success"\s*:\s*true"#),
        default_success_contains: &[],
        example_yaml: r#"  - name: "cloudflare-v6"
    provider: "cloudflare-v6"
    domain: "sub.example.com"
    args:
      token: "${CF_API_TOKEN}"
      zone_id: "${CF_ZONE_ID}"
      record_id: "${CF_RECORD_ID}""#,
    },
];

/// Find a provider by name or alias (case-insensitive).
pub fn get_provider(name: &str) -> Option<&'static Provider> {
    let lower = name.trim().to_lowercase();
    PROVIDERS.iter().find(|p| {
        p.name.eq_ignore_ascii_case(&lower)
            || p.aliases.iter().any(|a| a.eq_ignore_ascii_case(&lower))
    })
}

/// Return all available providers.
pub fn list_providers() -> &'static [Provider] {
    PROVIDERS
}

/// Comma-separated list of supported provider names.
pub fn supported_providers_str() -> String {
    PROVIDERS
        .iter()
        .map(|p| p.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Format the entire provider list as a human-readable summary for CLI output.
pub fn format_providers_list() -> String {
    let mut out = String::from("Available Predefined DDNS Providers:\n\n");
    for (i, p) in PROVIDERS.iter().enumerate() {
        let alias_str = if p.aliases.is_empty() {
            String::new()
        } else {
            format!(" (aliases: {})", p.aliases.join(", "))
        };
        out.push_str(&format!("{}. {}{}\n", i + 1, p.name, alias_str));
        out.push_str(&format!("   Description: {}\n", p.description));
        out.push_str(&format!("   Website:     {}\n", p.website));
        out.push_str(&format!("   URL:         {}\n", p.default_url));
        out.push_str(&format!(
            "   Method:      {} | Required Args: {}\n",
            p.default_method,
            if p.required_args.is_empty() {
                "none".to_string()
            } else {
                p.required_args.join(", ")
            }
        ));
        if !p.default_headers.is_empty() {
            let hdr_keys = p
                .default_headers
                .iter()
                .map(|(k, _)| *k)
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("   Headers:     {}\n", hdr_keys));
        }
        if let Some(body) = p.default_body {
            out.push_str(&format!("   Body:        {}\n", body));
        }
        if let Some(reg) = p.default_success_regex {
            out.push_str(&format!("   Success Reg: {}\n", reg));
        }
        out.push('\n');
    }
    out.push_str(
        "Use 'rdns --provider <NAME>' to view complete details and YAML configuration examples.\n",
    );
    out.push_str(
        "For Cloudflare, run 'python scripts/cf_lookup.py -d <DOMAIN>' to query Zone and Record IDs.\n",
    );
    out
}

/// Format detailed information and ready-to-use YAML example for a specific provider.
pub fn format_provider_detail(name: &str) -> Result<String, String> {
    let p = get_provider(name).ok_or_else(|| {
        format!(
            "Error: Unknown provider '{}'.\nAvailable providers: {}\nUse 'rdns --list-providers' to see all available providers.",
            name,
            supported_providers_str()
        )
    })?;

    let mut out = String::new();
    out.push_str(&format!("Provider:     {}\n", p.name));
    if !p.aliases.is_empty() {
        out.push_str(&format!("Aliases:      {}\n", p.aliases.join(", ")));
    }
    out.push_str(&format!("Description:  {}\n", p.description));
    out.push_str(&format!("Website:      {}\n", p.website));
    out.push_str(&format!(
        "Requires Domain: {}\n",
        if p.requires_domain { "Yes" } else { "No" }
    ));
    out.push_str(&format!(
        "Required Args:   {}\n",
        if p.required_args.is_empty() {
            "none".to_string()
        } else {
            p.required_args.join(", ")
        }
    ));
    if !p.optional_args.is_empty() {
        out.push_str(&format!(
            "Optional Args:   {}\n",
            p.optional_args.join(", ")
        ));
    }

    out.push_str("\nDefault Request Template:\n");
    out.push_str(&format!("  Method: {}\n", p.default_method));
    out.push_str(&format!("  URL:    {}\n", p.default_url));
    if !p.default_headers.is_empty() {
        out.push_str("  Headers:\n");
        for (k, v) in p.default_headers {
            out.push_str(&format!("    {}: {}\n", k, v));
        }
    }
    if let Some(body) = p.default_body {
        out.push_str(&format!("  Body:   {}\n", body));
    }
    if let Some(reg) = p.default_success_regex {
        out.push_str(&format!("  Success Regex:    {}\n", reg));
    }
    if !p.default_success_contains.is_empty() {
        out.push_str(&format!(
            "  Success Contains: {:?}\n",
            p.default_success_contains
        ));
    }

    out.push_str("\nExample Task Configuration:\n");
    out.push_str("tasks:\n");
    out.push_str(p.example_yaml);
    out.push('\n');

    if p.name.starts_with("cloudflare") {
        out.push_str("\nCloudflare Helper Tool:\n");
        out.push_str("  Run 'python scripts/cf_lookup.py -d <DOMAIN>' to automatically query Zone ID and Record IDs.\n");
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_lookup_and_defaults() {
        let dynu = get_provider("dynu").expect("dynu must exist");
        assert_eq!(dynu.name, "dynu");
        assert!(dynu.requires_domain);
        assert_eq!(dynu.required_args, &["password"]);

        let req = dynu.default_request();
        assert_eq!(req.method(), "GET");
        assert!(req.url().contains("api.dynu.com"));
        assert!(req.url().contains("{{domain}}"));
        assert!(req.url().contains("{{password}}"));
        assert_eq!(req.success_regex.as_deref(), Some("^(good|nochg)"));

        // Case insensitivity
        assert!(get_provider("DYNU").is_some());
        assert!(get_provider("DynV6").is_some());
        assert!(get_provider("duckdns").is_some());

        // Alias lookup
        let he = get_provider("hurricane-electric").expect("he alias must resolve");
        assert_eq!(he.name, "he");

        // Unknown
        assert!(get_provider("non_existent_provider_xyz").is_none());
    }

    #[test]
    fn test_cloudflare_dualstack_provider() {
        let cf = get_provider("cloudflare").expect("cloudflare dual-stack must exist");
        assert_eq!(cf.name, "cloudflare");
        assert!(cf.aliases.contains(&"cf"));
        assert!(cf.aliases.contains(&"cloudflare-dualstack"));
        assert!(cf.requires_domain);
        assert_eq!(
            cf.required_args,
            &["token", "zone_id", "record_id_v4", "record_id_v6"]
        );
        assert_eq!(cf.default_method, "POST");

        let req = cf.default_request();
        assert_eq!(req.method(), "POST");
        assert!(req.url().contains("api.cloudflare.com"));
        assert!(req.url().contains("/zones/{{zone_id}}/dns_records/batch"));
        assert_eq!(
            req.headers.get("Content-Type").map(|s| s.as_str()),
            Some("application/json")
        );
        assert_eq!(
            req.headers.get("Authorization").map(|s| s.as_str()),
            Some("Bearer {{token}}")
        );
        assert!(req.body.as_deref().unwrap().contains("record_id_v4"));
        assert!(req.body.as_deref().unwrap().contains("record_id_v6"));
        assert!(req.body.as_deref().unwrap().contains("{{ipv4}}"));
        assert!(req.body.as_deref().unwrap().contains("{{ipv6}}"));

        // Alias lookup
        assert!(get_provider("CF").is_some());
        assert!(get_provider("cloudflare-dualstack").is_some());
    }

    #[test]
    fn test_cloudflare_v4_and_v6_providers() {
        let cf_v4 = get_provider("cloudflare-v4").expect("cloudflare-v4 must exist");
        assert_eq!(cf_v4.name, "cloudflare-v4");
        assert!(cf_v4.aliases.contains(&"cf-v4"));
        assert_eq!(cf_v4.required_args, &["token", "zone_id", "record_id"]);
        assert_eq!(cf_v4.default_method, "PATCH");

        let req4 = cf_v4.default_request();
        assert_eq!(req4.method(), "PATCH");
        assert_eq!(req4.body.as_deref(), Some(r#"{"content":"{{ipv4}}"}"#));

        let cf_v6 = get_provider("cloudflare-v6").expect("cloudflare-v6 must exist");
        assert_eq!(cf_v6.name, "cloudflare-v6");
        assert!(cf_v6.aliases.contains(&"cf-v6"));
        assert_eq!(cf_v6.required_args, &["token", "zone_id", "record_id"]);
        assert_eq!(cf_v6.default_method, "PATCH");

        let req6 = cf_v6.default_request();
        assert_eq!(req6.method(), "PATCH");
        assert_eq!(req6.body.as_deref(), Some(r#"{"content":"{{ipv6}}"}"#));
    }

    #[test]
    fn test_formatting() {
        let list_str = format_providers_list();
        assert!(list_str.contains("dynu"));
        assert!(list_str.contains("dynv6"));
        assert!(list_str.contains("duckdns"));
        assert!(list_str.contains("cloudflare"));
        assert!(list_str.contains("cloudflare-v4"));
        assert!(list_str.contains("cloudflare-v6"));

        let detail = format_provider_detail("dynu").expect("detail formatting succeeds");
        assert!(detail.contains("Provider:     dynu"));
        assert!(detail.contains("Example Task Configuration:"));

        let cf_detail =
            format_provider_detail("cloudflare").expect("cf detail formatting succeeds");
        assert!(cf_detail.contains("Provider:     cloudflare"));
        assert!(cf_detail.contains("Authorization: Bearer {{token}}"));
        assert!(cf_detail.contains("record_id_v4"));
        assert!(cf_detail.contains("cf_lookup.py"));

        let err = format_provider_detail("invalid_xyz").unwrap_err();
        assert!(err.contains("Unknown provider"));
    }
}
