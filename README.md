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

- 🌐 **Decoupled Interface Pools & Unified Dual-Stack**:
  - Network egress profiles (`interfaces`) support unified dual-stack `ip:` resolution (concurrently detecting IPv4 and IPv6) with auto-adapting remote parsers and higher-priority `ipv4:` / `ipv6:` overrides.
  - An interface performs periodic IP detection once, and all bound tasks reuse the resolved result ("probe once, fan out to many").

- 🔄 **Native Dual-Stack & Single-Request Atomic Updates**:
  - Concurrently queries and validates both IPv4 and IPv6 addresses.
  - Injects both IPs into a single HTTP request (e.g., Cloudflare batch API or custom webhooks).

- 🧩 **Zero Hardcoded Provider SDKs & Adaptive Template Engine**:
  - Fully driven by declarative HTTP request templates (URL, Headers, Body, Method).
  - Supports variable substitution (`{{ipv4}}`, `{{ipv6}}`, `{{domain}}`, `{{args.param}}`) alongside **Conditional Replacement Patterns** (`{{?var:THEN|ELSE}}`) and quote-aware JSON comma cleaning.
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

Copy `config-example.yaml` to `config.yaml` and adjust it to your environment:

```bash
cp config-example.yaml config.yaml
```

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
    # Unified dual-stack IP resolution (simultaneously detects IPv4 and IPv6)
    ip:
      source: "remote"          # "remote" or "interface"
      urls:
        - "https://api64.ipify.org"
        - "https://icanhazip.com"
        - "https://ident.me"
    # Optional overrides: ipv4 / ipv6 have higher priority and override ip settings
    # ipv6:
    #   source: "interface"
    #   interface: "eth0"         # Or Windows friendly name like "以太网"
    #   prefer_slaac: true        # Prioritize RFC 4291 EUI-64 SLAAC address

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
| `{{?var:THEN}}` | **Conditional Pattern**: renders `THEN` (with `{}` replaced by `var`) if `var` is active, else omitted | `{{?ipv4:&ip={}}}` $\to$ `&ip=1.2.3.4` or `""` |
| `{{?var:THEN\|ELSE}}` | **Conditional with Else**: renders `THEN` if `var` is active, else renders `ELSE` | `{{?ipv4:&myip={}\|&myip=no}}` $\to$ `&myip=1.2.3.4` or `&myip=no` |

> [!TIP]
> **Interface Configuration & Protocol Overrides**:
>
> - **Unified `ip:` option**: Automatically detects both IPv4 and IPv6 using the same source (`remote` or `interface`). Remote probes auto-adapt to plain text, JSON (`{"ip": ...}`), or quoted responses.
> - **Higher-Priority `ipv4:` / `ipv6:` overrides**: Define dedicated `ipv4:` or `ipv6:` to override `ip:` settings (or set `enabled: false` to disable a protocol entirely for this interface).
> - **Task-level protocol suppression via `args`**:
>   - Set `ipv4: null` (or `ipv4: ~`) in the task's `args:` to suppress IPv4 and make it an IPv6-only task.
>   - Set `ipv6: null` (or `ipv6: ~`) in `args:` to suppress IPv6 and make it an IPv4-only task.
>   - Specify a static IP string (e.g. `ipv4: "1.2.3.4"`) in `args:` to override interface resolution for that task.
> - If an interface only defines `ipv4:` or `ipv6:`, tasks bound to it automatically run in IPv4-only or IPv6-only mode, and conditional patterns (`{{?ipv4:...}}` / `{{?ipv6:...}}`) adapt accordingly without any manual configuration.

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

#### Adaptive Query Parameter Webhook (Conditional Dual / Single Stack)

```yaml
tasks:
  - name: "custom-adaptive-ddns"
    interface: "Local"
    domain: "myserver.dynamic-dns.org"
    request:
      method: "GET"
      # Automatically includes &ip= or &ipv6= only when that protocol is active on the interface/task
      url: "https://ddns.example.org/nic/update?hostname={{domain}}&key=${DDNS_KEY}{{?ipv4:&ip={}}}{{?ipv6:&ipv6={}}}"
      success_contains:
        - "good"
        - "nochg"
```

---

### 3. Built-in Provider Presets

`rdns` ships with predefined templates for common DDNS configurations. All major providers (`cloudflare`, `dynu`, `dynv6`, `duckdns`, and `he`) are **adaptive**: they automatically support dual-stack, IPv4-only, or IPv6-only environments through conditional template replacement and JSON comma cleaning.

| Provider Name | Stack | Description | Required `args` |
| :--- | :--- | :--- | :--- |
| `cloudflare` | Adaptive (Dual/v4/v6) | Cloudflare DNS API v4 (adaptive batch update) | `token`, `zone_id` (`record_id_v4` if v4 active, `record_id_v6` if v6 active) |
| `dynu` | Adaptive (Dual/v4/v6) | Dynu Systems DDNS API (auto adapts; fallback `&myipv6=no`) | `password` (optional: `username`) |
| `dynv6` | Adaptive (Dual/v4/v6) | dynv6 Free Dynamic DNS API (auto adapts) | `token` |
| `duckdns` | Adaptive (Dual/v4/v6) | DuckDNS API (auto adapts) | `token` |
| `he` | Adaptive (Dual/v4/v6) | Hurricane Electric Dynamic DNS (auto adapts) | `password` |
| `noip` | IPv4 | No-IP Dynamic Update Client API | `username`, `password` |

#### Cloudflare

> [!TIP]
> **Cloudflare Helper Script**: If you don't know your `zone_id` or `record_id`, use the helper script [`scripts/cf_lookup.py`](scripts/cf_lookup.py) (uses Python standard library only) to automatically discover your Zone ID and DNS Record IDs, and generate ready-to-use YAML task configurations:
>
> ```bash
> python scripts/cf_lookup.py -d sub.example.com
> ```
>
> *(Interactive prompt for your API token if omitted or not exported in `CF_API_TOKEN`)*

The `cloudflare` provider uses the atomic batch update API (`/dns_records/batch`). Thanks to conditional template rendering and JSON normalizer, a single `cloudflare` provider configuration automatically handles:

- **Dual-Stack**: Updates both A and AAAA records in a single atomic request.
- **IPv4-Only**: Updates only the A record (e.g. when the interface only has IPv4 or `ipv6: null` is set).
- **IPv6-Only**: Updates only the AAAA record (e.g. when the interface only has IPv6 or `ipv4: null` is set).

```yaml
tasks:
  # Dual-stack atomic batch update
  - name: "cf-dualstack"
    interface: "Local"
    domain: "sub.example.com"
    provider: "cloudflare" # Aliases: "cf", "cf-dualstack", "cloudflare-dualstack"
    args:
      token: "${CF_API_TOKEN}"
      zone_id: "${CF_ZONE_ID}"
      record_id_v4: "${CF_RECORD_ID_V4}"
      record_id_v6: "${CF_RECORD_ID_V6}"

  # IPv4-only update on a dual-stack interface
  - name: "cf-v4-only"
    interface: "Local"
    domain: "ipv4.example.com"
    provider: "cloudflare"
    args:
      token: "${CF_API_TOKEN}"
      zone_id: "${CF_ZONE_ID}"
      record_id_v4: "${CF_RECORD_ID_V4}"
      ipv6: null # Suppresses IPv6 for this task

  # IPv6-only update on a dual-stack interface
  - name: "cf-v6-only"
    interface: "Local"
    domain: "ipv6.example.com"
    provider: "cloudflare"
    args:
      token: "${CF_API_TOKEN}"
      zone_id: "${CF_ZONE_ID}"
      record_id_v6: "${CF_RECORD_ID_V6}"
      ipv4: null # Suppresses IPv4 for this task
```

#### Dynu Systems

Automatically adapts to dual-stack, IPv4-only, or IPv6-only:

```yaml
tasks:
  - name: "dynu-adaptive"
    interface: "Local"
    domain: "yourname.freeddns.org"
    provider: "dynu"
    args:
      password: "${DYNU_PASSWORD}"
      # username: "optional_username"
      # ipv4: null # Optional: suppress IPv4 for IPv6-only
      # ipv6: null # Optional: suppress IPv6 for IPv4-only
```

#### dynv6

Automatically adapts to dual-stack, IPv4-only, or IPv6-only:

```yaml
tasks:
  - name: "dynv6-adaptive"
    interface: "Local"
    domain: "yourname.dynv6.net"
    provider: "dynv6"
    args:
      token: "${DYNV6_TOKEN}"
      # ipv4: null # Optional: suppress IPv4 for IPv6-only
      # ipv6: null # Optional: suppress IPv6 for IPv4-only
```

#### DuckDNS

Automatically adapts to dual-stack, IPv4-only, or IPv6-only:

```yaml
tasks:
  - name: "duckdns-adaptive"
    interface: "Local"
    domain: "yoursubdomain" # Subdomain prefix without .duckdns.org
    provider: "duckdns"
    args:
      token: "${DUCKDNS_TOKEN}"
      # ipv4: null # Optional: suppress IPv4 for IPv6-only
      # ipv6: null # Optional: suppress IPv6 for IPv4-only
```

#### Hurricane Electric (HE)

Automatically adapts to dual-stack, IPv4-only, or IPv6-only:

```yaml
tasks:
  - name: "he-adaptive"
    interface: "Local"
    domain: "yourname.dyn.he.net"
    provider: "he" # Alias: "hurricane-electric"
    args:
      password: "${HE_PASSWORD}"
      # ipv4: null # Optional: suppress IPv4 for IPv6-only
      # ipv6: null # Optional: suppress IPv6 for IPv4-only
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
  -c, --config <CONFIG>
          Path to YAML configuration file [default: config.yaml]
      --once
          Single execution mode: update once and immediately exit
      --daemon
          Long-running daemon mode with asynchronous interval polling
      --dry-run
          Dry-run mode: fetch IP and render templates without sending live HTTP write requests
      --check
          Validate configuration syntax and interface existence without running
      --list-providers
          List all predefined DDNS providers and their templates
      --show-provider <NAME>
          Query and show default request template and details for a predefined provider (for Cloudflare, see also 'python scripts/cf_lookup.py') [alias: --provider]
  -t, --worker-threads <THREADS>
          Number of Tokio runtime worker threads (1 for single-thread lightweight runtime)
  -s, --state <PATH>
          Override the path to the state persistence JSON file
      --write-state [<BOOL>]
          Control whether to write the state persistence file to disk (true/false) [alias: --save-state]
      --no-state
          Disable writing the state persistence file to disk [aliases: --no-state-file, --no-save-state]
      --no-netwatcher
          Disable netwatcher event-driven network change detection (fallback to timer polling)
  -l, --log-level <LEVEL>
          Log level filter (trace, debug, info, warn, error, off) [default: info]
  -h, --help
          Print help
  -V, --version
          Print version

Cloudflare Helper:
  Use 'python scripts/cf_lookup.py -d <DOMAIN>' to query Zone ID and Record IDs from Cloudflare.
```

---

## 📄 License

This project is licensed under the **GNU General Public License v3.0** (GPL-3.0-or-later). See the [LICENSE](LICENSE) file for details.

**Author**: Sprite Tong ([spritetong@gmail.com](mailto:spritetong@gmail.com))
