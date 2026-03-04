<p align="center">
  <strong>🛡️ AG Proxy Manager</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Tauri-v2-blue?logo=tauri" alt="Tauri v2" />
  <img src="https://img.shields.io/badge/Rust-1.75+-orange?logo=rust" alt="Rust" />
  <img src="https://img.shields.io/badge/Platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey" alt="Platform" />
  <img src="https://img.shields.io/badge/License-MIT-green" alt="License" />
  <img src="https://img.shields.io/badge/i18n-中文%20%7C%20English-9cf" alt="i18n" />
</p>

<p align="center">
  <a href="README.en.md">English</a> · <b>简体中文</b>
</p>

---

**AG Proxy Manager** 是一款基于 **Tauri v2**（Rust 后端 + 原生 JS 前端）构建的高性能桌面代理管理器，专为 Antigravity IDE 设计。它在本地建立一个 HTTPS 代理服务器，位于 IDE 与上游 Google Cloud Code API 之间，提供多账号轮换、配额感知的智能路由、请求链路追踪以及一键 IDE 集成——运行时无需任何外部依赖，完全 在Antigravity 里使用额度，暂未发现封号情况。

---

## ✨ 功能特性

### 🔑 多账号管理
- 支持通过 JSON 文件、Refresh Token 或 Google OAuth2 授权登录导入账号
- 支持单个 JSON、JSON 数组、JSONL（每行一个 JSON）及目录批量导入
- 每个账号可单独启用/禁用，并记录禁用原因和时间戳
- 会话间持久化当前选中账号
- 支持大量凭证的流式渐进加载，不阻塞 UI
- 自动去重（以邮箱为唯一键）

### 📊 配额监控
- 实时查询 Claude、Gemini Pro、Gemini Flash、Gemini Image 各模型组剩余配额
- 可视化进度条，颜色区分配额百分比
- 可配置配额切换阈值（0% / 20% / 40% / 60% / 80%）
- 可选每 2 分钟自动刷新配额
- 配额数据本地缓存，支持即时展示

### 🔄 智能路由
- **填充模式（Fill）** — 坚持使用同一账号直到配额耗尽，自动切换下一个
- **轮询模式（Round-robin）** — 在所有可用账号间均匀分发请求
- **容量故障转移** — 遇到 `MODEL_CAPACITY_EXHAUSTED`（HTTP 503）时，自动以互补模型（`claude-opus-4-6-thinking` ↔ `claude-sonnet-4-6-thinking`）重试，共最多 4 次尝试

### 🌐 双传输架构
- **代理模式（Legacy）** — 直接 HTTPS 代理，按 sandbox → daily → prod 顺序尝试三个上游端点
- **网关模式（Gateway）** — 通过本地官方 Language Server Bridge 中转，失败时自动回退到代理模式
- 支持完全自定义上游 URL

### 🔧 IDE 集成
- 一键对 4 个 IDE JS 核心文件打补丁（或从 `.js.bak` 备份还原）
- 自动生成自签名 CA 证书（有效期 2024–2034）并管理 Windows 系统信任库（`certutil`）
- 支持自定义补丁目标 URL

### 📈 Token 用量统计
- 按账号跟踪输入、输出、缓存读取、缓存创建 Token 用量
- 自动解析 Gemini（`usageMetadata`）、Claude（`usage.input_tokens`）、OpenAI（`usage.prompt_tokens`）三种 SSE 格式
- 持久化存储（每 10 次请求自动写盘），支持手动重置
- 内存中保留最近 1000 条请求记录

### 🔍 请求链路追踪
- 可视化请求链：Client → Local Proxy → Gateway → LS Bridge → Upstream
- 每跳显示状态码，可点击查看详情
- 正向（请求 →）与反向（响应 ←）双向展示

### 🎨 现代化界面
- 支持深色 / 浅色 / 跟随系统 三种主题
- 中文 / 英文 双语界面（设置 → 外观 → 语言）
- 7 个功能页面的侧边栏导航
- 实时 Toast 通知 + 流畅动画与玻璃拟态设计元素

### 🛡️ 第三方 AI 供应商
- 配置任意第三方 AI 供应商（OpenAI / Gemini / Claude 协议均支持）
- 模型名称映射（例如将 `gemini-3-flash` 映射到 `gpt-5.3-codex`），支持请求格式自动转换
- 每个供应商可单独启用/禁用，API Key 脱敏显示
- 供应商请求失败时最多重试 3 次（指数退避）

---

## 🏗️ 技术架构

### 项目结构

```
ag-proxy-tools/
├── src/                          # 前端（原生 JS + HTML + CSS）
│   ├── index.html                # 应用布局（~950 行）
│   ├── main.js                   # 状态管理 & IPC 调用（~2800 行）
│   ├── styles.css                # 完整设计系统（~2700 行）
│   └── assets/                   # 静态资产
│
├── src-tauri/                    # 后端（Rust + Tauri v2）
│   ├── tauri.conf.json           # Tauri 应用配置
│   └── src/
│       ├── main.rs               # 程序入口
│       ├── lib.rs                # Tauri 插件注册 & AppState 初始化
│       ├── models.rs             # 核心数据结构定义
│       ├── proxy.rs              # HTTPS 代理引擎（~3300 行）
│       ├── proxy_error.rs        # 错误分类 & 账号封禁逻辑
│       ├── account.rs            # 账号 CRUD、OAuth2 流程、流式加载
│       ├── quota.rs              # 配额查询 & 缓存（并发）
│       ├── provider.rs           # 第三方供应商路由 & 协议转换
│       ├── ls_bridge.rs          # 官方 Language Server Bridge（~1050 行）
│       ├── cert.rs               # TLS CA 生成 & 系统信任库操作
│       ├── patch.rs              # IDE JS 文件补丁
│       ├── token_stats.rs        # Token 用量统计管理器
│       ├── protobuf.rs           # 自研轻量级 Protobuf 编解码器
│       ├── constants.rs          # API 端点 & OAuth 配置
│       └── utils.rs              # 公共工具函数
│
├── scripts/                      # 构建 & 维护脚本
│   ├── check-encoding.js         # 验证文件编码一致性
│   ├── check-i18n-coverage.js    # 检查 i18n 翻译覆盖率
│   └── round-maintenance.ps1     # 自动化维护脚本
│
├── package.json
└── src-tauri/Cargo.toml
```

### 技术栈

| 层次 | 技术 | 版本 | 用途 |
|---|---|---|---|
| **前端** | Vanilla JS + HTML5 + CSS3 | — | UI 渲染、状态管理、IPC 调用 |
| **后端** | Rust（Tokio 异步运行时） | 1.75+ | 代理服务器、OAuth、文件 I/O、加密 |
| **框架** | Tauri | v2 | 桌面打包、IPC 桥接、系统集成 |
| **HTTP 客户端** | reqwest + rustls | 0.12 | 异步 HTTP/1.1 & HTTP/2 + TLS（纯 Rust） |
| **HTTP 服务器** | hyper + axum + hyper-util | 1 / 0.7 / 0.1 | HTTPS 代理服务器 & 本地网关 |
| **TLS** | rustls + rcgen + tokio-rustls | 0.23 / 0.13 / 0.26 | CA 生成、HTTPS 拦截 |
| **序列化** | serde + serde_json | 1 | JSON 序列化/反序列化 |
| **正则** | regex | 1 | URL 替换、项目 ID 提取 |
| **Protobuf** | 自研编解码（`protobuf.rs`） | — | LS Bridge 协议，无代码生成依赖 |
| **文件对话框** | rfd | 0.15 | 原生文件/目录选择器 |

---

## 🔄 数据流与请求处理

```
                        ┌─────────────────── AG Proxy Manager ───────────────────┐
                        │                                                         │
IDE 请求 ──HTTPS──────► │   本地代理 (port 9527)                                  │
                        │          │                                              │
                        │          ├──[供应商路由]──────► 第三方 AI 供应商         │
                        │          │                                              │
                        │          ├──[代理模式]─────────────────────────────►  │
                        │          │           sandbox.googleapis.com (主)        │
                        │          │           daily-cloudcode.googleapis.com     │
                        │          │           cloudcode-pa.googleapis.com (备)   │
                        │          │                                              │
                        │          └──[网关模式]                                  │
                        │                 │                                       │
                        │         本地网关 Extension Server                       │
                        │                 │                                       │
                        │         官方 LS 进程 (HTTPS 随机端口) ────────────────►│
                        └─────────────────┼───────────────────────────────────────┘
                                          │
                                          ▼
                              cloudcode-pa.googleapis.com
```

### 请求处理流水线（8步）

1. **TLS 解密** — 本地代理用自签名 CA 证书终止 IDE 的 HTTPS 连接（端口 9527）
2. **账号选择** — 根据路由策略（fill / round-robin）和配额缓存，选出可用账号；调用 `pick_account_index()` 实现
3. **Token 刷新** — 检测 Access Token 有效期，提前 5 分钟强制刷新（`do_refresh_token()`）
4. **请求改写** — 将体内 `"name":"projects/"` 类占位符替换为账号真实 GCP 项目 ID（6 阶段正则）
5. **供应商匹配** — 按模型名在第三方供应商中查找映射，命中则走独立协议转换路径
6. **上游转发** — 代理模式：依次尝试 3 个上游端点；网关模式：先转 LS Bridge，失败回退直连
7. **响应流式透传** — SSE 流式响应通过 `StreamBody` 透传回 IDE，同时提取 Token 用量元数据
8. **容量故障转移** — 遇到 HTTP 503 `MODEL_CAPACITY_EXHAUSTED`，以互补模型重试（最多 4 次）

### 上游端点顺序

| 优先级 | 主机名 | 说明 |
|---|---|---|
| 1（主） | `daily-cloudcode-pa.sandbox.googleapis.com` | 沙盒环境，最稳定 |
| 2（备） | `daily-cloudcode-pa.googleapis.com` | 每日构建环境 |
| 3（末） | `cloudcode-pa.googleapis.com` | 生产环境 |

---

## 🔐 账号封禁逻辑

| 触发条件 | 是否封禁 | 持续时间 | 触发时机 |
|---|---|---|---|
| 手动禁用（`disabled = true`） | ✅ 封禁 | 手动解除前永久 | 用户操作 |
| OAuth `invalid_grant` 错误 | ✅ 永久封禁 | 手动解除前永久 | **配额刷新时** |
| Token 刷新返回 `Bad Request` | ✅ 临时封禁 | 5 分钟 | 代理请求时 |
| 配额缓存 `is_forbidden = true` | ✅ 封禁 | 重新查询配额后解除 | **配额刷新时** |
| 上游返回 HTTP 401/403/429 | ❌ **不封禁** | — | 代理请求时 |
| 上游返回其他任何非 2xx | ❌ **不封禁** | — | 代理请求时 |

> **核心原则**：只有**配额刷新过程**中发生的认证失败才会触发账号封禁。代理转发时的上游错误（401/403/429 等）属于平台侧问题，不影响账号状态，避免误判。

### Token 刷新封禁细节

`Bad Request` 类型的封禁会在 `quota_error` 字段中记录时间戳，5 分钟后 `is_account_blocked()` 自动解封，无需人工干预。

---

## 🔒 IDE 补丁工作原理

补丁功能修改以下 4 个 IDE JS 文件（先备份为 `.js.bak`）：

| 文件 | 位置 | 说明 |
|---|---|---|
| `vs/workbench/api/node/extensionHostProcess.js` | IDE 安装目录/out/ | 扩展宿主进程 |
| `vs/workbench/api/worker/extensionHostWorkerMain.js` | IDE 安装目录/out/ | Worker 扩展宿主 |
| `main.js` | IDE 安装目录/ | 主进程入口 |
| `vs/code/node/cliProcessMain.js` | IDE 安装目录/out/ | CLI 进程入口 |

**补丁操作：**
1. 用正则 `https://(.*cloudcode.*\.com|127\.0\.0\.1:\d+|...)` 替换所有上游 URL 为代理地址
2. 在文件头注入 `process.env.NODE_TLS_REJECT_UNAUTHORIZED='0';`（忽略自签名证书校验）
3. 已备份文件不会重复备份（保护原始文件）

**IDE 自动探测路径（Windows）：**
- `%LOCALAPPDATA%\Programs\Antigravity`
- `%ProgramFiles%\Antigravity`

---

## � Language Server Bridge 工作原理

网关模式通过实现官方 LS 的 Extension Server 协议与 LS 进程通信：

1. **启动 Extension Server** — 在随机端口监听 LS 回调 RPC
2. **启动 LS 进程** — 以 `--extension_server_host=localhost:<port>` 参数启动，向 stdin 写入 Protobuf `Metadata`
3. **端口发现** — 等待 LS 回调 `LanguageServerStarted`（最长 60 秒），获取 LS 本地 HTTPS 端口
4. **Token 注入** — 响应 `SubscribeToUnifiedStateSyncTopic`，通过长连接流式推送 OAuth Access Token
5. **请求转发** — 代理将 IDE 请求转到 LS 的 HTTPS 端口，LS 再转发到上游官方 API

Extension Server 还实现了以下 LS 探针接口（返回 stub）：
- `IsAgentManagerEnabled`、`GetChromeDevtoolsMcpUrl`
- `SubscribeToUnifiedStateSyncTopic`（长连接，保持 Token 推送）

---

## �📦 环境要求

| 依赖 | 版本 | 安装方式 |
|---|---|---|
| Rust + cargo | 1.75+ | [rustup.rs](https://rustup.rs/) |
| Node.js + npm | 18+ | [nodejs.org](https://nodejs.org/) |
| Tauri CLI | v2 | `npm install -g @tauri-apps/cli@^2` |
| 操作系统 | Windows 10/11（主平台） | Linux/macOS 需额外配置证书管理 |

> **Windows 注意**：证书导入/删除操作（`import_cert` / `remove_cert`）需要管理员权限，代理服务器本身无需管理员权限。

---

## 🚀 快速开始

### 克隆 & 安装依赖

```bash
git clone https://github.com/your-username/ag-proxy-tools.git
cd ag-proxy-tools
npm install
```

### 开发模式

```bash
npx tauri dev
```

前端支持热重载；Rust 后端修改需重新编译。

### 生产构建

```bash
npx tauri build
```

Windows NSIS 安装包输出路径：
```
src-tauri/target/release/bundle/nsis/ag-proxy-tools_*.exe
```

---

## ⚙️ 配置参考

### 代理设置

| 配置项 | 默认值 | 可选值 | 说明 |
|---|---|---|---|
| 代理端口 | `9527` | 1024–65535 | 本地 HTTPS 监听端口 |
| 路由策略 | `fill` | `fill` / `round-robin` | 账号路由方式 |
| 配额阈值 | `0` | 0/20/40/60/80 | 配额低于此百分比时切换账号（仅 fill 模式） |
| 请求头透传 | `开启` | 开启 / 关闭 | 是否将 IDE 原始请求头转发上游 |
| 传输模式 | `代理模式` | `legacy` / `client_gateway` | 代理模式 vs. 网关模式 |
| 上游服务器 | `sandbox` | `sandbox` / `custom` | 官方上游链或自定义 URL |
| HTTP 版本 | `auto` | `auto` / `http1` / `http2` / `http10` | HTTP 协议版本协商 |
| 容量故障转移 | `开启` | 开启 / 关闭 | MODEL_CAPACITY_EXHAUSTED 时自动重试 |

### 环境变量

| 变量名 | 说明 |
|---|---|
| `AG_PROXY_GOOGLE_CLIENT_SECRET` | 自定义 Google OAuth 客户端密钥（优先于配置文件） |
| `AG_PROXY_OFFICIAL_LS_BINARY_PATH` | 指定官方 LS 二进制文件路径（优先于自动探测） |
| `AG_PROXY_STREAM_AUTH_PASSTHROUGH` | 设为 `1` 时，`streamGenerateContent` 请求透传原始 Authorization 头 |
| `AG_PROXY_VERBOSE_HEADER_LOG` | 设为 `1` 时，输出每个请求的完整请求头日志 |

### 持久化存储布局

所有数据存储在用户主目录下的 `.antigravity_proxy_manager/`（各平台通用）：

```
~/.antigravity_proxy_manager/
├── accounts/                          # 每个账号一个 JSON 文件（以邮箱命名）
│   └── user_at_example.com.json
├── providers.json                     # 第三方供应商配置列表
├── token_stats.json                   # Token 用量统计（每 10 次请求写盘）
├── ag-proxy-ca.crt                    # 自签名 CA 证书（PEM 格式）
├── ag-proxy-ca.key                    # CA 私钥（PEM 格式）
├── routing_strategy.txt               # fill | round-robin
├── quota_threshold.txt                # 0–80
├── header_passthrough.txt             # 1 | 0
├── transport_mode.txt                 # legacy | client_gateway
├── upstream_server.txt                # sandbox | custom
├── upstream_custom_url.txt            # 自定义上游 URL
├── http_protocol_mode.txt             # auto | http1 | http2 | http10
├── capacity_failover_enabled.txt      # 1 | 0
├── official_ls_enabled.txt            # 1 | 0
└── google_client_secret.txt           # 可选：自定义 OAuth 客户端密钥
```

### 账号文件格式

```json
{
  "id": null,
  "email": "user@example.com",
  "project": "my-gcp-project-id",
  "refresh_token": "1//0g...",
  "access_token": "ya29...",
  "expiry_timestamp": 1709999999,
  "disabled": false,
  "disabled_reason": null,
  "disabled_at": null,
  "quota_error": null
}
```

---

## 📄 API 参考（Tauri IPC 命令）

前端通过 `invoke('command_name', args)` 调用所有后端功能。

### 账号管理

| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `load_credentials` | — | `Account[]` | 从磁盘同步加载所有账号 |
| `load_credentials_stream` | `run_id: u64` | `u64` | 后台流式加载，逐条发送 `accounts-load-progress` 事件 |
| `switch_account` | `index: i32` | `String` | 设置当前活跃账号（-1 表示无选中） |
| `delete_account` | `index: i32` | `String` | 删除账号（内存 + 磁盘文件同步删除） |
| `import_credential_files` | — | `i32` | 打开文件/目录选择器，批量导入 JSON 凭证，返回成功导入数量 |
| `import_refresh_token` | `refresh_token: String` | `String` | 通过 Refresh Token 字符串导入账号 |
| `start_oauth_login` | — | `String` | 打开浏览器完成 Google OAuth2 PKCE 授权，本地 19876 端口接收回调 |
| `toggle_account_disabled` | `index: i32, disabled: bool` | `Account[]` | 启用/禁用账号，返回更新后的全部账号列表 |

### 代理控制

| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `start_proxy` | — | `String` | 启动本地 HTTPS 代理服务器（生成/加载 TLS 证书） |
| `stop_proxy` | — | `String` | 通过 oneshot Channel 优雅停止代理 |
| `save_port_config` | `proxy_port: u16` | `String` | 持久化端口设置 |
| `set_routing_strategy` | `strategy: String` | `String` | `"fill"` 或 `"round-robin"` |
| `get_routing_strategy` | — | `String` | — |
| `set_quota_threshold` | `threshold: i32` | `String` | 0–80（仅 fill 模式有效） |
| `get_quota_threshold` | — | `i32` | — |
| `set_header_passthrough` | `enabled: bool` | `String` | — |
| `get_header_passthrough` | — | `bool` | — |
| `set_upstream_server` | `server: String` | `String` | `"sandbox"` 或 `"custom"` |
| `get_upstream_server` | — | `String` | — |
| `set_upstream_custom_url` | `url: String` | `String` | 自定义上游基础 URL |
| `get_upstream_custom_url` | — | `String` | — |
| `set_http_protocol_mode` | `mode: String` | `String` | `"auto"` / `"http1"` / `"http2"` / `"http10"` |
| `get_http_protocol_mode` | — | `String` | — |
| `set_capacity_failover_enabled` | `enabled: bool` | `String` | — |
| `get_capacity_failover_enabled` | — | `bool` | — |

### 配额 & Token 统计

| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `fetch_quota` | `index: i32` | `QuotaData` | 查询单个账号配额（包含每个模型组的百分比） |
| `fetch_all_quotas` | — | `[String, QuotaData][]` | 并发批量查询（最多 5 路并发，Semaphore 限制） |
| `get_token_stats` | — | `GlobalStats` | 全局 + 每账号 Token 用量汇总 |
| `reset_token_stats` | — | `String` | 清空所有统计数据并写盘 |
| `flush_token_stats` | — | `String` | 强制将内存统计数据写入磁盘 |

### 供应商管理

| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `save_providers` | `providers: String`（JSON 数组） | `String` | 持久化供应商列表 |
| `load_saved_providers` | — | `String`（JSON 数组） | 从 `providers.json` 加载供应商列表 |

### 工具

| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `apply_patch` | `target_url: String` | `String` | 对 IDE JS 文件打补丁（自动备份为 `.js.bak`） |
| `remove_patch` | — | `String` | 从 `.js.bak` 备份还原 IDE JS 文件 |
| `check_patch_status` | — | `PatchStatus` | 检查补丁是否已应用及当前代理地址 |
| `import_cert` | — | `String` | 将 CA 证书安装到 Windows 系统信任库（需管理员） |
| `remove_cert` | — | `String` | 从 Windows 系统信任库删除 CA 证书（需管理员） |
| `check_cert_status` | — | `CertStatus` | 检查证书安装状态 |

### LS Bridge（网关模式）

| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `set_official_ls_enabled` | `enabled: bool` | `String` | 启用/禁用网关模式 |
| `get_official_ls_enabled` | — | `bool` | — |
| `check_official_ls_binary` | — | `bool` | 检查 LS 二进制文件是否可发现 |
| `start_official_ls` | — | `String` | 启动 LS 进程 + Extension Server |
| `stop_official_ls` | — | `String` | 终止 LS 进程（发送 SIGKILL） |
| `get_official_ls_status` | — | `HashMap<String, String>` | 运行状态 + HTTPS 端口信息 |

---

## 📡 后端事件

| 事件名 | 载荷类型 | 说明 |
|---|---|---|
| `log-event` | `LogPayload` | 日志消息（info / success / warning / error / dim） |
| `request-flow` | `RequestFlowPayload` | 每次请求的逐跳链路追踪数据 |
| `accounts-load-progress` | `AccountLoadProgress` | 账号流式加载进度 |
| `account-switched` | `i32` | 当前活跃账号索引变更通知 |

### 数据结构参考

```typescript
// 后端 log-event 事件
interface LogPayload {
  message: string;
  type: "info" | "success" | "warning" | "error" | "dim";
  details?: string;
}

// 请求链路追踪
interface RequestFlowPayload {
  id: string;           // UUID v4
  timestamp: string;    // HH:MM:SS
  method: string;
  path: string;
  account: string;
  mode: string;         // "proxy" | "网关"
  forward_hops: FlowHop[];
  return_hops: FlowHop[];
  final_status?: number;
  elapsed_ms: number;
  detail?: string;
}

interface FlowHop {
  node: string;         // "客户端" | "本地代理" | "网关" | "LS桥接" | "上游官方"
  status?: number;
  detail?: string;
}

// 账号加载进度
interface AccountLoadProgress {
  account?: Account;    // 当前加载的账号（done=true 时为空）
  loaded: number;
  total: number;
  done: boolean;
  run_id: number;       // 用于忽略过期的加载流
}

// 账号数据结构
interface Account {
  id?: string;
  email: string;
  project: string;
  refresh_token: string;
  access_token: string;
  expiry_timestamp: number;
  disabled: boolean;
  disabled_reason?: string;
  disabled_at?: number;
  quota_error?: {
    kind?: string;
    code?: number;
    message: string;
    timestamp: number;
  };
}

// 配额数据
interface QuotaData {
  models: ModelQuota[];
  last_updated: number;
  is_forbidden: boolean;
}

interface ModelQuota {
  name: string;         // "Claude" | "Gemini Pro" | "Gemini Flash" | "Gemini Image"
  percentage: number;   // 0–100
  reset_time: string;
}

// 第三方供应商
interface AiProvider {
  name: string;
  base_url: string;
  api_key: string;
  protocol: "openai" | "gemini" | "claude";
  model_map: Record<string, string>;  // 源模型名 -> 供应商模型名
  enabled: boolean;
}
```

---

## 🔐 安全说明

| 方面 | 实现 |
|---|---|
| **TLS 证书** | 自签名 CA 由 `rcgen` 生成，有效期 2024–2034，存储于 `~/.antigravity_proxy_manager/` |
| **凭证存储** | OAuth Refresh Token 以明文 JSON 存储；请勿共享账号目录 |
| **API Key** | 供应商 API Key 存储于 `providers.json`，UI 中脱敏显示 |
| **OAuth 密钥** | 运行时解析：环境变量 → `google_client_secret.txt` → 内置兜底值 |
| **TLS 实现** | 全程使用 `rustls`（纯 Rust），零 OpenSSL 依赖 |
| **CSP** | `tauri.conf.json` 中 CSP 设置为 `null`，用于 Tauri IPC 兼容性 |

---

## 🌍 国际化

应用支持**中文**和**英文**界面，在 设置 → 外观 → 语言 中切换，重启后生效。

- `UI_STATIC_TEXT` 字典：约 70+ 条，按 HTML 元素 ID 绑定，通过 `applyUiLanguage()` 批量应用
- `uiText(zh, en)` 辅助函数：用于 Toast、状态消息等动态文本
- 所有代码注释（HTML / CSS / JS / Rust）均使用英文，便于国际贡献者参与

---

## 🛠️ 开发指南

### 项目脚本

```bash
# 开发模式（前端热重载 + Rust 编译）
npx tauri dev

# 生产构建
npx tauri build

# 检查文件编码一致性
npm run check:encoding

# 检查 i18n 翻译覆盖率
npm run check:i18n
```

### 关键设计决策

1. **无前端构建步骤** — 纯 HTML/CSS/JS 通过 `frontendDist: ../src` 直接提供，零打包器开销
2. **`Arc<Mutex<T>>` 共享状态** — 所有 `AppState` 字段均用 `Arc<Mutex<T>>` 包装，临界区尽量短小
3. **Oneshot Channel 优雅停止** — `proxy_shutdown_tx: Mutex<Option<oneshot::Sender<()>>>` 实现干净的代理停止
4. **流式账号加载** — `load_credentials_stream` 后台逐条发送事件，大量凭证不阻塞 UI
5. **`OnceLock<Regex>` 静态正则** — `proxy.rs` 中编译的正则表达式以 `OnceLock` 存储，所有请求复用
6. **双传输自动回退** — 网关模式始终附带直连上游作为备用目标链
7. **错误类型分类系统** — `proxy_error.rs` 定义结构化错误种类（`auth_invalid_grant`、`rate_limited` 等），跨模块一致使用

### 新增后端命令步骤

1. 在对应的 `src-tauri/src/*.rs` 模块中实现函数，加 `#[tauri::command]` 标注
2. 在 `lib.rs` 的 `tauri::generate_handler![...]` 中注册
3. 在前端调用：`await invoke('command_name', { arg1, arg2 })`

---

## 🤝 参与贡献

欢迎各种形式的贡献！

1. Fork 本仓库
2. 创建特性分支：`git checkout -b feature/my-feature`
3. 提交改动：`git commit -m 'feat: add my feature'`
4. 推送分支：`git push origin feature/my-feature`
5. 提交 Pull Request

**注意事项：**
- 所有代码注释和 Commit 消息请使用英文
- 新增 UI 文字必须通过 `uiText(zh, en)` 或 `UI_STATIC_TEXT` 提供双语版本
- 提交前运行 `npm run check:encoding` 和 `npm run check:i18n`

---

## � 致谢

本项目在设计和实现过程中参考了以下开源项目，在此致谢：

| 项目 | 说明 |
|---|---|
| [CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI) | 提供了 CLI 代理接口设计思路，OAuth 凭证管理与轮换策略参考 |
| [cockpit-tools](https://github.com/jlcodes99/cockpit-tools) | 提供了配额可视化与账号管理面板的 UI/UX 设计参考 |
| [Antigravity-Manager](https://github.com/lbjlaq/Antigravity-Manager) | 提供了 Antigravity IDE 集成方式与补丁机制的实现参考 |

---

## �📝 开源许可

本项目采用 [MIT License](LICENSE) 开源。
