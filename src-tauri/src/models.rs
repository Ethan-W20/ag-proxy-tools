use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

#[derive(Clone, Serialize)]
pub struct LogPayload {
    pub message: String,
    #[serde(rename = "type")]
    pub log_type: String,
    pub details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: Option<String>,
    pub email: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub expiry_timestamp: i64,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_error: Option<QuotaErrorInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaErrorInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<u16>,
    pub message: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchStatus {
    pub applied: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertStatus {
    pub installed: bool,
    pub cert_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProvider {
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub protocol: String,                   // "openai" | "gemini" | "claude"
    pub model_map: HashMap<String, String>, // source model name -> provider model name
    pub enabled: bool,
}

pub struct AppState {
    pub accounts: Arc<Mutex<Vec<Account>>>,
    pub current_idx: Arc<Mutex<i32>>,
    pub proxy_running: Arc<Mutex<bool>>,
    pub proxy_shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
    pub providers: Arc<Mutex<Vec<AiProvider>>>,
    pub routing_strategy: Arc<Mutex<String>>,
    pub header_passthrough: Arc<Mutex<bool>>,
    pub official_ls_enabled: Arc<Mutex<bool>>,
    pub upstream_server: Arc<Mutex<String>>,
    pub upstream_custom_url: Arc<Mutex<String>>,
    pub http_protocol_mode: Arc<Mutex<String>>,
    pub capacity_failover_enabled: Arc<Mutex<bool>>,
    pub token_stats: crate::token_stats::TokenStatsManager,
    pub quota_threshold: Arc<Mutex<i32>>,
    pub quota_cache: Arc<Mutex<HashMap<String, QuotaData>>>,
    /// Recent context usage entries for sliding-window max
    pub last_context_usage: Arc<Mutex<Vec<(u64, String, i64)>>>,
    /// The most recent entry (never expires) — fallback when window is empty
    pub context_usage_latest: Arc<Mutex<(u64, String)>>,
    /// Sliding window size in seconds (configurable by user, default 15)
    pub context_ring_window_secs: Arc<Mutex<u64>>,
    /// Auto-accept configuration (synced to injected IDE script via /auto-accept-config endpoint)
    pub auto_accept_config: Arc<Mutex<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelQuota {
    pub name: String,
    pub percentage: i32,
    pub reset_time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaData {
    pub models: Vec<ModelQuota>,
    pub last_updated: i64,
    pub is_forbidden: bool,
}

/// A single hop in the request flow chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowHop {
    pub node: String,       // e.g. "Client", "Local Proxy", "Official LS", "Upstream"
    pub status: Option<u16>,
    pub detail: Option<String>,
}

/// Payload emitted via event `request-flow` to power the visual flow tracing panel.
/// Emitted multiple times per request:
///   - phase="received"   → request just arrived at proxy
///   - phase="forwarding" → selecting account & forwarding to upstream
///   - phase="streaming"  → upstream responded, streaming back
///   - phase="completed"  → request fully finished
///   - phase="error"      → request failed at some stage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestFlowPayload {
    pub id: String,
    pub timestamp: String,
    pub method: String,
    pub path: String,
    pub account: String,
    pub mode: String,               // "direct" / "official_ls" / "网关" / "proxy"
    pub phase: String,              // "received" / "forwarding" / "streaming" / "completed" / "error"
    pub target: Option<String>,     // upstream target URL or label
    /// Forward hops (request direction →)
    pub forward_hops: Vec<FlowHop>,
    /// Return hops (response direction ←)
    pub return_hops: Vec<FlowHop>,
    pub final_status: Option<u16>,
    pub elapsed_ms: u128,
    pub detail: Option<String>,
}
