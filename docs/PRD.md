# RDNS - Product Requirements Document (PRD)

This specification defines **RDNS**, a universal, lightweight, Webhook-driven Dynamic DNS (DDNS) client written in **Rust**. By abandoning rigid vendor-specific REST API SDK abstractions (such as those in `godns`) in favor of customizable HTTP request templates, RDNS fundamentally solves the challenges of "atomic dual-stack updates in a single request" and "multi-instance/multi-provider configuration conflicts."

---

### 1. Core Design Objectives

* **Zero Hardcoded Providers**: No dedicated vendor SDKs (e.g., Cloudflare, Dynu, AliDNS) are embedded. All DNS updates are abstracted through a unified **HTTP Request Engine**.
* **Decoupled Named Interface Pools**: Network egress paths are abstracted into independent `interfaces` pools. Each interface periodically executes a single IP probe, and all Webhook tasks bound to that interface share the result—eliminating redundant network queries across tasks.
* **Native Dual-Stack & Atomic Single-Request Updates**: A single task can concurrently monitor local IPv4 and IPv6 addresses and populate both into a single HTTP request payload.
* **Declarative Response Assertions**: Supports validating responses via HTTP status codes and regex patterns (e.g., `good|nochg`), preventing false positives where services return HTTP 200 despite business-level failures.
* **Intelligent RFC 4291 EUI-64 SLAAC Identification**: In dual-stack ISP environments where both DHCPv6 and SLAAC public IPv6 addresses are assigned, RDNS automatically recognizes and prioritizes the hardware MAC-derived, long-term stable SLAAC address.
* **Remote DNS Verification & Drift Reconciliation**: Integrates asynchronous `hickory-resolver` queries against public or custom DNS servers, actively detecting and reconciling cloud record drift (e.g., records manually altered via web consoles).
* **Adaptive Dual-Speed Polling**: **Shortens polling interval to `retry_interval` (default 60s) if and only if an update fails and remote DNS records mismatch**. This provides rapid self-healing while avoiding API rate-limiting storms during normal DNS TTL propagation delays.
* **Pure Static Delivery & Platform Adaptation**: Built on pure Rust `rustls` with platform-native root certificate trust, eliminating runtime OpenSSL shared-library dependencies. Scales seamlessly from ultra-lightweight single-threaded runtimes (embedded routers) to multi-threaded servers.

---

### 2. Core Functional Requirements

#### 2.1 IP Resolution & Interface Management (`ip`)

Network probing is centralized under the named `interfaces` configuration:

* **Named Interface Pools**:
  * Users define named egress interfaces (e.g., `Local`, `Tailscale`), polling intervals, and probe configurations (`ipv4`/`ipv6`).
  * In daemon mode, each `interface` runs an independent asynchronous coroutine for periodic IP resolution. All tasks referencing that interface reuse the resolved IP ("resolve once, fan out to many").
  * If a task omits the `interface` field, it automatically binds to the first configured interface.

* **Remote HTTP Probing**:
  * Supports probing via designated URLs with strict protocol-stack binding (`local_address` bound to `0.0.0.0` for IPv4 and `::` for IPv6).
  * Supports configuring multiple fallback URLs that are tried sequentially upon failure.

* **Local Interface Extraction & Address Sanitization**:
  * Directly enumerates physical/logical interfaces via `ifaddrsx`, matching interface names by regex or friendly names (e.g., Windows adapter names).
  * **IPv6 Sanitization**: Automatically filters out Link-Local (`fe80::/10`), Unique Local Addresses (ULA, `fc00::/7`), and RFC 4941 temporary privacy extension addresses.
  * **RFC 4291 EUI-64 SLAAC Priority (`prefer_slaac: true`)**: Recognizes the 64-bit interface identifier (`octets[11] == 0xff && octets[12] == 0xfe`) and prioritizes the stable physical MAC-mapped address over short-lived DHCPv6/temporary addresses.
  * **IPv4 Private IP Filtering**: Automatically filters out RFC 1918 private networks and RFC 6598 CGNAT (`100.64.0.0/10`) by default; can be allowed via `allow_private: true` for internal DNS setups.

#### 2.2 IP State Management, DNS Reconciliation & Adaptive Scheduling (`scheduler`, `persistence`)

* **In-Memory Cache & Idempotency**: Network requests are triggered only when a newly resolved IP differs from the last successfully updated IP. Supports `force_update_interval` for providers requiring periodic heartbeats (e.g., forced refresh every 24 hours).
* **Non-blocking State Persistence**: Persists the last successful IP state to local storage (e.g., `state.json`) using atomic temporary-file-and-rename semantics (`atomic_file.rs`). Avoids redundant requests after restarts to prevent provider rate-limiting.
* **Remote DNS Verification via `hickory-resolver`**:
  * Supports configuring upstream DNS servers globally or per-interface (`dns_server: "8.8.8.8"` or `"1.1.1.1:53"`). Defaults to host system DNS if unspecified.
  * When local IP has not changed, RDNS periodically queries remote A / AAAA records for the configured task `domain`.
  * If remote records mismatch the actual local IP, RDNS flags the discrepancy and triggers corrective updates immediately.
  * Features a 60-second cooldown period after successful updates to avoid redundant checks during initial DNS TTL propagation.
* **Adaptive Dual-Speed Polling**:
  * **Steady State**: Operates at `normal_interval` (default 300s) during normal operation to conserve bandwidth and API quotas.
  * **Reconciliation State**: **If and only if an update fails AND DNS records mismatch**, the subsequent poll sleep duration is shortened to `retry_interval` (default 60s) for rapid recovery.
  * **TTL Delay Protection**: Successful updates do **NOT** shorten intervals even if DNS queries still return stale records due to TTL propagation, preventing API ban storms.
  * **Automatic Convergence**: Once updates succeed and DNS records align, the scheduler smoothly returns to the 300s normal interval.

#### 2.3 Custom Request Engine, Task Arguments & Providers (`engine`, `provider`)

Abstracts every update into a parameterized HTTP request, complemented by built-in provider templates:

* **Unified Template Slots & Custom Parameters (`args`)**:
  * **Built-in System Slots**:
    * `{{ipv4}}`: Resolved public IPv4 address.
    * `{{ipv6}}`: Resolved public IPv6 address.
    * `{{domain}}`: Configured domain name.
    * `{{timestamp}}`: Current Unix timestamp (seconds/milliseconds).
  * **Custom Named Arguments (`args`)**:
    * Tasks support `args: HashMap<String, String>` for provider-specific parameters (e.g., `{{password}}`, `{{token}}`, `{{zone_id}}`).
    * URLs, Headers, and Request Bodies are processed through a single secure template engine (`src/engine/template.rs`).
* **Environment Variable Expansion**:
  * Config files support `${ENV_NAME}` and `${ENV_NAME:-default}` syntax, automatically expanded during YAML deserialization.
  * Sensitive tokens/passwords in `args` (e.g., `password: "${DYNU_PASSWORD}"`) can reference environment variables directly, keeping credentials out of version-controlled files.
* **Predefined Provider Templates (`provider`)**:
  * Tasks can specify `provider: "dynu"` (or `dynv6`, `duckdns`, `he`, `noip`, etc.).
  * **Automatic Template Assembly**: When `request` is omitted, standard HTTP templates, methods, URLs, and assertions (e.g., Dynu's `^(good|nochg)`) are auto-populated.
  * **Parameter Validation**: Verifies mandatory parameters on startup (e.g., `password` for Dynu, `token` for dynv6), failing fast with clear instructions if any are missing.
  * **Built-in Providers**: `dynu` (dual-stack), `dynu-ipv4`, `dynu-ipv6`, `dynv6` (dual-stack), `dynv6-ipv4`, `dynv6-ipv6`, `duckdns` (dual-stack), `duckdns-ipv4`, `duckdns-ipv6`, `he` (Hurricane Electric), `noip`, etc.
* **CLI Provider Introspection (`--list-providers` / `--provider`)**:
  * `rdns --list-providers`: Lists all built-in providers, websites, default templates, and required arguments.
  * `rdns --provider <NAME>` (alias `--show-provider <NAME>`): Displays full templates, argument explanations, and copy-pasteable YAML task examples. Works independently without requiring a config file.
* **Full HTTP Semantics**:
  * Methods: `GET`, `POST`, `PUT`, `PATCH`.
  * Headers: Custom headers (e.g., `Authorization: Bearer <token>`, `Content-Type: application/json`).
  * Body: Custom payloads (JSON, form URL-encoded text).
* **TLS Engine & Native Root Certificates**:
  * **Memory-Safe Protocol Stack**: Powered by `rustls`, avoiding dynamic OpenSSL dependencies for zero-dependency static binaries.
  * **Platform Native CA Stores**: Automatically loads OS root certificates (Windows Certificate Store, macOS Keychain, Linux `/etc/ssl/certs`).
  * **Insecure TLS Option**: Supports `tls_insecure: true` (default: `false`) for internal self-signed Webhooks, complete with security warnings.
* **Proxy Support**:
  * Configurable globally or per-task (`proxy: "http://127.0.0.1:7890"` or `socks5://127.0.0.1:1080`), automatically respecting environment variables (`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`).
* **Response Assertions**:
  * HTTP status code validation (default `200..=299`).
  * Response body validation: Supports `success_contains` (substring) or `success_regex` (regular expression), updating local state only upon successful verification.

#### 2.4 Task Scheduling, Runtime & Concurrency (`scheduler`, `lifecycle`)

* **Adaptive Tokio Worker Threads**:
  * Configurable via `global.worker_threads` in `config.yaml` or overridden via CLI `-t <N>` / `--worker-threads <N>`.
  * **True Single-Threaded Runtime**: Setting to `1` initializes a genuine `tokio::runtime::Builder::new_current_thread()` runtime, eliminating worker thread pools and thread switching for resource-constrained devices (OpenWrt / MIPS / 128MB RAM).
  * **Multi-Threaded Runtime**: Omitted values default to CPU core count for high-concurrency environments.
* **Task Isolation**: Tasks run with isolated timers, retry policies, and state tracking.
* **Single-Run Mode (`--once`)**: Executes all tasks once and exits with an appropriate status code (ideal for Cron or OpenWrt hotplug scripts).
* **Daemon Mode (`--daemon`)**: Long-running background daemon driven by asynchronous timers.
* **Configuration Hot-Reload**: Listens for `SIGHUP` on POSIX systems, re-validating and reloading `config.yaml` without stopping the process.

#### 2.5 Generic Notification System (`notification`)

* Unified Webhook notification dispatcher configurable globally or per-task:
  * **Events**: `on_change` (IP changed and successfully updated), `on_failure` (update failed), `on_recovery` (recovered after consecutive failures).
  * **Ordering & Invariants**: Dispatches `Recovery` first to clear failure counters and send recovery alerts, followed by `Change` only if actual IP changed.
  * **Heartbeat Denoising**: Heartbeat updates (`force_update_interval`) that do not change actual IPs suppress `on_change` notifications.
  * **Saturating Failure Counters**: Employs saturating addition (`count.saturating_add(1)`) to prevent integer overflow during prolonged network outages.
  * **Placeholders**: Supports `{{task_name}}`, `{{status}}`, `{{old_ip}}`, `{{new_ip}}`, `{{error_message}}`, `{{timestamp}}`.

#### 2.6 Graceful Shutdown & Lifecycle Management (`lifecycle`)

* **Cross-Platform Signal Handling**:
  * **POSIX (Linux/macOS)**: Listens for `SIGINT`, `SIGTERM`, and `SIGHUP`.
  * **Windows**: Captures console control events (`ctrl_c`, `ctrl_break`, `ctrl_close`, `ctrl_shutdown`, `ctrl_logoff`).
* **Coordinated In-Flight Request Draining**:
  * Broadcasts cancellation via `tokio_util::sync::CancellationToken`.
  * In-flight HTTP requests and assertions are allowed to complete before termination.
  * Bounded shutdown timeout (`global.shutdown_timeout`, default 10s) prevents hung processes.
* **Atomic State Flushing**: Flushes pending state mutations to disk via temporary file rename before shutdown.

#### 2.7 Command Line Interface & Options (`cli`)

* `-c, --config <PATH>`: Path to YAML configuration file (default: `config.yaml`).
* `--once`: Execute once and exit immediately.
* `--daemon`: Long-running daemon mode with asynchronous interval polling.
* `--dry-run`: Probes IPs and previews rendered HTTP templates with secret masking without sending live writes.
* `--check`: Validates configuration syntax and physical interface feasibility without running.
* `--list-providers`: Lists all built-in providers and template details.
* `--show-provider <NAME>` (alias `--provider`): Displays request templates and YAML examples for a provider.
* `-t, --worker-threads <N>`: Tokio runtime worker thread count (1 for true single-threaded runtime).
* `-s, --state <PATH>`: Override path to state persistence JSON file.
* `--write-state [<BOOL>]` / `--no-state`: Controls whether to persist state to disk (supports disabling for read-only systems).
* `-l, --log-level <LEVEL>`: Log level filter (`trace`, `debug`, `info`, `warn`, `error`, `off`; default: `info`).

---

### 3. Configuration Specification Example (`config.yaml`)

```yaml
global:
  interval: 300            # Global default steady-state polling interval (seconds)
  retry_interval: 60       # Shortened retry interval on update failure with DNS mismatch (seconds, default: 60)
  timeout: 10              # HTTP request timeout (seconds)
  shutdown_timeout: 10     # Graceful shutdown maximum wait timeout (seconds)
  worker_threads: 2        # Tokio worker threads (1 for true single-thread, omitted for CPU core count)
  log_level: "info"        # trace | debug | info | warn | error | off
  # proxy: "http://127.0.0.1:7890" # Optional global proxy (http / socks5)
  # dns_server: "8.8.8.8"  # Optional global DNS server

# Generic notification settings (optional)
notification:
  on_change:
    enabled: true
    method: "POST"
    url: "https://api.example.com/notify"
    headers:
      Content-Type: "application/json"
    body: |
      {
        "title": "RDNS IP Change Notification",
        "task": "{{task_name}}",
        "old_ip": "{{old_ip}}",
        "new_ip": "{{new_ip}}"
      }
  on_failure:
    enabled: true
    method: "POST"
    url: "https://api.example.com/notify"
    headers:
      Content-Type: "application/json"
    body: |
      {
        "title": "RDNS Update Failure Alert",
        "task": "{{task_name}}",
        "error": "{{error_message}}"
      }

# Named egress interface definitions
interfaces:
  - name: "Local"          # Identifier (first interface acts as default for tasks)
    interval: 300          # Optional polling interval (inherits global.interval by default)
    retry_interval: 60     # Optional fast retry interval on failure
    dns_server: "8.8.8.8"  # Optional DNS server for remote resolution verification
    ipv4:
      enabled: true
      source: "remote"
      urls:
        - "https://api.ipify.org"
        - "https://ip4.seeip.org"
    ipv6:
      enabled: true
      source: "interface"
      interface: "Local"   # Interface name (friendly name on Windows, eth0 on Linux)
      prefer_slaac: true   # Prioritize stable RFC 4291 EUI-64 SLAAC addresses

  - name: "Tailscale"      # Secondary interface example
    interval: 600
    ipv6:
      enabled: true
      source: "interface"
      interface: "tailscale0"

tasks:
  # Task 1: Dynu dual-stack atomic update (single request pushes v4 and v6)
  - name: "dynu-office-dualstack"
    # interface: "Local"   # Optional; defaults to first interface ("Local")
    domain: "wox-office.freeddns.org" # Checks remote resolution every cycle; reconciles drift
    force_update_interval: 86400      # 24-hour heartbeat keepalive
    request:
      method: "GET"
      url: "https://api.dynu.com/nic/update?hostname=wox-office.freeddns.org&myip={{ipv4}}&myipv6={{ipv6}}&password=${DYNU_PASSWORD}"
      success_regex: "^(good|nochg)"

  # Task 2: Standalone IPv6 JSON Webhook example
  - name: "custom-v6-webhook"
    interface: "Local"     # Shares resolved IP from Local interface, zero redundant queries
    domain: "home.example.com"
    request:
      method: "POST"
      url: "https://api.example.com/v1/ddns/update"
      headers:
        Authorization: "Bearer ${SECRET_TOKEN}"
        Content-Type: "application/json"
      body: |
        {
          "domain": "{{domain}}",
          "ip": "{{ipv6}}"
        }
      success_contains:
        - '"status":"success"'
```

---

### 4. Rust Technology Stack & Architecture

#### 4.1 Key Dependencies

| **Category** | **Crate** | **Rationale** |
| :--- | :--- | :--- |
| **Async Runtime** | `tokio = { version = "1", features = ["full"] }` | Production async scheduling with custom single-thread / multi-thread pools and cross-platform signal listeners |
| **Coordination** | `tokio-util = { version = "0.7", features = ["rt"] }` | Provides `CancellationToken` for hierarchical graceful shutdown broadcasts |
| **DNS Resolver** | `hickory-resolver = { version = "0.26.2", features = ["tokio"] }` | Pure Rust authoritative DNS resolver supporting UDP/TCP queries against system/custom servers |
| **HTTP Engine** | `reqwest = { version = "0.12", default-features = false, features = ["rustls-tls-native-roots", "json", "socks"] }` | Memory-safe `rustls` stack, static binary distribution, platform root CA trust, and SOCKS5 proxy support |
| **CLI Parser** | `clap = { version = "4", features = ["derive", "env"] }` | Strongly typed command line parser supporting `--config`, `--once`, `--dry-run`, `--worker-threads`, `--state`, `--write-state/--no-state`, `-l/--log-level`, etc. |
| **Config & Env** | `serde`, `serde_yaml`, `shellexpand` | Strongly typed deserialization with precise error line numbers and `${ENV_NAME}` expansion |
| **Interface Probing** | `ifaddrsx = "0.4.1"` | High-performance cross-platform adapter enumeration, RFC 4291 EUI-64 SLAAC, RFC 4941 privacy filtering, and CGNAT detection |
| **Text & Regex** | `regex` | Response assertions and template URL parsing |
| **Tracing** | `tracing`, `tracing-subscriber` | Structured asynchronous telemetry and log level filtering |

#### 4.2 Module Hierarchy

```text
src/
├── main.rs                 # Entrypoint: CLI parsing, runtime builder, lifecycle orchestration (SIGHUP reload)
├── cli.rs                  # CLI argument models and validation (Clap Derive)
├── config/                 # Configuration domain module
│   ├── mod.rs              # Unified Config export and loader
│   ├── parser.rs           # YAML deserialization and environment variable expansion (${VAR})
│   ├── model.rs            # Configuration entities (Global, Interface, Task, Request)
│   └── validator.rs        # Business rule validation (regex patterns, intervals, interface validation)
├── provider/               # Predefined DDNS provider registry
│   └── mod.rs              # Built-in provider templates (dynu, dynv6, duckdns, he, noip) and CLI introspection
├── lifecycle/              # Lifecycle and shutdown coordination
│   ├── mod.rs              # LifecycleManager with task draining and reload loops
│   ├── signal.rs           # Cross-platform signal listeners (POSIX SIGINT/SIGTERM/SIGHUP & Windows Console)
│   └── task_manager.rs     # Centralized JoinHandle tracking and drain timeout management
├── ip/                     # IP resolution and DNS verification engine
│   ├── mod.rs              # InterfaceIpResolver dual-stack concurrency and graceful degradation
│   ├── dns.rs              # Remote A/AAAA verification via hickory-resolver
│   ├── remote.rs           # Remote HTTP prober with explicit protocol stack binding
│   └── interface.rs        # Local interface IP reader (ULA, fe80::, RFC 4941 filtering, SLAAC prioritization)
├── engine/                 # HTTP Webhook engine
│   ├── mod.rs              # HttpEngine facade
│   ├── client.rs           # Client builder with rustls-tls-native-roots and connection pools
│   ├── template.rs         # Safe {{ipv4}}, {{ipv6}}, {{domain}} template substitution
│   ├── executor.rs         # Request execution and dry-run preview masking
│   └── verifier.rs         # Status code and regex/contains assertion verifier
├── scheduler/              # Task scheduler and state machine
│   ├── mod.rs              # SchedulerService cluster management
│   └── task.rs             # InterfaceScheduler probe loop and TaskExecutor (JoinSet concurrency, backoff)
├── persistence/            # State persistence
│   ├── mod.rs              # StateStore service (Actor channel, debounce, and read/write splitting)
│   └── atomic_file.rs      # Atomic file persistence via temporary file rename
├── notification/           # Generic notification system
│   ├── mod.rs              # NotificationDispatcher
│   └── events.rs           # Change, failure, and recovery event models
└── error.rs                # Strongly typed domain errors (via thiserror)
```

---

### 5. Resilience & Robustness Design

1. **Idempotency & Minimal Mutation Verification**:
   Checks `current_ipv4 == cached_ipv4 && current_ipv6 == cached_ipv6` before sending requests. If unchanged, DNS is consistent, and heartbeats have not expired, network requests are skipped to protect API quotas.
2. **Failure Isolation & Adaptive Backoff**:
   Concurrent execution with `tokio::task::JoinSet` prevents slow I/O on one task from blocking sibling tasks. Recoverable failures switch immediately to `retry_interval`. Failed periodic heartbeats apply exponential backoff (`60 * 2^(fail - 1)` seconds, capped at `normal_interval`) to prevent API storming.
3. **Dual-Stack Independence & Graceful Degradation**:
   IPv4 and IPv6 probing run concurrently with `tokio::join!`. If one protocol stack fails due to transient upstream routing issues while the other succeeds, the scheduler proceeds in degraded mode for the operational stack, logging warnings without aborting the entire round.
4. **RFC 4941 Temporary Address Filtering**:
   Detects RFC 4941 / RFC 8981 randomized privacy addresses (universal/local bit == 0, non-EUI-64, non-low-suffix static). Prioritizes static, DHCPv6, or EUI-64 SLAAC stable addresses, only falling back to temporary addresses when no stable address exists.
5. **NTP Clock-Skew Protection**:
   DNS cooldown intervals guard against clock jumps. When `now < last_success_time`, RDNS logs a warning and bypasses cooldown to prevent interval deadlocks.
6. **Safe Template Substitution**:
   If a template references `{{ipv4}}` but IPv4 resolution is disabled or failed, the engine aborts the request before dispatch, strictly preventing malformed literal URLs from reaching providers.
7. **Lightweight Static Delivery**:
   Statically compiled targets (`x86_64-pc-windows-msvc`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`) produce a single self-contained binary of only a few megabytes with zero dynamic dependencies, running directly on Windows, Alpine Linux, or OpenWrt routers.
8. **Non-blocking Persistence & Atomic File Operations**:
   StateStore uses a non-blocking channel actor architecture. Memory updates return immediately while a background task debounces writes and offloads atomic renames to `tokio::task::spawn_blocking`, eliminating disk stall in async loops.
