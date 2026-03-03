// Official Language Server bridge.
//
// Implements the Extension Server protocol that the official LS expects,
// including:
//   - LanguageServerStarted callback (port discovery)
//   - SubscribeToUnifiedStateSyncTopic (OAuth token injection)
//   - Various stub endpoints the LS probes at startup
//
// Architecture:
//   1. Start a local TCP server (Extension Server) on a random port
//   2. Launch the official LS binary with arguments pointing to Extension Server
//   3. Write protobuf Metadata to LS stdin
//   4. Wait for LS to call back LanguageServerStarted with its ports
//   5. Proxy then forwards requests to LS local HTTPS port

use base64::{engine::general_purpose, Engine as _};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::command;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Mutex as TokioMutex, Notify};
use tokio::time::{timeout, Duration};

use crate::protobuf;
use crate::utils::{get_antigravity_base_path, get_app_data_dir};

// ==================== Constants ====================

const LS_BINARY_ENV_KEY: &str = "AG_PROXY_OFFICIAL_LS_BINARY_PATH";
const LS_ENABLED_CONFIG_FILE: &str = "official_ls_enabled.txt";
const LS_START_TIMEOUT: Duration = Duration::from_secs(60);
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HTTP_REQUEST_BYTES: usize = 512 * 1024;
const CLOUD_CODE_PROD: &str = "https://cloudcode-pa.googleapis.com";
const DEFAULT_APP_DATA_DIR: &str = "antigravity";
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ==================== Static state ====================

struct OfficialLsRuntime {
    process: TokioMutex<Option<OfficialLsProcessHandle>>,
    last_error: Mutex<Option<String>>,
}

static LS_RUNTIME: OnceLock<OfficialLsRuntime> = OnceLock::new();

fn runtime() -> &'static OfficialLsRuntime {
    LS_RUNTIME.get_or_init(|| OfficialLsRuntime {
        process: TokioMutex::new(None),
        last_error: Mutex::new(None),
    })
}

fn set_last_error(err: Option<String>) {
    if let Ok(mut guard) = runtime().last_error.lock() {
        *guard = err;
    }
}

// ==================== LS auto-discovery ====================


fn discover_official_ls_binary_from_installed_ide() -> Option<String> {
    // Collect candidate IDE installation roots
    let mut ide_roots: Vec<PathBuf> = Vec::new();

    // 1. Known default install locations
    #[cfg(target_os = "windows")]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            ide_roots.push(PathBuf::from(&local_app_data).join("Programs").join("Antigravity"));
        }
        if let Ok(prog_files) = std::env::var("ProgramFiles") {
            ide_roots.push(PathBuf::from(prog_files).join("Antigravity"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        ide_roots.push(PathBuf::from("/Applications/Antigravity.app"));
        if let Ok(home) = std::env::var("HOME") {
            ide_roots.push(PathBuf::from(&home).join("Applications").join("Antigravity.app"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(home) = std::env::var("HOME") {
            ide_roots.push(PathBuf::from(&home).join(".local").join("share").join("Antigravity"));
        }
        ide_roots.push(PathBuf::from("/usr/share/antigravity"));
        ide_roots.push(PathBuf::from("/opt/antigravity"));
    }

    // 2. Walk up from current executable to find IDE root
    if let Ok(exe_path) = std::env::current_exe() {
        let mut dir = exe_path.as_path();
        for _ in 0..8 {
            if let Some(parent) = dir.parent() {
                dir = parent;
                ide_roots.push(dir.to_path_buf());
            } else {
                break;
            }
        }
    }

    // 3. Walk up from get_antigravity_base_path
    if let Some(base) = get_antigravity_base_path() {
        let mut dir = base.as_path();
        for _ in 0..5 {
            ide_roots.push(dir.to_path_buf());
            if let Some(parent) = dir.parent() {
                dir = parent;
            } else {
                break;
            }
        }
    }

    // Build candidate bin directories from each IDE root
    let mut bin_dirs: Vec<PathBuf> = Vec::new();
    for root in &ide_roots {
        // Standard packaged layout: <root>/resources/app/extensions/antigravity/bin
        bin_dirs.push(root.join("resources").join("app").join("extensions").join("antigravity").join("bin"));
        // Direct extensions layout: <root>/extensions/antigravity/bin
        bin_dirs.push(root.join("extensions").join("antigravity").join("bin"));
        #[cfg(target_os = "macos")]
        bin_dirs.push(root.join("Contents").join("Resources").join("app").join("extensions").join("antigravity").join("bin"));
    }

    // Deduplicate and filter to existing dirs
    bin_dirs.sort();
    bin_dirs.dedup();
    let bin_dirs: Vec<PathBuf> = bin_dirs.into_iter().filter(|d| d.is_dir()).collect();

    if bin_dirs.is_empty() {
        return None;
    }

    #[cfg(target_os = "windows")]
    let preferred = [
        "language_server_windows_x64.exe",
        "language_server_windows_arm64.exe",
        "language_server_windows.exe",
    ];
    #[cfg(target_os = "macos")]
    let preferred = [
        "language_server_macos_arm",
        "language_server_macos_x64",
        "language_server_macos",
        "language_server_darwin_arm64",
        "language_server_darwin_x64",
        "language_server_darwin",
        "language_server",
    ];
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let preferred = [
        "language_server_linux_x64",
        "language_server_linux_arm64",
        "language_server_linux",
        "language_server",
    ];

    for bin_dir in &bin_dirs {
        for name in &preferred {
            let candidate = bin_dir.join(name);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }

        // Dynamic fallback: scan directory
        if let Ok(entries) = fs::read_dir(bin_dir) {
            let mut fallback: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .filter(|p| {
                    let Some(name) = p.file_name().and_then(|v| v.to_str()) else {
                        return false;
                    };
                    let lower = name.to_ascii_lowercase();
                    if !lower.starts_with("language_server") {
                        return false;
                    }
                    #[cfg(target_os = "windows")]
                    if !lower.ends_with(".exe") {
                        return false;
                    }
                    true
                })
                .collect();
            fallback.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
            if let Some(first) = fallback.into_iter().next() {
                return Some(first.to_string_lossy().to_string());
            }
        }
    }

    None
}

fn official_ls_binary_path() -> Result<String, String> {
    // 1. Environment variable override
    if let Ok(v) = std::env::var(LS_BINARY_ENV_KEY) {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if path.is_file() {
                return Ok(trimmed.to_string());
            }
        }
    }

    // 2. Auto-discover from installed IDE
    if let Some(path) = discover_official_ls_binary_from_installed_ide() {
        return Ok(path);
    }

    Err(
        "官方 LS 二进制文件未找到。请确认已安装官方 Antigravity IDE，或设置环境变量 AG_PROXY_OFFICIAL_LS_BINARY_PATH"
            .to_string(),
    )
}

pub fn is_official_ls_binary_available() -> bool {
    official_ls_binary_path().is_ok()
}

// ==================== Extension Server protocol ====================

#[derive(Debug, Clone, Copy)]
struct LsStartedInfo {
    https_port: u16,
    _http_port: u16,
    _lsp_port: u16,
}

struct ExtensionServerState {
    csrf_token: String,
    uss_oauth_topic_bytes: Vec<u8>,
    empty_topic_bytes: Vec<u8>,
    started_sender: Mutex<Option<oneshot::Sender<LsStartedInfo>>>,
    shutdown_notify: Arc<Notify>,
}

struct ExtensionServerHandle {
    port: u16,
    csrf_token: String,
    started_receiver: oneshot::Receiver<LsStartedInfo>,
    shutdown_notify: Arc<Notify>,
    _task: tokio::task::JoinHandle<()>,
}

struct OfficialLsProcessHandle {
    child: Child,
    _stdout_task: Option<tokio::task::JoinHandle<()>>,
    _stderr_task: Option<tokio::task::JoinHandle<()>>,
    extension_server_shutdown: Arc<Notify>,
    started: LsStartedInfo,
    _ls_csrf_token: String,
}

impl OfficialLsProcessHandle {
    async fn shutdown(&mut self) {
        self.extension_server_shutdown.notify_waiters();
        let _ = self.child.start_kill();
        let _ = timeout(Duration::from_secs(2), self.child.wait()).await;
        if let Some(task) = self._stdout_task.take() {
            task.abort();
        }
        if let Some(task) = self._stderr_task.take() {
            task.abort();
        }
    }
}

// ---- HTTP parsing (raw TCP, no framework) ----

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|idx| idx + 4)
}

fn parse_content_length(header_bytes: &[u8]) -> usize {
    let text = String::from_utf8_lossy(header_bytes);
    for line in text.lines() {
        let mut parts = line.splitn(2, ':');
        let Some(name) = parts.next() else { continue };
        let Some(value) = parts.next() else { continue };
        if name.trim().eq_ignore_ascii_case("content-length") {
            return value.trim().parse::<usize>().unwrap_or(0);
        }
    }
    0
}

#[derive(Debug)]
struct ParsedRequest {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

async fn read_http_request<R: AsyncRead + Unpin>(stream: &mut R) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::with_capacity(4096);
    let mut chunk = [0u8; 2048];
    let mut header_end: Option<usize> = None;
    let mut content_length: usize = 0;

    loop {
        let bytes_read = timeout(REQUEST_READ_TIMEOUT, stream.read(&mut chunk))
            .await
            .map_err(|_| "read timeout".to_string())?
            .map_err(|e| format!("read error: {}", e))?;

        if bytes_read == 0 {
            break;
        }

        buffer.extend_from_slice(&chunk[..bytes_read]);
        if buffer.len() > MAX_HTTP_REQUEST_BYTES {
            return Err("request too large".to_string());
        }

        if header_end.is_none() {
            if let Some(end) = find_header_end(&buffer) {
                content_length = parse_content_length(&buffer[..end]);
                header_end = Some(end);
            }
        }

        if let Some(end) = header_end {
            if buffer.len() >= end.saturating_add(content_length) {
                return Ok(buffer[..(end + content_length)].to_vec());
            }
        }
    }
    Err("incomplete request".to_string())
}

fn parse_http_request(raw: &[u8]) -> Result<ParsedRequest, String> {
    let Some(header_end) = find_header_end(raw) else {
        return Err("missing header end".to_string());
    };
    let header_text = String::from_utf8_lossy(&raw[..header_end]);
    let mut lines = header_text.lines();
    let request_line = lines.next().ok_or("empty request line")?.trim();

    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or("missing method")?.to_string();
    let target = parts.next().ok_or("missing target")?.to_string();

    let mut headers = HashMap::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut hdr = line.splitn(2, ':');
        let Some(name) = hdr.next() else { continue };
        let Some(value) = hdr.next() else { continue };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }

    Ok(ParsedRequest {
        method,
        target,
        headers,
        body: raw[header_end..].to_vec(),
    })
}

fn normalize_path(target: &str) -> String {
    if let Ok(url) = url::Url::parse(&format!("http://localhost{}", target)) {
        return url.path().to_string();
    }
    target.to_string()
}

fn path_matches_rpc_method(path: &str, method_name: &str) -> bool {
    let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or(path);
    let base = last.split(':').next().unwrap_or(last);
    base == method_name
}

// ---- HTTP response builders ----

fn text_response(status_code: u16, status_text: &str, body: &str) -> Vec<u8> {
    let body_bytes = body.as_bytes();
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status_code, status_text, body_bytes.len()
    );
    let mut resp = headers.into_bytes();
    resp.extend_from_slice(body_bytes);
    resp
}

fn binary_response(status_code: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let headers = format!(
        "HTTP/1.1 {} OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status_code, content_type, body.len()
    );
    let mut resp = headers.into_bytes();
    resp.extend_from_slice(body);
    resp
}

fn extension_unary_response(content_type: &str, proto_body: &[u8]) -> Vec<u8> {
    if content_type.contains("connect") {
        // connect-protocol: length-prefixed envelope
        let mut envelope = Vec::with_capacity(5 + proto_body.len());
        envelope.push(0u8); // flags=0 (no compression)
        envelope.extend_from_slice(&(proto_body.len() as u32).to_be_bytes());
        envelope.extend_from_slice(proto_body);
        // end-of-stream envelope
        let eos = [0x02, 0, 0, 0, 2, b'{', b'}'];
        let mut full = envelope;
        full.extend_from_slice(&eos);
        binary_response(200, content_type, &full)
    } else {
        binary_response(200, if content_type.is_empty() { "application/proto" } else { content_type }, proto_body)
    }
}

fn chunked_http_stream_headers(status_code: u16, content_type: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {} OK\r\nContent-Type: {}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        status_code, content_type
    ).into_bytes()
}

fn encode_chunked_bytes(payload: &[u8]) -> Vec<u8> {
    format!("{:x}\r\n", payload.len()).into_bytes()
        .into_iter()
        .chain(payload.iter().copied())
        .chain(b"\r\n".iter().copied())
        .collect()
}

fn encode_connect_message_envelope(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + payload.len());
    out.push(0u8); // flags
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

fn encode_connect_end_ok_envelope() -> Vec<u8> {
    let mut out = Vec::with_capacity(7);
    out.push(0x02); // flags=end-stream
    out.extend_from_slice(&(2u32).to_be_bytes());
    out.extend_from_slice(b"{}");
    out
}

// ---- Protobuf message builders ----

fn build_uss_oauth_topic_bytes(
    access_token: &str,
    refresh_token: &str,
    expiry: i64,
) -> Vec<u8> {
    let oauth_info = protobuf::create_oauth_info(access_token, refresh_token, expiry);
    let oauth_info_b64 = general_purpose::STANDARD.encode(oauth_info);
    let row = protobuf::encode_string_field(1, &oauth_info_b64);
    let entry = [
        protobuf::encode_string_field(1, "oauthTokenInfoSentinelKey"),
        protobuf::encode_len_delim_field(2, &row),
    ]
    .concat();
    protobuf::encode_len_delim_field(1, &entry)
}

fn build_unified_state_sync_update(topic_bytes: &[u8]) -> Vec<u8> {
    // UnifiedStateSyncUpdate: field 1 = initial_state (Topic)
    protobuf::encode_len_delim_field(1, topic_bytes)
}

fn build_official_ls_metadata_bytes() -> Vec<u8> {
    let mut out = Vec::new();
    let push_str = |buf: &mut Vec<u8>, field_num: u32, value: &str| {
        if !value.is_empty() {
            buf.extend(protobuf::encode_string_field(field_num, value));
        }
    };

    // exa.codeium_common_pb.Metadata
    push_str(&mut out, 1, "Antigravity");       // ide_name
    push_str(&mut out, 7, "1.19.5");            // ide_version
    push_str(&mut out, 12, "antigravity");       // extension_name
    push_str(&mut out, 4, "zh-CN");             // locale
    push_str(&mut out, 24, &uuid::Uuid::new_v4().to_string()); // device fingerprint

    if out.is_empty() {
        out.extend(protobuf::encode_varint(0));
    }
    out
}

fn parse_ls_started_request(body: &[u8]) -> Result<LsStartedInfo, String> {
    let mut offset = 0usize;
    let mut https_port: Option<u16> = None;
    let mut http_port: Option<u16> = None;
    let mut lsp_port: Option<u16> = None;

    while offset < body.len() {
        let (tag, new_offset) = protobuf::read_varint(body, offset)?;
        let wire_type = (tag & 7) as u8;
        let field_num = (tag >> 3) as u32;

        match (field_num, wire_type) {
            (1, 0) => {
                let (v, end) = protobuf::read_varint(body, new_offset)?;
                https_port = u16::try_from(v).ok();
                offset = end;
                continue;
            }
            (2, 0) => {
                let (v, end) = protobuf::read_varint(body, new_offset)?;
                lsp_port = u16::try_from(v).ok();
                offset = end;
                continue;
            }
            (5, 0) => {
                let (v, end) = protobuf::read_varint(body, new_offset)?;
                http_port = u16::try_from(v).ok();
                offset = end;
                continue;
            }
            _ => {}
        }
        offset = protobuf::skip_field(body, new_offset, wire_type)?;
    }

    Ok(LsStartedInfo {
        https_port: https_port.ok_or("LanguageServerStarted missing https_port")?,
        _http_port: http_port.unwrap_or(0),
        _lsp_port: lsp_port.unwrap_or(0),
    })
}

fn parse_subscribe_topic(body: &[u8]) -> Result<String, String> {
    // Connect envelope: 1 byte flags + 4 bytes length + payload
    if body.len() < 5 {
        return Err("connect body too short".to_string());
    }
    let len = u32::from_be_bytes([body[1], body[2], body[3], body[4]]) as usize;
    let start = 5usize;
    let end = start + len;
    if end > body.len() {
        return Err("connect frame truncated".to_string());
    }
    let payload = &body[start..end];

    // Parse SubscribeToUnifiedStateSyncTopicRequest: field 1 = topic (string)
    let mut offset = 0usize;
    while offset < payload.len() {
        let (tag, new_offset) = protobuf::read_varint(payload, offset)?;
        let wire_type = (tag & 7) as u8;
        let field_num = (tag >> 3) as u32;
        if field_num == 1 && wire_type == 2 {
            let (len, content_offset) = protobuf::read_varint(payload, new_offset)?;
            let len = len as usize;
            let end = content_offset + len;
            if end > payload.len() {
                return Err("topic field truncated".to_string());
            }
            let topic = std::str::from_utf8(&payload[content_offset..end])
                .map_err(|e| format!("topic UTF-8 error: {}", e))?;
            return Ok(topic.to_string());
        }
        offset = protobuf::skip_field(payload, new_offset, wire_type)?;
    }
    Err("missing topic field".to_string())
}

// ---- Extension Server connection handler ----

enum ExtensionAction {
    Close(Vec<u8>),
    HoldStream {
        content_type: String,
        first_message: Vec<u8>,
        shutdown_notify: Arc<Notify>,
    },
}

async fn handle_extension_connection<S: AsyncRead + tokio::io::AsyncWrite + Unpin>(
    mut stream: S,
    state: Arc<ExtensionServerState>,
) {
    let action = match read_http_request(&mut stream).await {
        Ok(raw) => match parse_http_request(&raw) {
            Ok(parsed) => route_extension_request(parsed, state).await,
            Err(err) => ExtensionAction::Close(text_response(400, "Bad Request", &err)),
        },
        Err(err) => ExtensionAction::Close(text_response(400, "Bad Request", &err)),
    };

    match action {
        ExtensionAction::Close(resp) => {
            let _ = stream.write_all(&resp).await;
            let _ = stream.flush().await;
            let _ = stream.shutdown().await;
        }
        ExtensionAction::HoldStream {
            content_type,
            first_message,
            shutdown_notify,
        } => {
            let headers = chunked_http_stream_headers(200, &content_type);
            let _ = stream.write_all(&headers).await;
            let _ = stream
                .write_all(&encode_chunked_bytes(&encode_connect_message_envelope(
                    &first_message,
                )))
                .await;
            let _ = stream.flush().await;

            // Hold connection open until shutdown
            shutdown_notify.notified().await;

            let _ = stream
                .write_all(&encode_chunked_bytes(&encode_connect_end_ok_envelope()))
                .await;
            let _ = stream.write_all(b"0\r\n\r\n").await; // chunked final
            let _ = stream.flush().await;
            let _ = stream.shutdown().await;
        }
    }
}

async fn route_extension_request(
    parsed: ParsedRequest,
    state: Arc<ExtensionServerState>,
) -> ExtensionAction {
    let path = normalize_path(&parsed.target);
    let method = parsed.method.to_ascii_uppercase();
    let content_type = parsed
        .headers
        .get("content-type")
        .cloned()
        .unwrap_or_else(|| "application/proto".to_string());
    let request_csrf = parsed
        .headers
        .get("x-codeium-csrf-token")
        .cloned()
        .unwrap_or_default();

    if method == "OPTIONS" {
        return ExtensionAction::Close(text_response(200, "OK", ""));
    }
    if method != "POST" {
        return ExtensionAction::Close(text_response(405, "Method Not Allowed", "POST only"));
    }
    if request_csrf != state.csrf_token {
        return ExtensionAction::Close(text_response(403, "Forbidden", "Invalid CSRF token"));
    }

    // LanguageServerStarted — LS reports its ports
    if path_matches_rpc_method(&path, "LanguageServerStarted") {
        match parse_ls_started_request(&parsed.body) {
            Ok(started) => {
                if let Ok(mut guard) = state.started_sender.lock() {
                    if let Some(tx) = guard.take() {
                        let _ = tx.send(started);
                    }
                }
                return ExtensionAction::Close(extension_unary_response(&content_type, &[]));
            }
            Err(err) => {
                return ExtensionAction::Close(text_response(400, "Bad Request", &err));
            }
        }
    }

    // SubscribeToUnifiedStateSyncTopic — LS subscribes to OAuth token updates
    if path_matches_rpc_method(&path, "SubscribeToUnifiedStateSyncTopic") {
        let topic = match parse_subscribe_topic(&parsed.body) {
            Ok(v) => v,
            Err(err) => {
                return ExtensionAction::Close(text_response(400, "Bad Request", &err));
            }
        };

        let topic_bytes = match topic.as_str() {
            "uss-oauth" => &state.uss_oauth_topic_bytes,
            _ => &state.empty_topic_bytes,
        };
        let update = build_unified_state_sync_update(topic_bytes);

        return ExtensionAction::HoldStream {
            content_type: "application/connect+proto".to_string(),
            first_message: update,
            shutdown_notify: state.shutdown_notify.clone(),
        };
    }

    // Stub endpoints the LS probes
    if path_matches_rpc_method(&path, "IsAgentManagerEnabled") {
        let body = [
            protobuf::encode_varint((1 << 3) as u64),
            vec![1u8],
        ]
        .concat();
        return ExtensionAction::Close(extension_unary_response(&content_type, &body));
    }
    if path_matches_rpc_method(&path, "GetChromeDevtoolsMcpUrl") {
        let body = protobuf::encode_string_field(1, "");
        return ExtensionAction::Close(extension_unary_response(&content_type, &body));
    }

    // Catch-all empty OK for known probe paths
    let empty_ok_methods = [
        "CheckTerminalShellSupport",
        "GetBrowserOnboardingPort",
        "PushUnifiedStateSyncUpdate",
        "GetSecretValue",
        "StoreSecretValue",
        "LogEvent",
        "RecordError",
        "RestartUserStatusUpdater",
        "OpenSetting",
        "PlaySound",
        "BroadcastConversationDeletion",
    ];
    if empty_ok_methods.iter().any(|m| path_matches_rpc_method(&path, m) || path.ends_with(&format!("/{}", m))) {
        return ExtensionAction::Close(extension_unary_response(&content_type, &[]));
    }

    // Unknown endpoint — return empty OK to avoid blocking LS
    ExtensionAction::Close(extension_unary_response(&content_type, &[]))
}

// ---- Extension Server launcher ----

async fn start_extension_server(
    access_token: &str,
    refresh_token: &str,
    expiry: i64,
) -> Result<ExtensionServerHandle, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Extension Server bind failed: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Extension Server port read failed: {}", e))?
        .port();
    let csrf_token = uuid::Uuid::new_v4().to_string();
    let (started_sender, started_receiver) = oneshot::channel();
    let shutdown_notify = Arc::new(Notify::new());

    let state = Arc::new(ExtensionServerState {
        csrf_token: csrf_token.clone(),
        uss_oauth_topic_bytes: build_uss_oauth_topic_bytes(access_token, refresh_token, expiry),
        empty_topic_bytes: Vec::new(),
        started_sender: Mutex::new(Some(started_sender)),
        shutdown_notify: shutdown_notify.clone(),
    });

    let shutdown_clone = shutdown_notify.clone();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_clone.notified() => break,
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _)) => {
                            let s = state.clone();
                            tokio::spawn(handle_extension_connection(stream, s));
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    });

    Ok(ExtensionServerHandle {
        port,
        csrf_token,
        started_receiver,
        shutdown_notify,
        _task: task,
    })
}

// ---- LS process launcher ----

fn spawn_ls_log_task<R: AsyncRead + Unpin + Send + 'static>(
    reader: R,
    tag: &'static str,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                tracing::info!("[OfficialLS][{}] {}", tag, trimmed);
            }
        }
    })
}

/// Start the official LS process with full Extension Server protocol.
///
/// Requires a valid account token for OAuth injection.
async fn start_official_ls_process(
    access_token: &str,
    refresh_token: &str,
    expiry: i64,
) -> Result<OfficialLsProcessHandle, String> {
    let binary_path = official_ls_binary_path()?;
    let mut ext_server =
        start_extension_server(access_token, refresh_token, expiry).await?;
    let ls_csrf = uuid::Uuid::new_v4().to_string();
    let cloud_code_endpoint = CLOUD_CODE_PROD;
    let app_data_dir = DEFAULT_APP_DATA_DIR;

    let mut cmd = Command::new(&binary_path);
    cmd.arg("--enable_lsp")
        .arg("--random_port")
        .arg("--csrf_token")
        .arg(&ls_csrf)
        .arg("--extension_server_port")
        .arg(ext_server.port.to_string())
        .arg("--extension_server_csrf_token")
        .arg(&ext_server.csrf_token)
        .arg("--cloud_code_endpoint")
        .arg(cloud_code_endpoint)
        .arg("--app_data_dir")
        .arg(app_data_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.as_std_mut().creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动官方 LS 失败: {}", e))?;

    let Some(stdout) = child.stdout.take() else {
        let _ = child.start_kill();
        return Err("官方 LS stdout 不可用".to_string());
    };

    let stdout_task = spawn_ls_log_task(stdout, "stdout");
    let stderr_task = child
        .stderr
        .take()
        .map(|stderr| spawn_ls_log_task(stderr, "stderr"));

    // Write metadata to stdin
    if let Some(mut stdin) = child.stdin.take() {
        let metadata = build_official_ls_metadata_bytes();
        stdin
            .write_all(&metadata)
            .await
            .map_err(|e| format!("写入 LS Metadata 失败: {}", e))?;
        let _ = stdin.shutdown().await;
    }

    // Wait for LanguageServerStarted callback
    let started = timeout(LS_START_TIMEOUT, &mut ext_server.started_receiver)
        .await
        .map_err(|_| "等待官方 LS 启动超时 (60s)".to_string())?
        .map_err(|_| "官方 LS 启动通知通道已关闭".to_string())?;

    tracing::info!(
        "[OfficialLS] 启动完成: https_port={}",
        started.https_port
    );

    Ok(OfficialLsProcessHandle {
        child,
        _stdout_task: Some(stdout_task),
        _stderr_task: stderr_task,
        extension_server_shutdown: ext_server.shutdown_notify,
        started,
        _ls_csrf_token: ls_csrf,
    })
}

// ==================== Public API ====================

/// Returns the base URL of the running official LS, or None if not running.
pub async fn get_official_ls_https_base_url() -> Option<String> {
    let guard = runtime().process.lock().await;
    guard
        .as_ref()
        .map(|h| format!("https://127.0.0.1:{}", h.started.https_port))
}

/// Check if official LS mode is enabled (persisted setting).
pub fn is_official_ls_enabled() -> bool {
    let path = get_app_data_dir().join(LS_ENABLED_CONFIG_FILE);
    match fs::read_to_string(path) {
        Ok(v) => !matches!(v.trim().to_lowercase().as_str(), "0" | "off" | "false"),
        Err(_) => true, // default: enabled
    }
}

#[derive(Debug, Serialize)]
pub struct OfficialLsStatus {
    pub enabled: bool,
    pub running: bool,
    pub pid: Option<u32>,
    pub https_port: Option<u16>,
    pub binary_path: Option<String>,
    pub last_error: Option<String>,
}

// ==================== Tauri commands ====================

#[command]
pub fn set_official_ls_enabled(enabled: bool) -> Result<bool, String> {
    let path = get_app_data_dir().join(LS_ENABLED_CONFIG_FILE);
    let value = if enabled { "1" } else { "0" };
    fs::write(&path, value).map_err(|e| format!("保存 LS 配置失败: {}", e))?;
    Ok(enabled)
}

#[command]
pub fn get_official_ls_enabled() -> bool {
    is_official_ls_enabled()
}

#[command]
pub fn check_official_ls_binary() -> Result<String, String> {
    official_ls_binary_path()
}

#[command]
pub async fn start_official_ls(
    access_token: String,
    refresh_token: String,
    expiry: i64,
) -> Result<String, String> {
    let mut guard = runtime().process.lock().await;

    // If already running, check if still alive
    if let Some(handle) = guard.as_mut() {
        match handle.child.try_wait() {
            Ok(Some(_)) => {
                *guard = None; // exited
            }
            Ok(None) => {
                return Ok(format!(
                    "官方 LS 已在运行, port={}",
                    handle.started.https_port
                ));
            }
            Err(_) => {
                *guard = None;
            }
        }
    }

    match start_official_ls_process(&access_token, &refresh_token, expiry).await {
        Ok(handle) => {
            let port = handle.started.https_port;
            let pid = handle.child.id();
            *guard = Some(handle);
            set_last_error(None);
            Ok(format!(
                "官方 LS 启动成功, port={}, pid={}",
                port,
                pid.map(|p| p.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            ))
        }
        Err(e) => {
            set_last_error(Some(e.clone()));
            Err(e)
        }
    }
}

#[command]
pub async fn stop_official_ls() -> Result<String, String> {
    let mut guard = runtime().process.lock().await;
    if let Some(mut handle) = guard.take() {
        let pid = handle.child.id();
        handle.shutdown().await;
        set_last_error(None);
        Ok(format!(
            "官方 LS 已停止, pid={}",
            pid.map(|p| p.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ))
    } else {
        Ok("官方 LS 未在运行".to_string())
    }
}

#[command]
pub async fn get_official_ls_status() -> Result<OfficialLsStatus, String> {
    let mut guard = runtime().process.lock().await;
    let enabled = is_official_ls_enabled();
    let binary_path = official_ls_binary_path().ok();
    let last_error = runtime()
        .last_error
        .lock()
        .ok()
        .and_then(|g| g.clone());

    let mut running = false;
    let mut pid = None;
    let mut https_port = None;

    if let Some(handle) = guard.as_mut() {
        match handle.child.try_wait() {
            Ok(Some(_)) => {
                *guard = None;
            }
            Ok(None) => {
                running = true;
                pid = handle.child.id();
                https_port = Some(handle.started.https_port);
            }
            Err(e) => {
                set_last_error(Some(format!("LS process check failed: {}", e)));
                *guard = None;
            }
        }
    }

    Ok(OfficialLsStatus {
        enabled,
        running,
        pid,
        https_port,
        binary_path,
        last_error,
    })
}
