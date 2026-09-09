这份需求说明书旨在设计一个基于 **Rust** 编写的通用型、轻量级、完全基于 **Custom URL (Webhook)** 的 DDNS 客户端。它剔除了 `godns` 对各服务商 REST API 的生硬封装，采用通用的 HTTP 请求模板，从架构层面彻底解决“双栈单请求原子更新”和“多实例/多服务商命名冲突”问题。

### 一、 核心设计定位

*   **零服务商硬编码**：不内置任何特定 DNS 服务商（如 Cloudflare, Dynu, AliDNS 等）的专用 SDK，统一抽象为 **HTTP 请求引擎**。
    
*   **数组驱动的并发任务架构**：以 `tasks` 列表（Array）为基础，彻底告别以“服务商名称作为 Map Key”的死板设计，支持任意数量、同服务商或不同服务商的任务并行独立运行。
    
*   **原生双栈与单请求原子更新**：单任务可同时感知本地 IPv4 与 IPv6，并支持在单次 HTTP 请求模板中同时填充双栈地址。
    
*   **声明式成功校验**：支持基于 HTTP 状态码与响应体正则（如 `good|nochg`）判断是否更新成功，杜绝因服务商返回 200 但业务报错而导致的假成功。

*   **纯静态交付与平台自适应**：基于纯 Rust 实现的 `rustls` 栈并默认导入系统原生证书链，无须依赖 OpenSSL 动态库；支持从极简单线程（嵌入式路由器）到高并发多线程的工作线程配置自适应。
    

### 二、 核心功能需求

#### 1\. IP 获取模块 (IP Resolver)

每个任务可独立配置 IPv4 / IPv6 的获取策略：

*   **远程 HTTP 查询**：支持通过指定的 URL 查询，底层需支持强制绑定协议栈（`local_address` 绑定 `0.0.0.0` 查 v4，绑定 `::` 查 v6）。
    
*   **本地网卡直读与智能过滤**：
    *   支持直接遍历本地物理网卡获取公网地址，支持通过正则过滤接口名（如 `eth.*|enp.*`）及 IPv6 前缀。
    *   **IPv6 智能净化**：自动过滤链路本地地址（`fe80::/10`）、唯一本地地址 ULA（`fc00::/7`）及 RFC 4941 临时隐私扩展地址（Temporary Addresses），确保仅采集长期稳定的公网 SLAAC / DHCPv6 单播地址。
    *   **IPv4 私网防呆**：默认排除 RFC 1918 私有地址与 CGNAT（`100.64.0.0/10`），可通过 `allow_private: true` 显式开启私网 DNS 场景。
    
*   **容错与回退**：支持配置多个查询源（URL 数组），按顺序回退尝试。
    

#### 2\. IP 状态管理与变更检测 (State & Cache Engine)

*   **内存缓存与幂等比对**：仅当检测到新 IP 与上次更新成功的 IP 不一致时才触发外部请求；若服务商要求定期心跳，支持配置 `force_update_interval`（如 24 小时强制同步一次）。
    
*   **状态持久化（可选）**：支持将上次成功的 IP 状态写入本地持久化文件（如 `state.json`），防止程序重启时盲目重复请求导致服务商触发频控（Rate Limit）。
    

#### 3\. 自定义请求引擎与 TLS 规范 (Custom Request Engine & TLS)

将每次更新动作抽象为一个参数化 HTTP 请求：

*   **变量插槽（Template Variables）**：支持在 URL、Headers、Body 中使用以下占位符：
    *   `{{ipv4}}`：当前获取到的公网 IPv4。
    *   `{{ipv6}}`：当前获取到的公网 IPv6。
    *   `{{domain}}`：配置的域名。
    *   `{{timestamp}}`：当前时间戳（秒/毫秒）。
        
*   **环境变量展开（Secrets & Env Injection）**：配置文件全面支持 `${ENV_NAME}` 与 `${ENV_NAME:-default}` 语法，避免明文硬编码 Token / 密码，方便容器与 CI/CD 部署。

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
    *   **触发事件**：`on_change`（IP 变更成功时）、`on_failure`（更新失败时）、`on_recovery`（从连续失败中恢复时）。
    *   **通知占位符**：支持 `{{task_name}}`、`{{status}}`、`{{old_ip}}`、`{{new_ip}}`、`{{error_message}}`、`{{timestamp}}`。
    *   **告警防刷**：内置连续失败降噪机制（仅首次失败触发告警或按指数退避发送），防止断网期间告警轰炸。
    

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


#### 7\. 命令行工具与安全演练 (CLI & Dry-Run Engine)

*   `--check`：静态语法检查，验证 `config.yaml` 格式、必填字段、网络接口与正则表达式合法性。
*   `--dry-run`：执行真实 IP 探测与模板渲染并在控制台输出格式化预览（打印将发往的 Method、URL、Headers、Body、匹配规则，对敏感 Token 自动脱敏打码），**但不向服务端发出真实写入请求**。
*   `--config <PATH>`（`-c`）：指定配置文件路径（默认查找 `./config.yaml`、`/etc/rdns/config.yaml` 等）。
*   `--once`：单次触发全部任务更新后以状态码退出。
*   `--daemon`：常驻后台守护运行。
*   `--worker-threads <N>`（`-t`）：指定 Tokio 运行时工作线程数（优先级高于配置文件）。


### 三、 配置文件规范示例 (`config.yaml`)

采用 YAML 的列表驱动格式（`tasks:`），并原生支持环境变量注入：

YAML

```yaml
global:
  interval: 300            # 全局默认轮询间隔（秒）
  timeout: 10              # HTTP 请求超时（秒）
  shutdown_timeout: 10     # 优雅停机排空最长等待时间（秒）
  worker_threads: 2        # Tokio 异步工作线程数（设为 1 则启用极轻量单线程，不设默认自适应 CPU 核心数）
  log_level: "info"        # trace | debug | info | warn | error
  # proxy: "http://127.0.0.1:7890" # 可选全局代理 (支持 http / socks5)

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

tasks:
  # 任务 1：Dynu 双栈原子更新（单请求同时推 v4 和 v6，支持环境变量注入）
  - name: "dynu-office-dualstack"
    interval: 600

    ipv4:
      enabled: true
      source: "remote"
      urls:
        - "https://api.ipify.org"
        - "https://ip4.seeip.org"

    ipv6:
      enabled: true
      source: "remote"
      urls:
        - "https://api64.ipify.org"
        - "https://ip6.seeip.org"
      # 或者走网卡读取 (自动排除链路本地 fe80::、内网 ULA fc00::/7 及临时隐私扩展地址):
      # source: "interface"
      # interface: "eth0"
      # ipv6_prefix: "240e:"

    request:
      method: "GET"
      # 支持环境变量展开 ${DYNU_PASSWORD}，避免密码明文存储
      url: "https://api.dynu.com/nic/update?hostname=wox-office.freeddns.org&myip={{ipv4}}&myipv6={{ipv6}}&password=${DYNU_PASSWORD}"
      # 校验响应体，必须包含 good 或 nochg 才算成功
      success_regex: "^(good|nochg)"

  # 任务 2：另一个独立的纯 IPv6 更新示例（POST JSON 格式）
  - name: "custom-v6-webhook"
    interval: 300

    ipv6:
      enabled: true
      source: "interface"
      interface: "eth0"

    request:
      method: "POST"
      url: "https://api.example.com/v1/ddns/update"
      headers:
        Authorization: "Bearer ${SECRET_TOKEN}"
        Content-Type: "application/json"
      body: |
        {
          "domain": "home.example.com",
          "ip": "{{ipv6}}"
        }
      success_contains:
        - '"status":"success"'
``` 

### 四、 Rust 技术栈与模块架构

#### 1\. 核心依赖选型

| **模块类别** | **推荐 Crates** | **选型理由** |
| --- | --- | --- |
| **异步运行时** | `tokio = { version = "1", features = ["rt-multi-thread", "signal", "time", "macros"] }` | 工业级异步调度标准，支持定制单线程 / 多线程工作线程池及跨平台信号监听 |
| **异步协调与排空** | `tokio-util = { version = "0.7", features = ["rt"] }` | 提供 `CancellationToken` 实现各并发任务的优雅停机广播 |
| **HTTP 引擎** | `reqwest = { version = "0.12", default-features = false, features = ["rustls-tls-native-roots", "json", "socks"] }` | 基于 `rustls` 纯内存安全实现，跨平台纯静态编译；默认导入操作系统原生证书链，自动信任企业/系统根证书；支持 SOCKS5 代理 |
| **命令行解析** | `clap = { version = "4", features = ["derive", "env"] }` | 强类型 CLI 解析，易于扩展 `--config`, `--once`, `--dry-run`, `--worker-threads` 等命令参数 |
| **配置解析与展开** | `serde`, `serde_yaml`, `shellexpand` | 强类型结构体反序列化，错误提示精确到行号；支持 `${ENV_NAME}` 环境变量安全解析 |
| **网卡 IP 嗅探** | `get_if_addrs` | 轻量级跨平台（Linux/Windows/macOS）网卡 IP 枚举 |
| **文本与正则** | `regex` | 用于响应断言校验与 URL 变量模板替换 |
| **日志与观测** | `tracing`, `tracing-subscriber` | 结构化异步日志输出，排查异步任务追踪极其直观 |

#### 2\. 系统模块分层架构

Plaintext

```
src/
├── main.rs            # CLI 入口、定制 Tokio Runtime (worker_threads)、跨平台信号监听
├── config.rs          # YAML 反序列化、环境变量展开 (${VAR}) 与合法性校验
├── state.rs           # 任务运行状态、IP 历史比对与原子持久化缓存 (rename 机制)
├── ip_fetcher/        # IP 探测引擎
│   ├── mod.rs         # 统一 IP 获取 trait
│   ├── remote.rs      # HTTP 请求探测 (绑定指定协议栈)
│   └── interface.rs   # 本地网卡直接读取 (过滤 ULA、fe80:: 与临时隐私扩展)
├── engine/            # HTTP Webhook 驱动引擎
│   ├── client.rs      # 基于 rustls-tls-native-roots 的 Reqwest Client 单例封装
│   ├── template.rs    # {{ipv4}}, {{ipv6}}, {{domain}} 占位符安全替换
│   ├── requester.rs   # 构造并执行请求 (支持 Dry-Run 拦截)
│   └── verifier.rs    # 基于状态码与 regex / contains 进行断言判断
├── scheduler.rs       # 任务调度循环 (结合 CancellationToken 实现优雅退出与 SIGHUP 热重载)
└── notification.rs    # 通用 Webhook 告警回调 (on_change / on_failure / on_recovery)
``` 

### 五、 异常防护与健壮性设计

1.  **防抖与最小变更校验**：
    
    在向远程发送请求前，检查 `current_ipv4 == cached_ipv4 && current_ipv6 == cached_ipv6`。若无变动且未达到心跳阈值，直接跳过请求，杜绝 API 刷屏。
    
2.  **局部异常隔离与退避**：
    
    当 `task A` 遇到 DNS 超时或服务商 502 时，利用 `tokio::spawn` 隔离故障上下文，严禁崩溃进程，只打印结构化错误日志，并在下个周期按指数退避算法重试，不影响 `task B` 的正常运行。
    
3.  **安全变量替换**：
    
    若请求模板中包含了 `{{ipv4}}`，但该任务当前未开启 IPv4 或获取失败，引擎应直接中断本次 HTTP 请求并记录 Warn 日志，严禁将包含字面量 `{{ipv4}}` 的畸形 URL 发送给服务商。
    
4.  **轻量静态交付**：
    
    利用 `x86_64-pc-windows-msvc` / `x86_64-unknown-linux-musl` / `aarch64-unknown-linux-musl` 进行静态编译，产物为一个几兆大小的单二进制文件，零系统动态依赖（纯 Rust 栈无需 OpenSSL），可直接丢到 Windows、Alpine Linux 或 ImmortalWrt 路由器中运行。

5.  **停机排空与原子落盘**：
    
    接收到退出信号时，主控流程进入排空倒计时并广播 `CancellationToken`，保护正在进行的单次请求完整结束；状态持久化采用“临时文件写入 + 原子重命名 (Atomic Rename)”策略，杜绝因强制切断或断电引发的持久化文件损坏。