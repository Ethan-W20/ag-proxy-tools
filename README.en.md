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
  <b>English</b> · <a href="README.md">简体中文</a>
</p>

---

**AG Proxy Manager** is a high-performance desktop application built with **Tauri v2** (Rust backend + Vanilla JS frontend). It acts as a local HTTPS proxy between your AI coding IDE and the upstream Google Cloud Code API, providing multi-account rotation, quota-aware intelligent routing, request flow tracing, and seamless IDE integration — with zero runtime external dependencies.

---

## ✨ Features

### 🔑 Multi-Account Management
- Import Google Cloud credentials via JSON file, Refresh Token, or Google OAuth2 login
- Supports single JSON, JSON array, JSONL (one JSON per line), and directory batch import
- Per-account enable/disable with reason and timestamp tracking
- Smart account selection persistence across sessions
- Progressive (streaming) account loading for large credential sets — non-blocking UI
- Automatic deduplication by email address

### 📊 Quota Monitoring
- Real-time quota querying for Claude, Gemini Pro, Gemini Flash, and Gemini Image model groups
- Visual progress bars with color-coded percentages
- Configurable quota switch threshold (0% / 20% / 40% / 60% / 80%)
- Optional auto-refresh every 2 minutes (up to 5 concurrent quota queries)
- Quota data cached locally for instant display

### 🔄 Intelligent Routing
- **Fill mode** — stick to one account until quota is depleted, then auto-switch
- **Round-robin mode** — distribute requests evenly across all enabled accounts
- **Capacity failover** — automatic complementary model switching (`claude-opus-4-6-thinking` ↔ `claude-sonnet-4-6-thinking`) on HTTP 503 `MODEL_CAPACITY_EXHAUSTED`, up to 4 retry attempts

### 🌐 Dual Transport Architecture
- **Proxy mode (Legacy)** — direct HTTPS proxy, tries 3 upstream endpoints in order: sandbox → daily → prod
- **Gateway mode** — routes through the official Language Server bridge; falls back to direct mode on failure
- Fully custom upstream URL support

### 🔧 IDE Integration
- One-click patch management for 4 IDE JS core files (apply / revert from `.js.bak` backup)
- Self-signed HTTPS CA certificate generation (valid 2024–2034) and Windows system trust store management via `certutil`
- Custom patch target URL support

### 📈 Token Usage Statistics
- Tracks input, output, cache read, and cache creation tokens per account
- Automatic parsing of Gemini (`usageMetadata`), Claude (`usage.input_tokens`), and OpenAI (`usage.prompt_tokens`) SSE formats
- Persistent storage (auto-save every 10 requests), manual reset option
- Keeps the most recent 1000 request records in memory

### 🔍 Request Flow Tracing
- Visual request chain: Client → Local Proxy → Gateway → LS Bridge → Upstream
- Per-hop status codes with clickable detail inspection
- Forward (→) and return (←) direction display

### 🎨 Modern UI
- Dark / Light / System theme support
- Chinese / English bilingual interface (Settings → Appearance → Language)
- Responsive sidebar navigation with 7 functional pages
- Real-time toast notifications + smooth animations and glass-morphism design

### 🛡️ AI Provider Management
- Configure third-party AI providers (OpenAI / Gemini / Claude protocols)
- Model name mapping (e.g., map `gemini-2.5-flash` → `gpt-4o`) with automatic protocol conversion
- Per-provider enable/disable with API key masking
- Up to 3 retries with exponential backoff on provider failures

---

## 🏗️ Architecture Overview

### Project Structure

```
ag-proxy-tools/
├── src/                          # Frontend (Vanilla JS + HTML + CSS)
│   ├── index.html                # Application layout (~950 lines)
│   ├── main.js                   # State management & IPC (~2800 lines)
│   ├── styles.css                # Complete design system (~2700 lines)
│   └── assets/                   # Static assets
│
├── src-tauri/                    # Backend (Rust + Tauri v2)
│   ├── tauri.conf.json           # Tauri application configuration
│   └── src/
│       ├── main.rs               # Entry point
│       ├── lib.rs                # Tauri plugin registration & AppState init
│       ├── models.rs             # Core data structure definitions
│       ├── proxy.rs              # HTTPS proxy engine (~3300 lines)
│       ├── proxy_error.rs        # Error classification & account blocking logic
│       ├── account.rs            # Account CRUD, OAuth2 flow, streaming loader
│       ├── quota.rs              # Quota fetching & caching (concurrent)
│       ├── provider.rs           # Third-party AI provider routing & protocol conversion
│       ├── ls_bridge.rs          # Official Language Server bridge (~1050 lines)
│       ├── cert.rs               # TLS CA generation & system trust store ops
│       ├── patch.rs              # IDE JS binary patching
│       ├── token_stats.rs        # Token usage statistics manager
│       ├── protobuf.rs           # Custom lightweight protobuf encoder/decoder
│       ├── constants.rs          # API endpoints & OAuth configuration
│       └── utils.rs              # Shared utilities
│
├── scripts/                      # Build & maintenance scripts
│   ├── check-encoding.js         # Verify file encoding consistency
│   ├── check-i18n-coverage.js    # Check i18n translation coverage
│   └── round-maintenance.ps1     # Automated maintenance runner
│
└── package.json
```

### Technology Stack

| Layer | Technology | Version | Purpose |
|---|---|---|---|
| **Frontend** | Vanilla JS + HTML5 + CSS3 | — | UI rendering, state management, IPC |
| **Backend** | Rust (Tokio async runtime) | 1.75+ | Proxy server, OAuth, file I/O, crypto |
| **Framework** | Tauri | v2 | Desktop packaging, IPC bridge, system API |
| **HTTP Client** | reqwest + rustls | 0.12 | Async HTTP/1.1 & HTTP/2 + TLS (pure Rust) |
| **HTTP Server** | hyper + axum + hyper-util | 1 / 0.7 / 0.1 | HTTPS proxy server & local gateway |
| **TLS** | rustls + rcgen + tokio-rustls | 0.23 / 0.13 / 0.26 | CA generation, HTTPS interception |
| **Serialization** | serde + serde_json | 1 | JSON (de)serialization |
| **Regex** | regex | 1 | URL replacement, project ID extraction |
| **Protobuf** | Custom encoder (`protobuf.rs`) | — | LS bridge protocol, no codegen deps |
| **File Picker** | rfd | 0.15 | Native file/directory picker dialog |

---

## 🔄 Data Flow & Request Processing

```
                           ┌─────────── AG Proxy Manager ──────────────┐
                           │                                            │
IDE Request ──HTTPS──────► │  Local Proxy (port 9527)                  │
                           │         │                                  │
                           │         ├──[Provider Route]──► 3rd-party AI│
                           │         │                                  │
                           │         ├──[Proxy Mode]──────────────────►│
                           │         │   sandbox.googleapis.com (1st)   │
                           │         │   daily-cloudcode.googleapis.com │
                           │         │   cloudcode-pa.googleapis.com    │
                           │         │                                  │
                           │         └──[Gateway Mode]                  │
                           │                  │                         │
                           │          Extension Server (local)          │
                           │                  │                         │
                           │          Official LS Process (random HTTPS)│
                           └──────────────────┼─────────────────────────┘
                                              │
                                              ▼
                                cloudcode-pa.googleapis.com
```

### Request Processing Pipeline (8 steps)

1. **TLS Termination** — The local proxy presents a self-signed CA certificate and decrypts the IDE's HTTPS request (port 9527).
2. **Account Selection** — `pick_account_index()` selects an eligible account based on routing strategy and quota cache.
3. **Token Refresh** — `get_valid_token_for_index()` refreshes expired tokens proactively (5-minute buffer before expiry).
4. **Request Rewriting** — Replaces `"name":"projects/"` and similar empty-project-ID placeholders with the account's real GCP project ID via a 6-stage regex pipeline.
5. **Provider Lookup** — Matches the request's model name against third-party provider mappings; routes to that provider with protocol conversion if matched.
6. **Upstream Forwarding** — Proxy mode: tries up to 3 upstream endpoints. Gateway mode: forwards to LS bridge port, falls back to direct on failure.
7. **Response Streaming** — SSE responses are piped back via `StreamBody`; token usage metadata is extracted concurrently.
8. **Capacity Failover** — On HTTP 503 `MODEL_CAPACITY_EXHAUSTED`, retries with the complementary model (up to 4 total attempts).

### Upstream Endpoint Order

| Priority | Hostname | Description |
|---|---|---|
| 1 (primary) | `daily-cloudcode-pa.sandbox.googleapis.com` | Sandbox environment — most stable |
| 2 (fallback) | `daily-cloudcode-pa.googleapis.com` | Daily build environment |
| 3 (last resort) | `cloudcode-pa.googleapis.com` | Production environment |

---

## 🔐 Account Blocking Logic

| Condition | Blocked? | Duration | Triggered by |
|---|---|---|---|
| `disabled = true` (manual) | ✅ Yes | Until manually re-enabled | User action |
| `invalid_grant` OAuth error | ✅ Permanent | Until manually re-enabled | **Quota refresh** |
| Token refresh `Bad Request` | ✅ Temporary | 5 minutes | Proxy request |
| Quota cache `is_forbidden = true` | ✅ Yes | Until quota re-fetched | **Quota refresh** |
| HTTP 401 / 403 / 429 from upstream | ❌ **No** | — | Proxy request |
| Any other non-2xx from upstream | ❌ **No** | — | Proxy request |

> **Key design principle**: Only authentication failures _during quota refresh_ trigger account blocking. Errors encountered during request _forwarding_ are considered platform-side issues and never affect account status.

### Token Refresh Blocking Detail

`Bad Request` errors are stored in `quota_error` with a timestamp. `is_account_blocked()` automatically unblocks the account 5 minutes later — no manual intervention needed.

---

## 🔒 IDE Patch Internals

The patch command modifies these 4 IDE JS files (backed up as `.js.bak` before modification):

| File | Location | Purpose |
|---|---|---|
| `vs/workbench/api/node/extensionHostProcess.js` | `{IDE}/out/` | Extension Host process |
| `vs/workbench/api/worker/extensionHostWorkerMain.js` | `{IDE}/out/` | Worker Extension Host |
| `main.js` | `{IDE}/` | Main process entry |
| `vs/code/node/cliProcessMain.js` | `{IDE}/out/` | CLI process entry |

**Patch operations:**
1. Replace all upstream URLs matching `https://(.*cloudcode.*\.com|...)` with the proxy address
2. Inject `process.env.NODE_TLS_REJECT_UNAUTHORIZED='0';` at file head (suppresses self-signed cert rejection)
3. Existing `.js.bak` backups are never overwritten (original file is protected)

**IDE auto-detection paths (Windows):**
- `%LOCALAPPDATA%\Programs\Antigravity`
- `%ProgramFiles%\Antigravity`

---

## 🔗 Language Server Bridge Internals

Gateway mode implements the official LS Extension Server protocol:

1. **Start Extension Server** — Listens on a random TCP port for LS callbacks
2. **Launch LS process** — Passes `--extension_server_host=localhost:<port>`, writes Protobuf `Metadata` to stdin
3. **Port discovery** — Waits up to 60 seconds for `LanguageServerStarted` callback to learn the LS HTTPS port
4. **OAuth token injection** — Responds to `SubscribeToUnifiedStateSyncTopic` with a long-held chunked streaming connection, pushing live Access Tokens
5. **Request forwarding** — Proxy forwards IDE requests to the LS's local HTTPS port; LS forwards to official upstream API

**Extension Server stub endpoints implemented:**
- `LanguageServerStarted` — discovers LS HTTPS port
- `SubscribeToUnifiedStateSyncTopic` — long-poll OAuth token injection
- `IsAgentManagerEnabled`, `GetChromeDevtoolsMcpUrl`, and ~10 other LS probe endpoints

---

## 📦 Prerequisites

| Requirement | Version | Install |
|---|---|---|
| Rust + cargo | 1.75+ | [rustup.rs](https://rustup.rs/) |
| Node.js + npm | 18+ | [nodejs.org](https://nodejs.org/) |
| Tauri CLI | v2 | `npm install -g @tauri-apps/cli@^2` |
| OS | Windows 10/11 (primary) | Linux/macOS: additional cert setup required |

> **Windows note**: `import_cert` / `remove_cert` require administrator privileges. The proxy server itself does not.

---

## 🚀 Getting Started

### Clone & Install

```bash
git clone https://github.com/your-username/ag-proxy-tools.git
cd ag-proxy-tools
npm install
```

### Development Mode

```bash
npx tauri dev
```

Frontend hot-reload is active. Rust changes require recompilation.

### Production Build

```bash
npx tauri build
```

Windows NSIS installer output path:
```
src-tauri/target/release/bundle/nsis/ag-proxy-tools_*.exe
```

---

## ⚙️ Configuration Reference

### Proxy Settings

| Setting | Default | Values | Description |
|---|---|---|---|
| Proxy Port | `9527` | 1024–65535 | Local HTTPS listen port |
| Routing Strategy | `fill` | `fill` / `round-robin` | Account routing mode |
| Quota Threshold | `0` | 0/20/40/60/80 | Switch account when quota % drops below this (fill mode only) |
| Header Passthrough | `on` | `on` / `off` | Forward original IDE request headers upstream |
| Transport Mode | `legacy` | `legacy` / `client_gateway` | Proxy mode vs. Gateway mode |
| Upstream Server | `sandbox` | `sandbox` / `custom` | Official upstream chain or custom URL |
| HTTP Protocol | `auto` | `auto` / `http1` / `http2` / `http10` | HTTP version negotiation |
| Capacity Failover | `on` | `on` / `off` | Auto-retry on MODEL_CAPACITY_EXHAUSTED |

### Environment Variables

| Variable | Description |
|---|---|
| `AG_PROXY_GOOGLE_CLIENT_SECRET` | Custom Google OAuth client secret (overrides config file) |
| `AG_PROXY_OFFICIAL_LS_BINARY_PATH` | Path to the official LS binary (overrides auto-detection) |
| `AG_PROXY_STREAM_AUTH_PASSTHROUGH` | Set to `1` to pass through the raw `Authorization` header for `streamGenerateContent` requests |
| `AG_PROXY_VERBOSE_HEADER_LOG` | Set to `1` to log full request headers for every request |

### Persistent Storage Layout

All data is stored in `~/.antigravity_proxy_manager/` (home directory, all platforms):

```
~/.antigravity_proxy_manager/
├── accounts/                          # One JSON file per account (named by email)
│   └── user_at_example.com.json
├── providers.json                     # Third-party provider configuration list
├── token_stats.json                   # Token usage statistics (auto-saved every 10 requests)
├── ag-proxy-ca.crt                    # Self-signed CA certificate (PEM)
├── ag-proxy-ca.key                    # CA private key (PEM)
├── routing_strategy.txt               # fill | round-robin
├── quota_threshold.txt                # 0–80
├── header_passthrough.txt             # 1 | 0
├── transport_mode.txt                 # legacy | client_gateway
├── upstream_server.txt                # sandbox | custom
├── upstream_custom_url.txt            # Custom upstream base URL
├── http_protocol_mode.txt             # auto | http1 | http2 | http10
├── capacity_failover_enabled.txt      # 1 | 0
├── official_ls_enabled.txt            # 1 | 0
└── google_client_secret.txt           # Optional: custom OAuth client secret
```

### Account File Format

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

## 📄 API Reference (Tauri IPC Commands)

All backend functionality is exposed as Tauri IPC commands: `invoke('command_name', args)`.

### Account Management

| Command | Arguments | Returns | Description |
|---|---|---|---|
| `load_credentials` | — | `Account[]` | Synchronously load all accounts from disk |
| `load_credentials_stream` | `run_id: u64` | `u64` | Background streaming load via `accounts-load-progress` events |
| `switch_account` | `index: i32` | `String` | Set active account index (-1 = none) |
| `delete_account` | `index: i32` | `String` | Remove account from memory + disk file |
| `import_credential_files` | — | `i32` | Open file/folder picker, batch import; returns count |
| `import_refresh_token` | `refresh_token: String` | `String` | Import account from refresh token string |
| `start_oauth_login` | — | `String` | Open browser for Google OAuth2 PKCE flow; local port 19876 receives callback |
| `toggle_account_disabled` | `index: i32, disabled: bool` | `Account[]` | Enable/disable account; returns updated account list |

### Proxy Control

| Command | Arguments | Returns | Description |
|---|---|---|---|
| `start_proxy` | — | `String` | Start local HTTPS proxy server (generates/loads TLS cert) |
| `stop_proxy` | — | `String` | Graceful shutdown via oneshot channel |
| `save_port_config` | `proxy_port: u16` | `String` | Persist port setting |
| `set_routing_strategy` | `strategy: String` | `String` | `"fill"` or `"round-robin"` |
| `get_routing_strategy` | — | `String` | — |
| `set_quota_threshold` | `threshold: i32` | `String` | 0–80 (effective in fill mode only) |
| `get_quota_threshold` | — | `i32` | — |
| `set_header_passthrough` | `enabled: bool` | `String` | — |
| `get_header_passthrough` | — | `bool` | — |
| `set_upstream_server` | `server: String` | `String` | `"sandbox"` or `"custom"` |
| `get_upstream_server` | — | `String` | — |
| `set_upstream_custom_url` | `url: String` | `String` | Custom upstream base URL |
| `get_upstream_custom_url` | — | `String` | — |
| `set_http_protocol_mode` | `mode: String` | `String` | `"auto"` / `"http1"` / `"http2"` / `"http10"` |
| `get_http_protocol_mode` | — | `String` | — |
| `set_capacity_failover_enabled` | `enabled: bool` | `String` | — |
| `get_capacity_failover_enabled` | — | `bool` | — |

### Quota & Token Statistics

| Command | Arguments | Returns | Description |
|---|---|---|---|
| `fetch_quota` | `index: i32` | `QuotaData` | Fetch quota for a single account |
| `fetch_all_quotas` | — | `[String, QuotaData][]` | Concurrent batch fetch (max 5 parallel via Semaphore) |
| `get_token_stats` | — | `GlobalStats` | Global + per-account token usage |
| `reset_token_stats` | — | `String` | Clear all statistics and write to disk |
| `flush_token_stats` | — | `String` | Force persist in-memory stats to disk |

### Provider Management

| Command | Arguments | Returns | Description |
|---|---|---|---|
| `save_providers` | `providers: String` (JSON array) | `String` | Persist provider list |
| `load_saved_providers` | — | `String` (JSON array) | Load provider list from `providers.json` |

### Tools

| Command | Arguments | Returns | Description |
|---|---|---|---|
| `apply_patch` | `target_url: String` | `String` | Patch IDE JS files (auto-backs up to `.js.bak`) |
| `remove_patch` | — | `String` | Restore IDE JS files from `.js.bak` backups |
| `check_patch_status` | — | `PatchStatus` | Check if patch is applied and current proxy address |
| `import_cert` | — | `String` | Install CA to Windows system trust store (requires admin) |
| `remove_cert` | — | `String` | Remove CA from Windows system trust store (requires admin) |
| `check_cert_status` | — | `CertStatus` | Check certificate installation status |

### LS Bridge

| Command | Arguments | Returns | Description |
|---|---|---|---|
| `set_official_ls_enabled` | `enabled: bool` | `String` | Enable/disable Gateway mode |
| `get_official_ls_enabled` | — | `bool` | — |
| `check_official_ls_binary` | — | `bool` | Check if LS binary is discoverable |
| `start_official_ls` | — | `String` | Launch LS process + Extension Server |
| `stop_official_ls` | — | `String` | Kill LS process |
| `get_official_ls_status` | — | `HashMap<String, String>` | Running status + HTTPS port info |

---

## 📡 Backend Events

| Event | Payload Type | Description |
|---|---|---|
| `log-event` | `LogPayload` | Log messages (info / success / warning / error / dim) |
| `request-flow` | `RequestFlowPayload` | Per-request hop-by-hop flow trace data |
| `accounts-load-progress` | `AccountLoadProgress` | Progressive account loading progress |
| `account-switched` | `i32` | Active account index change notification |

### Data Structure Reference

```typescript
// log-event payload
interface LogPayload {
  message: string;
  type: "info" | "success" | "warning" | "error" | "dim";
  details?: string;
}

// Request flow tracing
interface RequestFlowPayload {
  id: string;           // UUID v4
  timestamp: string;    // "HH:MM:SS"
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

// Account loading progress
interface AccountLoadProgress {
  account?: Account;    // Current account being loaded (absent when done=true)
  loaded: number;
  total: number;
  done: boolean;
  run_id: number;       // Use to discard stale loading streams
}

// Account object
interface Account {
  id?: string;
  email: string;
  project: string;
  refresh_token: string;
  access_token: string;
  expiry_timestamp: number;
  disabled: boolean;
  disabled_reason?: string;
  disabled_at?: number;         // Unix timestamp
  quota_error?: {
    kind?: string;              // "auth_invalid_grant" | "rate_limited" | ...
    code?: number;
    message: string;
    timestamp: number;
  };
}

// Quota data
interface QuotaData {
  models: ModelQuota[];
  last_updated: number;         // Unix timestamp
  is_forbidden: boolean;
}

interface ModelQuota {
  name: string;                 // "Claude" | "Gemini Pro" | "Gemini Flash" | "Gemini Image"
  percentage: number;           // 0–100
  reset_time: string;
}

// Third-party AI provider
interface AiProvider {
  name: string;
  base_url: string;
  api_key: string;
  protocol: "openai" | "gemini" | "claude";
  model_map: Record<string, string>;  // source model name -> provider model name
  enabled: boolean;
}

// Patch status
interface PatchStatus {
  applied: boolean;
  message: string;   // e.g. "本地模式 (127.0.0.1:9527)" | "未应用补丁"
}

// Certificate status
interface CertStatus {
  installed: boolean;
  cert_path: string;
}
```

---

## 🔐 Security Notes

| Aspect | Implementation |
|---|---|
| **TLS** | Self-signed CA generated with `rcgen`, valid 2024–2034, stored in `~/.antigravity_proxy_manager/` |
| **Credentials** | OAuth refresh tokens stored as plain JSON; do not share your `accounts/` directory |
| **API Keys** | Provider API keys stored in `providers.json`; masked in UI |
| **OAuth Secret** | Resolved at runtime: `AG_PROXY_GOOGLE_CLIENT_SECRET` env var → `google_client_secret.txt` → built-in fallback |
| **TLS implementation** | Uses `rustls` exclusively — zero OpenSSL dependency |
| **CSP** | Intentionally `null` in `tauri.conf.json` for Tauri IPC compatibility |

---

## 🌍 Internationalization

The application supports **Chinese (zh)** and **English (en)**, switchable in Settings → Appearance → Language.

- `UI_STATIC_TEXT` dictionary — 70+ entries keyed by HTML element ID, applied via `applyUiLanguage()`
- `uiText(zh, en)` helper — used for dynamic strings (toasts, status messages, dialogs)
- All code comments (HTML, CSS, JS, Rust) are in English for international contributors

---

## 🛠️ Development Guide

### Project Scripts

```bash
npx tauri dev           # Development with hot-reload
npx tauri build         # Production build
npm run check:encoding  # Verify file encoding consistency
npm run check:i18n      # Check i18n translation coverage
```

### Key Design Decisions

1. **No frontend build step** — Plain HTML/CSS/JS served via `frontendDist: ../src`. Zero bundler overhead.
2. **`Arc<Mutex<T>>` for shared state** — All `AppState` fields use `Arc<Mutex<T>>`. Critical sections are kept minimal.
3. **Oneshot channel for proxy shutdown** — `proxy_shutdown_tx: Mutex<Option<oneshot::Sender<()>>>` enables clean stop without process kill.
4. **Progressive account loading** — `load_credentials_stream` spawns a background task and emits per-account events, never blocking UI.
5. **`OnceLock<Regex>` for compiled regexes** — Regexes in `proxy.rs` are compiled once and reused across all requests.
6. **Dual transport with automatic fallback** — Gateway mode always appends direct upstream endpoints as fallback targets.
7. **Structured error taxonomy** — `proxy_error.rs` defines error kinds (`auth_invalid_grant`, `rate_limited`, etc.) used consistently for account blocking decisions and UI display.

### Adding a New Backend Command

1. Implement the function in the appropriate `src-tauri/src/*.rs` module with `#[tauri::command]`
2. Register it in `lib.rs` inside `tauri::generate_handler![...]`
3. Call from the frontend: `await invoke('command_name', { arg1, arg2 })`

---

## 🤝 Contributing

Contributions are welcome!

1. Fork the repository
2. Create a feature branch: `git checkout -b feature/my-feature`
3. Commit your changes: `git commit -m 'feat: add my feature'`
4. Push to the branch: `git push origin feature/my-feature`
5. Open a Pull Request

**Guidelines:**
- All code comments and commit messages must be in English
- New UI text strings must provide both Chinese and English via `uiText(zh, en)` or `UI_STATIC_TEXT`
- Run `npm run check:encoding` and `npm run check:i18n` before submitting

---

## 🙏 Acknowledgements

This project was inspired by and references the following open-source projects:

| Project | Contribution |
|---|---|
| [CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI) | CLI proxy interface design, OAuth credential management and rotation strategy |
| [cockpit-tools](https://github.com/jlcodes99/cockpit-tools) | UI/UX design reference for quota visualization and account management dashboard |
| [Antigravity-Manager](https://github.com/lbjlaq/Antigravity-Manager) | Implementation reference for Antigravity IDE integration and patch mechanisms |

---

## 📝 License

This project is licensed under the [MIT License](LICENSE).
