这份需求说明书旨在设计一个基于 **Rust** 编写的通用型、轻量级、完全基于 **Custom URL (Webhook)** 的 DDNS 客户端。它剔除了 `godns` 对各服务商 REST API 的生硬封装，采用通用的 HTTP 请求模板，从架构层面彻底解决“双栈单请求原子更新”和“多实例/多服务商命名冲突”问题。

### 一、 核心设计定位

*   **零服务商硬编码**：不内置任何特定 DNS 服务商（如 Cloudflare, Dynu, AliDNS 等）的专用 SDK，统一抽象为 **HTTP 请求引擎**。
    
*   **命名的 Interface 出口池与任务解耦**：将网络出口抽象为独立的 `interfaces` 池，每个网卡出口周期性执行单次 IP 探测，其绑定的所有 Webhook 任务共享探测结果，彻底根绝多任务场景下的并发重复查询。
    
*   **原生双栈与单请求原子更新**：单任务可同时感知本地 IPv4 与 IPv6，并支持在单次 HTTP 请求模板中同时填充双栈地址。
    
*   **声明式成功校验**：支持基于 HTTP 状态码与响应体正则（如 `good|nochg`）判断是否更新成功，杜绝因服务商返回 200 但业务报错而导致的假成功。

*   **RFC 4291 EUI-64 SLAAC 智能识别**：针对宽带运营商双栈下网卡同时分配 DHCPv6 与 SLAAC 两个公网 IPv6 的场景，自动识别基于硬件 MAC 派生的稳定 SLAAC 地址并优先绑定。

*   **基于 hickory-resolver 的远端 DNS 核对与纠错**：引入标准库 `hickory-resolver` 异步查询公网/自建 DNS 服务器，当云端记录被手动改错时主动感知并纠错。

*   **状态自适应双速轮询机制**：**Update 失败 并且 DNS 查询不一致才缩短周期至 `retry_interval`（默认 60s）**，既能快速自愈，又避免了正常更新后因 DNS TTL 传播延迟导致盲目刷爆服务商 API。

*   **纯静态交付与平台自适应**：基于纯 Rust 实现的 `rustls` 栈并默认导入系统原生证书链，无须依赖 OpenSSL 动态库；支持从极简单线程（嵌入式路由器）到高并发多线程的工作线程配置自适应。
    

### 二、 核心功能需求

#### 1. IP 获取与网卡出口模块 (IP & Interface Resolver)

网络探测统一归属为命名的 `interfaces` 列表管理：

*   **命名的出口接口池 (Named Interface Pool)**：
    *   用户在 `interfaces` 中定义出口名（如 `Local`、`Tailscale`）、查询周期及探测源（`ipv4`/`ipv6`）。
    *   在后台常驻模式下，每个 `interface` 独占 1 个异步协程周期性执行 IP 嗅探，所有引用该网卡的任务直接复用嗅探结果，实现“单次探测，多任务扇出”。
    *   任务若未配置 `interface` 字段，自动默认绑定配置中的首个网卡接口。

*   **远程 HTTP 查询**：
    *   支持通过指定的 URL 查询，底层强制绑定协议栈（`local_address` 绑定 `0.0.0.0` 查 v4，绑定 `::` 查 v6）。
    *   支持配置多个查询源（URL 数组），按顺序回退尝试。
    
*   **本地网卡直读与智能净化**：
    *   支持直接遍历本地物理网卡获取公网地址，支持通过网卡友好名称或正则匹配接口名。
    *   **IPv6 智能净化**：自动过滤链路本地地址（`fe80::/10`）、唯一本地地址 ULA（`fc00::/7`）及 RFC 4941 临时隐私扩展地址（Temporary Addresses）。
    *   **RFC 4291 EUI-64 SLAAC 优先识别 (`prefer_slaac: true`)**：自动判定 IPv6 后 64 位接口标识符（`octets[11] == 0xff && octets[12] == 0xfe`），在存在多个公网 IPv6 时精准优先抓取长久稳定的物理 MAC 映射地址。
    *   **IPv4 私网防呆**：默认排除 RFC 1918 私有地址与 CGNAT（`100.64.0.0/10`），可通过 `allow_private: true` 显式开启私网 DNS 场景。


#### 2. IP 状态管理、DNS 纠错与自适应调度 (State, DNS Reconciliation & Adaptive Loop)

*   **内存缓存与幂等比对**：仅当检测到新 IP 与上次更新成功的 IP 不一致时才触发外部请求；若服务商要求定期心跳，支持配置 `force_update_interval`（如 24 小时强制同步一次）。
    
*   **状态持久化**：支持将上次成功的 IP 状态写入本地持久化文件（如 `state.json`），采用临时文件与原子替换（Atomic Rename）保证可靠性，防止程序重启时盲目重复请求导致服务商触发频控（Rate Limit）。

*   **基于 hickory-resolver 的远端 DNS 记录核对与自动纠错**：
    *   支持在全局或 `interface` 级配置公网 DNS 服务器（`dns_server: "8.8.8.8"` 或 `1.1.1.1:53`），未配置则自动使用宿主机系统 DNS。
    *   当本地 IP 未变动时，每次轮询自动通过 `hickory-resolver` 异步查询当前任务域名的公网 A / AAAA 记录。
    *   若发现远端 DNS 记录与实际本地 IP 不一致（如控制台手动改错），立即判定异常并触发 DDNS 纠错更新。
    *   更新成功后带有 60 秒冷却防抖保护（`last_success_time` 60s 内不重复查 DNS），避免 TTL 缓存传播期的反复无效查询。

*   **状态自适应双速轮询（Dual-Speed Adaptive Polling）**：
    *   **稳态（Steady State）**：系统正常运行时保持长周期 `interval`（默认 300s，节能、省流量且不占用 API 配额）。
    *   **快速恢复态（Reconciliation State）**：**当且仅当 Update 失败 并且 DNS 查询不一致时**，下一次轮询休眠时间自动缩短为 `retry_interval`（默认 60s），进行快速重试以尽早恢复。
    *   **防误判与频控保护**：Update 成功时，即使 DNS 处于 TTL 传播延迟中暂时返回旧记录，也**绝对不缩短周期**，严防误判导致的接口刷屏封号。
    *   **收敛自动回退**：一旦后续 Update 成功且 DNS 记录核对一致，系统自动平滑恢复为正常的 300s 周期。
    

#### 3. 自定义请求引擎、命名参数与预定义服务商 (Request Engine, Task Args & Providers)

将每次更新动作抽象为一个参数化 HTTP 请求，并提供预定义服务商支持：

*   **变量插槽与统一模板渲染（Template Variables & Unified Slots）**：
    *   **内置系统插槽**：
        *   `{{ipv4}}`：当前获取到的公网 IPv4。
        *   `{{ipv6}}`：当前获取到的公网 IPv6。
        *   `{{domain}}`：配置的域名。
        *   `{{timestamp}}`：当前时间戳（秒/毫秒）。
    *   **自定义命名参数插槽 (`args`)**：
        *   Task 支持配置 `args: HashMap<String, String>`，为模板提供额外的命名参数（如 `{{password}}`、`{{token}}`、`{{zone_id}}`）。
        *   URL、Headers、Body 全面走统一的模板引擎 `src/engine/template.rs` 进行安全替换，杜绝重复代码。
        
*   **环境变量展开（Secrets & Env Injection）**：
    *   配置文件全面支持 `${ENV_NAME}` 与 `${ENV_NAME:-default}` 语法，在 YAML 反序列化时自动展开。
    *   `args` 中的密码/Token（如 `password: "${DYNU_PASSWORD}"`）可直接引用环境变量，实现凭据与代码配置隔离。

*   **预定义服务商模板体系 (`provider`)**：
    *   Task 支持声明 `provider: "dynu"`（或 `dynv6`, `duckdns`, `he`, `noip` 等）。
    *   **模板自动装配**：当 Task 未显式配置 `request` 时，系统自动套用对应服务商的标准 HTTP 模板与成功断言（如 Dynu 的 `^(good|nochg)`）。
    *   **参数强校验**：启动时自动核对该服务商必需的参数（如 Dynu 必需 `password`，dynv6 必需 `token`，均需 `domain`），缺失立即给出明确指引。
    *   **内置支持清单**：`dynu` (双栈), `dynu-ipv4`, `dynu-ipv6`, `dynv6` (双栈), `dynv6-ipv4`, `dynv6-ipv6`, `duckdns` (双栈), `duckdns-ipv4`, `duckdns-ipv6`, `he` (Hurricane Electric), `noip` 等。

*   **命令行服务商查询与自省 (`--list-providers` / `--provider`)**：
    *   `rdns --list-providers`：列出所有内置服务商、官网、默认 URL 模板与必需参数。
    *   `rdns --provider <NAME>`（别名 `--show-provider <NAME>`）：显示指定服务商的完整 HTTP 模板、参数说明与可直接复制的 YAML 任务范例。无需配置文件即可独立运行。

*   **完整 HTTP 语义支持**：
    *   支持 `GET`、`POST`、`PUT`、`PATCH`。
    *   支持自定义 Header（如 `Authorization: Bearer <token>`、`Content-Type: application/json`）。
    *   支持自定义 Body（支持 JSON、Form 表单文本）。

*   **TLS 引擎与系统证书信任 (Rustls + System Certs)**：
    *   **纯 Rust 安全协议栈**：采用 `rustls` 纯内存安全实现，避免依赖本地 OpenSSL 动态库，实现真正的全静态零依赖交付。
    *   **默认导入操作系统原生证书链**：默认启用平台原生根证书库（Windows 根证书存储库、macOS Keychain、Linux 系统 CA 目录），自动信任企业内部私有 CA 与系统级信任凭据。
    *   **自签名跳过校验**：支持配置 `tls_insecure: true`（默认 `false`），适配局域网自建私有 DNS Webhook。

*   **网络代理 (Proxy)**：
    *   支持在全局或单任务中配置 `proxy: "http://127.0.0.1:7890"` 或 `socks5://127.0.0.1:1080`，并默认感知系统环境变量（`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`）。

*   **响应断言与验证（Response Assertion）**：
    *   状态码校验（默认 `200..=299`）。
    *   响应体校验：支持 `success_contains`（子串匹配）或 `success_regex`（正则表达式），只有匹配到指定内容（如 Dynu 的 `^(good|nochg)`）才视为成功并更新本地 IP 缓存。
        

#### 4\. 任务调度、运行时与并发控制 (Scheduler, Runtime & Multi-Task)

*   **工作线程数自适应 (Worker Threads)**：
    *   支持在 `config.yaml` 中配置 `global.worker_threads`，并可通过 CLI 参数 `--worker-threads <N>` / `-t <N>` 显式覆盖。
    *   **极简单线程**：配置为 `1` 时以极度轻量的单工作线程异步运行时运行，专为 OpenWrt / MIPS / 128MB 等资源受限的嵌入式路由器量身打造；
    *   **多线程并发**：未指定时默认自适应 CPU 核心数，调度高并发多任务。

*   **任务隔离**：每个任务拥有独立的周期调度器（`interval`）、重试策略与状态机，互不阻塞。
    
*   **单次运行模式 (`--once`)**：支持一次性触发全部任务后直接以对应退出码退出（用于配合系统任务计划程序或 OpenWrt Hotplug 脚本）。
    
*   **服务模式 (`--daemon`)**：常驻运行，基于异步定时器定期轮询，支持优雅退出。

*   **配置热重载 (Hot Reload)**：在 POSIX 平台（Linux/macOS）下监听 `SIGHUP` 信号，平滑重新读取 `config.yaml` 并热更新受影响的任务，无需停机。
    

#### 5\. 通用通知系统 (Generic Notification)

*   统一抽象为基于通用 Webhook 的通知系统，支持在全局或任务级别配置：
    *   **触发事件**：`on_change`（IP 真实变动并更新成功时）、`on_failure`（更新失败时）、`on_recovery`（从连续失败中恢复成功时）。
    *   **通知时序与状态机不变量**：成功更新时优先派发 `Recovery` 消费并清理历史失败计数，触发 `on_recovery` 告警恢复；紧接着根据真实 IP 是否变动（`ip_actually_changed`）决定是否触发 `on_change`。
    *   **心跳与保活去噪**：当因定期心跳（`force_update_interval`）或云端 DNS 记录核对触发强制更新时，若实际 IP 未发生改变，**抑制 `on_change` 事件**，彻底杜绝心跳周期产生的通知风暴。
    *   **告警防刷与饱和保护**：内置连续失败降噪机制（仅首次失败触发告警），失败计数采用饱和递增（Saturating Add），防止长期断网数值溢出回绕重复告警。
    *   **通知占位符**：支持 `{{task_name}}`、`{{status}}`、`{{old_ip}}`、`{{new_ip}}`、`{{error_message}}`、`{{timestamp}}`。
    

#### 6\. 跨平台优雅停机与生命周期管理 (Graceful Shutdown & Lifecycle)

为了保证常驻服务模式（`--daemon`）在容器停止、系统关机或用户手动中断时，不截断进行中的请求且不损坏持久化状态，系统提供跨平台统一的优雅退出机制：

*   **跨平台信号与系统事件捕获**：
    *   **Linux / macOS (POSIX)**：利用 `tokio::signal::unix` 监听 `SIGINT`（Ctrl+C 用户中断）与 `SIGTERM`（容器 stop / systemd 停止 / kill 终止信号）。
    *   **Windows**：利用 `tokio::signal::windows` 捕获控制台控制事件（`ctrl_c`, `ctrl_break`, `ctrl_close`, `ctrl_shutdown`, `ctrl_logoff`），确保在 CMD/PowerShell 窗口被点击关闭、系统注销或服务关机时均能触发停机。
    *   **统一抽象**：在 `main.rs` 封装跨平台通用的 `wait_shutdown_signal()` 异步监听函数，任意有效信号到达即启动安全关闭流程。

*   **任务协同排空 (In-Flight Request Draining)**：
    *   **广播取消信号**：使用 `tokio_util::sync::CancellationToken` 作为全局取消令牌。当主进程捕获停机信号后，立即调用 `token.cancel()` 向所有后台任务调度器广播停机指令，各调度器立即停止开启新的轮询周期。
    *   **保护正在进行的请求**：若某个任务正处于远程 IP 探测或正在向 DNS 服务商发送更新 Webhook 的过程中，停机流程等待当前单次网络请求执行完毕，防止外部请求中途被粗暴杀死导致服务商侧状态不一致。
    *   **强制排空超时兜底 (Drain Timeout)**：主进程在收到停机信号后启动超时倒计时（由全局 `shutdown_timeout` 配置，默认 10 秒）。若超时时限内仍有请求挂起未退出，记录 Warning/Error 日志并强制退出进程，杜绝僵尸进程常驻。

*   **状态原子落盘与资源释放**：
    *   在任务排空完成后，触发内存状态落盘。采用“写入 `.tmp` 临时文件后原子重命名 (`rename`)”机制保存 `state.json`，防止停机断电产生半截损坏的脏文件。
    *   显式释放网络连接池，刷新 `tracing` 日志缓冲区，保证停机前最后一条日志被完整写入标准输出或日志文件。


#### 7. 命令行工具与安全演练 (CLI & Dry-Run Engine)

*   `--list-providers`：列出所有内置预定义服务商名称、官网、请求模板及所需参数。
*   `--provider <NAME>`（别名 `--show-provider <NAME>`）：查询指定服务商的默认请求模板详情与 YAML 任务配置示例。
*   `--check`：静态语法检查，验证 `config.yaml` 格式、必填字段、网络接口与正则表达式合法性。
*   `--dry-run`：执行真实 IP 探测与模板渲染并在控制台输出格式化预览（打印将发往的 Method、URL、Headers、Body、匹配规则，对敏感 Token/密码自动脱敏打码），**但不向服务端发出真实写入请求**。
*   `--config <PATH>`（`-c`）：指定配置文件路径（默认查找 `./config.yaml`）。
*   `--once`：单次触发全部任务更新后以状态码退出。
*   `--daemon`：常驻后台守护运行。
*   `--worker-threads <N>`（`-t`）：指定 Tokio 运行时工作线程数（优先级高于配置文件）。


### 三、 配置文件规范示例 (`config.yaml`)

```yaml
global:
  interval: 300            # 全局默认稳态轮询间隔（秒）
  retry_interval: 60       # Update 失败且 DNS 记录不一致时的快速重试恢复间隔（秒，默认 60）
  timeout: 10              # HTTP 请求超时（秒）
  shutdown_timeout: 10     # 优雅停机排空最长等待时间（秒）
  worker_threads: 2        # Tokio 异步工作线程数（设为 1 则启用极轻量单线程，不设默认自适应 CPU 核心数）
  log_level: "info"        # trace | debug | info | warn | error
  # proxy: "http://127.0.0.1:7890" # 可选全局代理 (支持 http / socks5)
  # dns_server: "8.8.8.8"  # 可选全局 DNS 服务器

# 全局通用通知系统（可选）
notification:
  on_change:
    enabled: true
    method: "POST"
    url: "https://api.example.com/notify"
    headers:
      Content-Type: "application/json"
    body: |
      {
        "title": "RDNS IP 变动通知",
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
        "title": "RDNS 更新告警",
        "task": "{{task_name}}",
        "error": "{{error_message}}"
      }

# 命名的网络出口/接口池定义（每个 interface 对应一个独立的 IP 查询协程，杜绝并发重复查询）
interfaces:
  - name: "Local"          # 接口唯一标识（第一个接口作为任务未显式指定时的默认接口）
    interval: 300          # 可选，IP 查询间隔（默认继承 global.interval）
    retry_interval: 60     # 可选，Update 失败且 DNS 不一致时的快速恢复重试间隔
    dns_server: "8.8.8.8"  # 可选 DNS 服务器，用于向远端核对域名实际解析记录（如发现被手动改错自动恢复）
    ipv4:
      enabled: true
      source: "remote"
      urls:
        - "https://api.ipify.org"
        - "https://ip4.seeip.org"
    ipv6:
      enabled: true
      source: "interface"
      interface: "Local"   # 网卡名称（Windows 为网卡友好名称，Linux 为 eth0 等）
      prefer_slaac: true   # 自动优先匹配稳定的 RFC 4291 EUI-64 SLAAC 地址

  - name: "Tailscale"      # 辅助专用接口示例
    interval: 600
    ipv6:
      enabled: true
      source: "interface"
      interface: "tailscale0"

tasks:
  # 任务 1：Dynu 双栈原子更新（单请求同时推 v4 和 v6，支持环境变量注入）
  - name: "dynu-office-dualstack"
    # interface: "Local"   # 可选，未配置则默认绑定第一个 interface ("Local")
    domain: "wox-office.freeddns.org" # 配置对应域名，每次轮询自动核对远端解析，不一致立即触发纠错更新
    force_update_interval: 86400      # 24小时兜底保活心跳
    request:
      method: "GET"
      # 支持环境变量展开 ${DYNU_PASSWORD}，避免密码明文存储
      url: "https://api.dynu.com/nic/update?hostname=wox-office.freeddns.org&myip={{ipv4}}&myipv6={{ipv6}}&password=${DYNU_PASSWORD}"
      # 校验响应体，必须包含 good 或 nochg 才算成功
      success_regex: "^(good|nochg)"

  # 任务 2：另一个独立的纯 IPv6 更新示例（POST JSON 格式）
  - name: "custom-v6-webhook"
    interface: "Local"     # 共享 Local 接口单次查询到的 IP，不发起任何冗余网络查询
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

### 四、 Rust 技术栈与模块架构

#### 1. 核心依赖选型

| **模块类别** | **推荐 Crates** | **选型理由** |
| --- | --- | --- |
| **异步运行时** | `tokio = { version = "1", features = ["full"] }` | 工业级异步调度标准，支持定制单线程 / 多线程工作线程池及跨平台信号监听 |
| **异步协调与排空** | `tokio-util = { version = "0.7", features = ["rt"] }` | 提供 `CancellationToken` 实现各并发任务的优雅停机广播 |
| **DNS 解析引擎** | `hickory-resolver = { version = "0.26.2", features = ["tokio"] }` | 纯 Rust 官方社区权威 DNS 解析标准库，支持 UDP/TCP 查询与系统/外部 DNS 解析 |
| **HTTP 引擎** | `reqwest = { version = "0.12", default-features = false, features = ["rustls-tls-native-roots", "json", "socks"] }` | 基于 `rustls` 纯内存安全实现，跨平台纯静态编译；默认导入操作系统原生证书链，自动信任企业/系统根证书；支持 SOCKS5 代理 |
| **命令行解析** | `clap = { version = "4", features = ["derive", "env"] }` | 强类型 CLI 解析，易于扩展 `--config`, `--once`, `--dry-run`, `--worker-threads`, `--state` 等命令参数 |
| **配置解析与展开** | `serde`, `serde_yaml`, `shellexpand` | 强类型结构体反序列化，错误提示精确到行号；支持 `${ENV_NAME}` 环境变量安全解析 |
| **网卡 IP 嗅探** | `ifaddrsx = "0.4.1"` | 高性能跨平台（Windows/Linux/macOS）网卡与 IP 枚举，原生支持 Windows 友好网卡名称、RFC 4291 EUI-64 SLAAC 识别、RFC 4941 临时地址识别与 RFC 6598 CGNAT 识别 |
| **文本与正则** | `regex` | 用于响应断言校验与 URL 变量模板替换 |
| **日志与观测** | `tracing`, `tracing-subscriber` | 结构化异步日志输出，排查异步任务追踪极其直观 |

#### 2. 系统模块分层架构

```text
src/
├── main.rs                 # 进程入口：CLI 解析、运行时构建、顶级生命周期编排 (支持 SIGHUP 热重载)
├── cli.rs                  # 命令行参数模型与校验 (Clap Derive，支持 --state, -t, --check 等)
├── config/                 # 配置领域模块
│   ├── mod.rs              # 统一导出 Config 结构体与加载接口
│   ├── parser.rs           # YAML 反序列化、环境变量展开 (${VAR})
│   ├── model.rs            # 强类型配置实体定义 (Global, Interface, Task, Request)
│   └── validator.rs        # 业务规则校验 (网卡名/正则、重试周期、正则表达式语法等)
├── provider/               # 预定义 DDNS 服务商注册中心
│   └── mod.rs              # 内置 dynu, dynv6, duckdns, he, noip 模板与 CLI 查询
├── lifecycle/              # 生命周期与停机管理
│   ├── mod.rs              # 生命周期编排服务 (LifecycleManager，支持 Drain 与 Reload)
│   ├── signal.rs           # 跨平台信号监听器 (POSIX SIGINT/SIGTERM/SIGHUP & Windows Console)
│   └── task_manager.rs     # JoinHandle 集中追踪与排空超时控制器
├── ip/                     # IP 嗅探与 DNS 核对引擎
│   ├── mod.rs              # InterfaceIpResolver 双栈并发调度与单栈优雅降级
│   ├── dns.rs              # 基于 hickory-resolver 的云端 A/AAAA 记录核对引擎
│   ├── remote.rs           # 基于协议栈强制绑定的远程 HTTP 探测器
│   └── interface.rs        # 本地网卡直读与智能净化器 (过滤 ULA、fe80::、RFC 4941 临时地址，优先稳定地址/SLAAC)
├── engine/                 # HTTP Webhook 驱动引擎
│   ├── mod.rs              # HttpEngine 外观服务
│   ├── client.rs           # 基于 rustls-tls-native-roots 的 Client 构造与连接池
│   ├── template.rs         # {{ipv4}}, {{ipv6}}, {{domain}} 占位符安全替换
│   ├── executor.rs         # 请求构造与执行 (支持 Dry-Run 模式拦截)
│   └── verifier.rs         # 响应状态码与正则/包含断言校验
├── scheduler/              # 任务调度与状态机
│   ├── mod.rs              # 调度集群服务 (SchedulerService)
│   └── task.rs             # 接口探测协程 (InterfaceScheduler) 与单任务执行器 (TaskExecutor，支持 JoinSet 并发、心跳退避)
├── persistence/            # 状态持久化
│   ├── mod.rs              # StateStore 服务 (Channel Actor 异步非阻塞落盘与读写分离)
│   └── atomic_file.rs      # 基于临时文件与系统 Rename 的原子落盘保证
├── notification/           # 通用通知系统
│   ├── mod.rs              # NotificationDispatcher 通知分发器
│   └── events.rs           # 变更、失败、恢复事件模型
└── error.rs                # 领域强类型错误定义 (基于 thiserror)
```

### 五、 异常防护与健壮性设计

1.  **防抖与最小变更校验**：
    在向远程发送请求前，检查 `current_ipv4 == cached_ipv4 && current_ipv6 == cached_ipv6`。若无变动、DNS 记录一致且未达到心跳阈值，直接跳过请求，杜绝 API 刷屏。
    
2.  **局部异常隔离与自适应退避**：
    每个接口内部绑定的多任务采用 `tokio::task::JoinSet` 并发驱动，单任务慢 I/O 不会阻塞同接口其他任务。当任务更新失败且 DNS 记录不一致（或无 domain 任务）时，下一次轮询自动切入 `retry_interval` 快速重试；当心跳保活强制更新失败时，启用指数退避（`60 * 2^(fail-1)` 秒，封顶于正常周期），防止服务商宕机时每轮死刷。
    
3.  **双栈独立探测与单栈优雅降级**：
    IPv4 与 IPv6 探测采用 `tokio::join!` 并发发起。当某一协议栈因上游路由或网络波动暂时失效时，只要另一协议栈探测成功，即以降级模式驱动相应单栈更新任务，记录 Warning 日志而非粗暴中断整个接口轮询。

4.  **RFC 4941 临时 IPv6 地址过滤与稳定地址优先**：
    智能识别 IPv6 Privacy Extensions 生成的短期外发临时地址（Universal/Local 掩码 `0x02` 为 0 且非 EUI-64 / 静态 DHCP），优先选用静态、DHCPv6 或 EUI-64 SLAAC 稳定地址作为入站解析目标，仅在宿主机纯隐私地址模式下回退选用。

5.  **NTP 时钟回拨保护**：
    DNS 查询防抖冷却期支持单调时钟保护。若宿主机发生 NTP 时钟跳变导致当前时间小于上次成功时间，主动绕过冷却阻断并记录 Warning 日志，防止系统陷入无限防抖休眠。

6.  **安全变量替换**：
    若请求模板中包含了 `{{ipv4}}`，但该任务当前未开启 IPv4 或获取失败，引擎应直接中断本次 HTTP 请求并记录 Warn 日志，严禁将包含字面量 `{{ipv4}}` 的畸形 URL 发送给服务商。
    
7.  **轻量静态交付**：
    利用 `x86_64-pc-windows-msvc` / `x86_64-unknown-linux-musl` / `aarch64-unknown-linux-musl` 进行静态编译，产物为一个几兆大小的单二进制文件，零系统动态依赖（纯 Rust 栈无需 OpenSSL），可直接丢到 Windows、Alpine Linux 或 OpenWrt 路由器中运行。

8.  **非阻塞异步状态持久化与原子落盘**：
    持久化存储采用 Channel Actor 异步读写分离架构，任务更新内存快照后立即返回，由后台 Worker 合并防抖并移交 `spawn_blocking` 执行临时文件原子重命名（Atomic Rename），彻底隔绝慢磁盘 I/O 阻塞异步调度循环。接收到停机信号时触发排空与最终保存，杜绝持久化文件损坏。