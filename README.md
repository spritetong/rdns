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
  - Includes ready-to-use templates for `cloudflare` (dual-stack), `cloudflare-v4`, `cloudflare-v6`, `dynu-ipv4`, `dynu-ipv6`, `duckdns`, and easily extensible for any HTTP-based DDNS provider.

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
      api_token: "${CF_API_TOKEN}" # Supports environment variable expansion
      zone_id: "your_zone_id"
      v4_record_id: "v4_record_id"
      v6_record_id: "v6_record_id"
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
