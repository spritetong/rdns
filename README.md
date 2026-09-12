# RDNS (Rust Dynamic DNS)

A lightweight, robust, event-driven Dynamic DNS (DDNS) daemon written in Rust.

`rdns` replaces rigid, vendor-specific SDK abstractions with a unified, high-performance HTTP template engine. It natively supports atomic dual-stack (IPv4 + IPv6) updates in a single request, intelligent network event watching, and RFC 4291 EUI-64 SLAAC identification.

---

## ✨ Features

- ⚡ **Event-Driven Change Detection (`netwatcher`)**:
  - Listens directly to OS-level network interface changes.
  - Eliminates wasteful empty polling while keeping a reliable fallback timer.
  - Built-in debounce coalescing and virtual adapter filtering (`vEthernet`, WSL, Docker, TAP/TUN) with whitelist support for user-configured adapters.
  - Modern Standby / Sleep-wake protection with DNS readiness probes.

- 🌐 **Decoupled Interface Pools**:
  - Network egress profiles (`interfaces`) are abstracted from update tasks.
  - An interface performs periodic IP detection once, and all bound tasks reuse the resolved result ("probe once, fan out to many").

- 🔄 **Native Dual-Stack & Single-Request Atomic Updates**:
  - Concurrently queries and validates both IPv4 and IPv6 addresses.
  - Injects both IPs into a single HTTP request (e.g., Cloudflare batch API or custom webhooks).

- 🧩 **Zero Hardcoded Provider SDKs**:
  - Fully driven by declarative HTTP request templates (URL, Headers, Body, Method).
  - Supports GET, POST, PUT, PATCH with Mustache-style variable templating (`{{ipv4}}`, `{{ipv6}}`, `{{domain}}`, `{{args.param}}`).
  - Declarative response assertions via HTTP status codes and regex patterns (e.g. `good|nochg`).

- 📦 **Built-in Provider Presets**:
  - Includes ready-to-use templates for `cloudflare`, `dynu`, `dynv6`, `duckdns`, Hurricane Electric (`he`), and `noip` across single-stack and atomic dual-stack configurations, plus seamless raw HTTP webhook extensibility.

- 🔍 **RFC 4291 EUI-64 SLAAC Recognition**:
  - Automatically identifies and prioritizes stable hardware-derived SLAAC IPv6 addresses over temporary privacy (RFC 4941) or transient DHCPv6 addresses.

- 🛡️ **DNS Drift Detection & Active Reconciliation**:
  - Queries authoritative/public DNS servers via asynchronous `hickory-resolver`.
  - Actively detects discrepancies between local IP and registered cloud DNS records, initiating self-healing updates.

- ⏱️ **Adaptive Dual-Speed Scheduling**:
  - Operates at normal interval (default `300s`) during steady state.
  - Automatically enters shortened `retry_interval` (default `60s`) when update fails and DNS records mismatch.

- 💾 **Atomic Local State Persistence**:
  - Non-blocking state cache using atomic write-and-rename (`state.json`).
  - Prevents redundant write requests across daemon restarts.

- 📢 **Webhook Notifications**:
  - Supports customizable HTTP alerts on IP changes (`on_change`) and task failures (`on_failure`).

- 🔒 **Native & Custom Root Certificates**:
  - Automatically loads host platform native root CA certificates into `rustls` (via `rustls-native-certs`).
  - Supports loading extra custom CA certificate bundles or private/corporate roots (`cacerts`) globally or per-task.

- 🪶 **Embedded & Server Friendly**:
  - Configurable Tokio worker threads (`worker_threads: 1` for memory-constrained embedded routers, multi-threaded for servers).
  - Clean, standardized OpenWrt DDNS style operational logs.

---

## 🚀 Quick Start

### Build from Source

Ensure you have Rust (edition 2024 / Rust 1.85+) installed:

```bash
git clone https://github.com/spritetong/rdns.git
cd rdns
cargo build --release
```

The optimized binary will be placed in `target/release/rdns` (or `rdns.exe` on Windows).

### Basic Usage

```bash
# Verify configuration syntax and parameter validity
rdns --check -c config.yaml

# Perform a dry-run preview (generates templates without sending HTTP writes)
rdns --dry-run -c config.yaml

# Run a single update cycle and exit
rdns --once -c config.yaml

# Run as a continuous daemon
rdns -c config.yaml

# Disable netwatcher event listening (pure polling mode)
rdns --no-netwatcher -c config.yaml
```

---

## ⚙️ Configuration (`config.yaml`)

```yaml
global:
  interval: 300                # Default polling interval in seconds
  retry_interval: 60           # Fast recovery retry interval in seconds
  timeout: 10                  # HTTP request timeout
  shutdown_timeout: 10         # Maximum graceful shutdown drain timeout
  worker_threads: 2            # Tokio worker threads (1 for embedded routers)
  log_level: "info"            # trace | debug | info | warn | error | off
  # cacerts: "/path/to/extra-ca-certificates.pem" # Optional custom CA bundle (PEM or DER)
  netwatcher: true             # Enable event-driven network change detection
  netwatcher_debounce_ms: 2000 # Debounce window for network events (ms)

# Generic notification webhooks (optional)
notification:
  on_change:
    enabled: true
    method: "POST"
    url: "https://api.example.com/notify"
    headers:
      Content-Type: "application/json"
    body: |
      {
        "event": "ip_changed",
        "task": "{{task_name}}",
        "old_ip": "{{old_ip}}",
        "new_ip": "{{new_ip}}"
      }

# Network egress interfaces
interfaces:
  - name: "Local"
    interval: 300
    dns_server: "8.8.8.8"
    ipv4:
      enabled: true
      source: "remote"
      urls:
        - "https://api.ipify.org"
        - "https://v4.ident.me"
    ipv6:
      enabled: true
      source: "interface"
      interface: "eth0"         # Or Windows friendly name like "以太网"
      prefer_slaac: true        # Prioritize RFC 4291 EUI-64 SLAAC address

# DDNS Tasks
tasks:
  - name: "cloudflare-ddns"
    interface: "Local"
    domain: "example.com"
    provider: "cloudflare"      # Built-in dual-stack Cloudflare preset
    args:
      token: "${CF_API_TOKEN}"  # Supports environment variable expansion
      zone_id: "<your_zone_id>"
      record_id_v4: "<record_id_v4>"
      record_id_v6: "<record_id_v6>"
```

---

## 📋 Task Configuration & Provider Templates

`rdns` tasks can be configured using either **built-in provider presets** or **custom raw HTTP webhooks**.

### 1. Template Variables Reference

When `rdns` executes an update, placeholders in URLs, headers, and request bodies are substituted with current runtime values:

| Variable | Description | Example Output |
| :--- | :--- | :--- |
| `{{ipv4}}` | Resolved public IPv4 address | `203.0.113.195` |
| `{{ipv6}}` | Resolved public IPv6 address | `2001:db8::8a2e:370:7334` |
| `{{domain}}` | Configured task domain | `sub.example.com` |
| `{{task_name}}` | Name of the task | `cloudflare-ddns` |
| `{{timestamp}}` | Current Unix epoch timestamp (seconds) | `1741780000` |
| `{{args.KEY}}` or `{{KEY}}` | Custom parameters defined under `args` | `{{token}}`, `{{zone_id}}` |
| `${ENV_VAR}` | Environment variable (interpolated on config load) | `${CF_API_TOKEN}` |

---

### 2. Custom Raw HTTP Webhook Tasks

If your DNS provider is not among the built-in presets, or if you want to notify a custom API, internal microservice, or router webhook, omit `provider` and define a custom `request` block:

#### JSON POST Dual-Stack Webhook

```yaml
tasks:
  - name: "custom-dualstack-webhook"
    interface: "Local"
    domain: "home.example.com"
    request:
      method: "POST"
      url: "https://api.custom-dns.com/v1/update"
      headers:
        Authorization: "Bearer ${DNS_API_KEY}"
        Content-Type: "application/json"
      body: |
        {
          "domain": "{{domain}}",
          "ipv4": "{{ipv4}}",
          "ipv6": "{{ipv6}}",
          "updated_at": {{timestamp}}
        }
      # Assertions: regex match or required substrings in response
      success_regex: '"status"\s*:\s*"success"'
      success_contains:
        - "success"
      # Optional network and TLS settings
      proxy: "http://127.0.0.1:7890"             # Optional HTTP or SOCKS5 proxy
      tls_insecure: false                       # Set true to bypass TLS certificate verification
      cacerts: "/etc/ssl/certs/internal-ca.pem"  # Task-specific custom CA certificates
```

#### Simple GET Query Parameter Hook

```yaml
tasks:
  - name: "custom-get-ddns"
    interface: "Local"
    domain: "myserver.dynamic-dns.org"
    request:
      method: "GET"
      url: "https://ddns.example.org/nic/update?hostname={{domain}}&myip={{ipv4}}&key=${DDNS_KEY}"
      success_contains:
        - "good"
        - "nochg"
```

---

### 3. Built-in Provider Presets

`rdns` ships with predefined templates for 14 common DDNS configurations. When using a `provider`, standard URLs, HTTP methods, headers, request bodies, and success assertions are pre-configured, requiring only credentials and IDs in `args`.

| Provider Name | Stack | Description | Required `args` |
| :--- | :--- | :--- | :--- |
| `cloudflare` | Dual-Stack | Cloudflare DNS API v4 (batch A & AAAA update) | `token`, `zone_id`, `record_id_v4`, `record_id_v6` |
| `cloudflare-v4` | IPv4 | Cloudflare DNS API v4 (A record PATCH) | `token`, `zone_id`, `record_id` |
| `cloudflare-v6` | IPv6 | Cloudflare DNS API v4 (AAAA record PATCH) | `token`, `zone_id`, `record_id` |
| `dynu` | Dual-Stack | Dynu Systems DDNS API | `password` (optional: `username`) |
| `dynu-ipv4` | IPv4 | Dynu Systems DDNS API | `password` (optional: `username`) |
| `dynu-ipv6` | IPv6 | Dynu Systems DDNS API | `password` (optional: `username`) |
| `dynv6` | Dual-Stack | dynv6 Free Dynamic DNS API | `token` |
| `dynv6-ipv4` | IPv4 | dynv6 Free Dynamic DNS API | `token` |
| `dynv6-ipv6` | IPv6 | dynv6 Free Dynamic DNS API | `token` |
| `duckdns` | Dual-Stack | DuckDNS API | `token` |
| `duckdns-ipv4` | IPv4 | DuckDNS API | `token` |
| `duckdns-ipv6` | IPv6 | DuckDNS API | `token` |
| `he` | IPv4 | Hurricane Electric Dynamic DNS | `password` |
| `noip` | IPv4 | No-IP Dynamic Update Client API | `username`, `password` |

#### Cloudflare

Dual-stack atomic update (single batch API call updating both A and AAAA records):

```yaml
tasks:
  - name: "cf-dualstack"
    interface: "Local"
    domain: "sub.example.com"
    provider: "cloudflare" # Aliases: "cf", "cf-dualstack", "cloudflare-dualstack"
    args:
      token: "${CF_API_TOKEN}"
      zone_id: "${CF_ZONE_ID}"
      record_id_v4: "${CF_RECORD_ID_V4}"
      record_id_v6: "${CF_RECORD_ID_V6}"
```

Single-stack IPv4 or IPv6 updates:

```yaml
tasks:
  - name: "cf-v4"
    interface: "Local"
    domain: "ipv4.example.com"
    provider: "cloudflare-v4" # Aliases: "cf-v4", "cloudflare-ipv4"
    args:
      token: "${CF_API_TOKEN}"
      zone_id: "${CF_ZONE_ID}"
      record_id: "${CF_RECORD_ID_V4}"

  - name: "cf-v6"
    interface: "Local"
    domain: "ipv6.example.com"
    provider: "cloudflare-v6" # Aliases: "cf-v6", "cloudflare-ipv6"
    args:
      token: "${CF_API_TOKEN}"
      zone_id: "${CF_ZONE_ID}"
      record_id: "${CF_RECORD_ID_V6}"
```

#### Dynu Systems

```yaml
tasks:
  # Dual-Stack (IPv4 & IPv6)
  - name: "dynu-dual"
    interface: "Local"
    domain: "yourname.freeddns.org"
    provider: "dynu"
    args:
      password: "${DYNU_PASSWORD}"
      # username: "optional_username"

  # IPv4 Only
  - name: "dynu-v4"
    interface: "Local"
    domain: "yourname.freeddns.org"
    provider: "dynu-ipv4"
    args:
      password: "${DYNU_PASSWORD}"

  # IPv6 Only
  - name: "dynu-v6"
    interface: "Local"
    domain: "yourname.freeddns.org"
    provider: "dynu-ipv6"
    args:
      password: "${DYNU_PASSWORD}"
```

#### dynv6

```yaml
tasks:
  # Dual-Stack (IPv4 & IPv6)
  - name: "dynv6-dual"
    interface: "Local"
    domain: "yourname.dynv6.net"
    provider: "dynv6"
    args:
      token: "${DYNV6_TOKEN}"

  # IPv4 Only
  - name: "dynv6-v4"
    interface: "Local"
    domain: "yourname.dynv6.net"
    provider: "dynv6-ipv4"
    args:
      token: "${DYNV6_TOKEN}"

  # IPv6 Only
  - name: "dynv6-v6"
    interface: "Local"
    domain: "yourname.dynv6.net"
    provider: "dynv6-ipv6"
    args:
      token: "${DYNV6_TOKEN}"
```

#### DuckDNS

```yaml
tasks:
  # Dual-Stack (IPv4 & IPv6)
  - name: "duckdns-dual"
    interface: "Local"
    domain: "yoursubdomain" # Subdomain prefix without .duckdns.org
    provider: "duckdns"
    args:
      token: "${DUCKDNS_TOKEN}"

  # IPv4 Only
  - name: "duckdns-v4"
    interface: "Local"
    domain: "yoursubdomain"
    provider: "duckdns-ipv4"
    args:
      token: "${DUCKDNS_TOKEN}"

  # IPv6 Only
  - name: "duckdns-v6"
    interface: "Local"
    domain: "yoursubdomain"
    provider: "duckdns-ipv6"
    args:
      token: "${DUCKDNS_TOKEN}"
```

#### Hurricane Electric (HE)

```yaml
tasks:
  - name: "he-v4"
    interface: "Local"
    domain: "yourname.dyn.he.net"
    provider: "he" # Alias: "hurricane-electric"
    args:
      password: "${HE_PASSWORD}"
```

#### No-IP

```yaml
tasks:
  - name: "noip-v4"
    interface: "Local"
    domain: "yourname.ddns.net"
    provider: "noip"
    args:
      username: "${NOIP_USERNAME}"
      password: "${NOIP_PASSWORD}"
```

---

### 4. Overriding Provider Defaults

You can combine a built-in `provider` with a `request` block to selectively customize proxy settings, TLS options, headers, or extra CA certificates without having to rewrite the URL or payload:

```yaml
tasks:
  - name: "cf-with-proxy"
    interface: "Local"
    domain: "sub.example.com"
    provider: "cloudflare"
    args:
      token: "${CF_API_TOKEN}"
      zone_id: "${CF_ZONE_ID}"
      record_id_v4: "${CF_RECORD_ID_V4}"
      record_id_v6: "${CF_RECORD_ID_V6}"
    request:
      proxy: "socks5://127.0.0.1:1080"             # Route updates through a SOCKS5 proxy
      cacerts: "/etc/ssl/certs/internal-ca.pem"    # Extra custom CA certificates
      # headers:
      #   X-Custom-Header: "CustomValue"
```

---

## 🛠️ CLI Reference

```text
Usage: rdns [OPTIONS]

Options:
  -c, --config <CONFIG>          Path to YAML configuration file [default: config.yaml]
      --check                    Validate configuration syntax and parameters then exit
      --dry-run                  Render templates and preview requests without sending
      --once                     Execute a single update cycle then exit
      --no-state                 Disable writing state file to disk
      --no-netwatcher            Disable event-driven network change monitoring
  -w, --worker-threads <THREADS> Number of Tokio worker threads (overrides config)
  -l, --log-level <LOG_LEVEL>    Log level filter [possible values: trace, debug, info, warn, error, off]
  -h, --help                     Print help
  -V, --version                  Print version
```

---

## 📄 License

This project is licensed under the **GNU General Public License v3.0** (GPL-3.0-or-later). See the [LICENSE](LICENSE) file for details.

**Author**: Sprite Tong ([spritetong@gmail.com](mailto:spritetong@gmail.com))
