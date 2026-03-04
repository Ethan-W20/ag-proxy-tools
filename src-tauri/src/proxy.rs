use bytes::Bytes;
use chrono::Utc;
use futures_util::stream::StreamExt;
use http::header::{HeaderMap, HeaderName};
use http_body_util::StreamBody;
use hyper::body::{Frame, Incoming};
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{Emitter, State};
use tokio::sync::oneshot;

use crate::cert::ensure_cert_exists;
use crate::constants::{
    get_client_id, get_client_secret, TARGET_HOST_1, TARGET_HOST_2, TARGET_HOST_3, TOKEN_URL,
    USERINFO_URL,
};
use crate::models::{Account, AiProvider, AppState, QuotaData, QuotaErrorInfo};
use crate::provider::{
    extract_model_from_body, extract_model_from_path, find_provider_for_model, forward_to_provider,
};
use crate::utils::{emit_log, emit_request_flow, full_body, get_app_data_dir, BoxBody};
pub(crate) use crate::proxy_error::{
    classify_error_kind_from_message, classify_error_kind_from_status,
    should_disable_account_for_error_kind,
};

// ==================== Token refresh ====================

pub async fn do_refresh_token(refresh_token: &str) -> Result<(String, i64), String> {
    let client = reqwest::Client::new();
    let client_id = get_client_id()?;
    let client_secret = get_client_secret()?;
    let params = [
        ("client_id", client_id.as_str()),
        ("client_secret", client_secret.as_str()),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];

    let resp = client
        .post(TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token 请求失败: {}", e))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Token 响应解析失败: {}", e))?;

    if let Some(access_token) = json.get("access_token").and_then(|t| t.as_str()) {
        let expires_in = json
            .get("expires_in")
            .and_then(|e| e.as_i64())
            .unwrap_or(3600);
        let expiry = Utc::now().timestamp() + expires_in;
        Ok((access_token.to_string(), expiry))
    } else {
        let err = json
            .get("error_description")
            .and_then(|e| e.as_str())
            .or_else(|| json.get("error").and_then(|e| e.as_str()))
            .unwrap_or("unknown error");
        Err(format!("Token 刷新失败: {}", err))
    }
}


fn normalize_current_idx(current_idx: i32, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    if current_idx < 0 {
        return 0;
    }
    let idx = current_idx as usize;
    if idx >= len {
        0
    } else {
        idx
    }
}

fn min_quota_percentage(quota: &QuotaData) -> Option<i32> {
    quota.models.iter().map(|m| m.percentage).min()
}

fn is_account_blocked(account: &Account, quota_cache: &HashMap<String, QuotaData>) -> bool {
    if account.disabled {
        return true;
    }
    // Treat invalid_grant / OAuth Bad Request as temporary account block.
    // Do not permanently block rate-limit/auth-unauthorized type failures.
    if let Some(ref qe) = account.quota_error {
        let kind = qe.kind.as_deref().unwrap_or("");
        let is_token_broken = kind == "auth_invalid_grant"
            || ((qe.message.contains("token refresh failed") || qe.message.contains("Token 刷新失败"))
                && qe.message.contains("Bad Request"));
        if is_token_broken {
            let age_secs = Utc::now().timestamp() - qe.timestamp;
            if age_secs < 300 {
                return true;
            }
        }
    }
    quota_cache
        .get(&account.email)
        .map(|q| q.is_forbidden)
        .unwrap_or(false)
}

fn is_account_below_threshold(
    account: &Account,
    quota_cache: &HashMap<String, QuotaData>,
    threshold: i32,
) -> bool {
    if threshold <= 0 {
        return false;
    }
    let Some(quota) = quota_cache.get(&account.email) else {
        return false;
    };
    if quota.is_forbidden {
        return true;
    }
    let Some(min_pct) = min_quota_percentage(quota) else {
        return false;
    };
    min_pct < threshold
}

fn pick_account_index(
    accounts: &[Account],
    current_idx: i32,
    routing_strategy: &str,
    quota_cache: &HashMap<String, QuotaData>,
    quota_threshold: i32,
) -> Option<usize> {
    if accounts.is_empty() {
        return None;
    }

    let len = accounts.len();
    let start = normalize_current_idx(current_idx, len);
    let apply_threshold = routing_strategy == "fill" && quota_threshold > 0;

    let find_candidate = |respect_threshold: bool| -> Option<usize> {
        for offset in 0..len {
            let idx = (start + offset) % len;
            let account = &accounts[idx];
            if is_account_blocked(account, quota_cache) {
                continue;
            }
            if respect_threshold
                && is_account_below_threshold(account, quota_cache, quota_threshold)
            {
                continue;
            }
            return Some(idx);
        }
        None
    };

    if let Some(idx) = find_candidate(apply_threshold) {
        return Some(idx);
    }

    if apply_threshold {
        return find_candidate(false);
    }

    None
}

fn advance_current_idx(current_idx: &Arc<Mutex<i32>>, accounts_len: usize) {
    if accounts_len == 0 {
        return;
    }
    let mut idx_lock = current_idx.lock().unwrap();
    let cur = normalize_current_idx(*idx_lock, accounts_len);
    *idx_lock = ((cur + 1) % accounts_len) as i32;
}


fn clear_account_quota_error_marker(accounts: &Arc<Mutex<Vec<Account>>>, index: usize) {
    let mut account_to_persist = None;
    {
        let mut accounts_lock = accounts.lock().unwrap();
        if index < accounts_lock.len() && accounts_lock[index].quota_error.is_some() {
            accounts_lock[index].quota_error = None;
            account_to_persist = Some(accounts_lock[index].clone());
        }
    }
    if let Some(account) = account_to_persist {
        let _ = crate::account::persist_account(&account);
    }
}

pub async fn get_valid_token_for_index(
    accounts: Arc<Mutex<Vec<Account>>>,
    index: usize,
) -> Result<String, String> {
    let (email, refresh_token, access_token, needs_refresh) = {
        let accounts_lock = accounts.lock().unwrap();
        if accounts_lock.is_empty() {
            return Err("no available accounts".to_string());
        }
        if index >= accounts_lock.len() {
            return Err("invalid account index".to_string());
        }
        let acc = &accounts_lock[index];
        let now = Utc::now().timestamp();
        let needs = acc.access_token.is_empty() || acc.expiry_timestamp < now + 300;
        (
            acc.email.clone(),
            acc.refresh_token.clone(),
            acc.access_token.clone(),
            needs,
        )
    };

    if !needs_refresh {
        return Ok(access_token);
    }

    match do_refresh_token(&refresh_token).await {
        Ok((new_token, new_expiry)) => {
            let mut account_to_persist = None;
            {
                let mut accounts_lock = accounts.lock().unwrap();
                if index < accounts_lock.len() {
                    accounts_lock[index].access_token = new_token.clone();
                    accounts_lock[index].expiry_timestamp = new_expiry;
                    accounts_lock[index].disabled = false;
                    accounts_lock[index].disabled_reason = None;
                    accounts_lock[index].disabled_at = None;
                    accounts_lock[index].quota_error = None;
                    account_to_persist = Some(accounts_lock[index].clone());
                }
            }
            if let Some(account) = account_to_persist {
                let _ = crate::account::persist_account(&account);
            }
            Ok(new_token)
        }
        Err(e) => {
            let mut account_to_persist = None;
            {
                let mut accounts_lock = accounts.lock().unwrap();
                if index < accounts_lock.len() {
                    let kind = classify_error_kind_from_message(&e);
                    let is_invalid_grant = should_disable_account_for_error_kind(&kind);
                    if is_invalid_grant {
                        accounts_lock[index].disabled = true;
                        accounts_lock[index].disabled_reason =
                            Some(format!("invalid_grant: {}", e));
                        accounts_lock[index].disabled_at = Some(Utc::now().timestamp());
                    }
                    accounts_lock[index].quota_error = Some(QuotaErrorInfo {
                        kind: Some(kind),
                        code: None,
                        message: format!("OAuth error: {}", e),
                        timestamp: Utc::now().timestamp(),
                    });
                    account_to_persist = Some(accounts_lock[index].clone());
                }
            }
            if let Some(account) = account_to_persist {
                let _ = crate::account::persist_account(&account);
            }
            Err(format!("[{}] Token refresh failed: {}", email, e))
        }
    }
}

const CAPACITY_MODEL_OPUS: &str = "claude-opus-4-6-thinking";
const CAPACITY_MODEL_SONNET: &str = "claude-sonnet-4-6-thinking";

pub(crate) fn normalize_http_protocol_mode(mode: &str) -> String {
    match mode.trim().to_lowercase().as_str() {
        "http10" | "h10" | "http1.0" | "1.0" => "http10".to_string(),
        "http1" | "h1" | "http1.1" => "http1".to_string(),
        "http2" | "h2" => "http2".to_string(),
        _ => "auto".to_string(),
    }
}

#[derive(Clone, Debug)]
struct ForwardTarget {
    label: String,
    target_url: String,
}

/// Resolve forward targets.
///
/// When official LS is enabled and running, the first target is the LS local port.
/// Always falls back to direct upstream targets.
async fn resolve_forward_targets(
    path_query: &str,
    official_ls_enabled: bool,
    upstream_server: &str,
    upstream_custom_url: &str,
) -> Result<Vec<ForwardTarget>, String> {
    let mut targets = Vec::new();

    if official_ls_enabled {
        if let Some(base_url) = crate::ls_bridge::get_official_ls_https_base_url().await {
            targets.push(ForwardTarget {
                label: "official_ls".to_string(),
                target_url: format!("{}{}", base_url, path_query),
            });
        }
    }

    // Always add direct upstream as fallback
    targets.extend(build_legacy_forward_targets(
        path_query,
        upstream_server,
        upstream_custom_url,
    )?);

    Ok(targets)
}

pub(crate) fn normalize_upstream_server(server: &str) -> String {
    match server.trim().to_lowercase().as_str() {
        "custom" => "custom".to_string(),
        _ => "sandbox".to_string(),
    }
}

pub(crate) fn normalize_upstream_custom_url(custom_url: &str) -> Result<String, String> {
    let trimmed = custom_url.trim().trim_end_matches('/').to_string();
    if trimmed.is_empty() {
        return Err("自定义上游地址不能为空".to_string());
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Ok(trimmed);
    }
    Ok(format!("https://{}", trimmed))
}

fn build_legacy_forward_targets(
    path_query: &str,
    upstream_server: &str,
    upstream_custom_url: &str,
) -> Result<Vec<ForwardTarget>, String> {
    let server = normalize_upstream_server(upstream_server);
    let mut targets = Vec::new();
    if server == "custom" {
        let custom_base = normalize_upstream_custom_url(upstream_custom_url)?;
        targets.push(ForwardTarget {
            label: "custom".to_string(),
            target_url: format!("{}{}", custom_base, path_query),
        });
    } else {
        // Fixed fallback order for official upstreams: sandbox -> daily -> prod
        for host in [TARGET_HOST_1, TARGET_HOST_2, TARGET_HOST_3] {
            targets.push(ForwardTarget {
                label: host.to_string(),
                target_url: format!("https://{}{}", host, path_query),
            });
        }
    }
    Ok(targets)
}

fn normalize_project_resource(project_raw: &str) -> Option<String> {
    let trimmed = project_raw.trim().trim_matches('"').trim();
    if trimmed.is_empty() {
        return None;
    }
    let pid = trimmed
        .strip_prefix("projects/")
        .unwrap_or(trimmed)
        .trim_start_matches('/')
        .trim();
    if pid.is_empty() {
        return None;
    }
    Some(format!("projects/{}", pid))
}

fn extract_project_resource_from_load_code_assist(value: &serde_json::Value) -> Option<String> {
    if let Some(project_val) = value.get("cloudaicompanionProject") {
        if let Some(s) = project_val.as_str() {
            if let Some(resource) = normalize_project_resource(s) {
                return Some(resource);
            }
        }
        if let Some(id) = project_val.get("id").and_then(|v| v.as_str()) {
            if let Some(resource) = normalize_project_resource(id) {
                return Some(resource);
            }
        }
    }
    None
}

fn should_fix_project_placeholder(path_query: &str) -> bool {
    path_query.contains("/v1internal:")
        || path_query.contains("projects/")
        || path_query.contains(":streamGenerateContent")
        || path_query.contains(":generateContent")
        || path_query.contains(":countTokens")
}

fn replace_empty_project_placeholders(value: &mut serde_json::Value, replacement: &str) -> bool {
    match value {
        serde_json::Value::String(s) => {
            let normalized = s.trim();
            // Case 1: Entire value is just "projects/", "projects", or "projects//"
            if normalized == "projects/" || normalized == "projects" || normalized == "projects//" {
                *s = replacement.to_string();
                return true;
            }
            // Case 2: Value like "projects//locations/us-central1/..." where project ID is missing
            // Pattern: "projects/" immediately followed by "/" or nothing before known path segments
            if normalized.starts_with("projects//") {
                // Extract the project ID from replacement (e.g., "projects/my-project" -> "my-project")
                let pid = replacement.strip_prefix("projects/").unwrap_or(replacement);
                let suffix = &normalized["projects/".len()..]; // starts with "/"
                *s = format!("projects/{}{}", pid, suffix);
                return true;
            }
            // Case 3: Nested JSON encoded as string.
            if (normalized.starts_with('{') && normalized.ends_with('}'))
                || (normalized.starts_with('[') && normalized.ends_with(']'))
            {
                if let Ok(mut inner) = serde_json::from_str::<serde_json::Value>(normalized) {
                    if replace_empty_project_placeholders(&mut inner, replacement) {
                        if let Ok(serialized) = serde_json::to_string(&inner) {
                            *s = serialized;
                            return true;
                        }
                    }
                }
            }
            false
        }
        serde_json::Value::Object(map) => {
            let mut changed = false;
            for v in map.values_mut() {
                if replace_empty_project_placeholders(v, replacement) {
                    changed = true;
                }
            }
            changed
        }
        serde_json::Value::Array(arr) => {
            let mut changed = false;
            for item in arr.iter_mut() {
                if replace_empty_project_placeholders(item, replacement) {
                    changed = true;
                }
            }
            changed
        }
        _ => false,
    }
}

fn is_empty_project_string(s: &str) -> bool {
    let normalized = s.trim();
    // Exact empty project patterns
    if normalized == "projects/"
        || normalized == "projects"
        || normalized == "projects//"
        || normalized.starts_with("projects//")
    {
        return true;
    }
    // "projects/" with no real ID before next path segment, e.g. "projects/locations/..."
    // These look like "projects/locations" or "projects/regions" without an ID in between
    if let Some(after) = normalized.strip_prefix("projects/") {
        let after = after.trim_start_matches('/');
        // If the part after "projects/" starts directly with a known GCP resource segment
        // (meaning no project ID was inserted), treat as empty
        if after.is_empty()
            || after.starts_with("locations/")
            || after.starts_with("regions/")
            || after.starts_with("zones/")
            || after.starts_with("global/")
        {
            return true;
        }
    }
    false
}

fn contains_empty_project_placeholders(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(s) => {
            if is_empty_project_string(s) {
                return true;
            }
            if (s.starts_with('{') && s.ends_with('}'))
                || (s.starts_with('[') && s.ends_with(']'))
            {
                if let Ok(inner) = serde_json::from_str::<serde_json::Value>(s) {
                    return contains_empty_project_placeholders(&inner);
                }
            }
            false
        }
        serde_json::Value::Object(map) => {
            map.values().any(contains_empty_project_placeholders)
        }
        serde_json::Value::Array(arr) => arr.iter().any(contains_empty_project_placeholders),
        _ => false,
    }
}

fn body_contains_empty_project_placeholders(body_bytes: &Bytes) -> bool {
    if body_bytes.is_empty() {
        return false;
    }
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(body_bytes) {
        return contains_empty_project_placeholders(&value);
    }
    // Fallback: raw string scan for common patterns
    let raw = String::from_utf8_lossy(body_bytes);
    raw.contains("\"name\":\"projects/\"")
        || raw.contains("\"name\": \"projects/\"")
        || raw.contains("projects//")
        || raw.contains("\"name\":\"projects/locations")
        || raw.contains("\"name\": \"projects/locations")
        || raw.contains("\"name\":\"projects/regions")
        || raw.contains("\"name\": \"projects/regions")
}

fn patch_name_project_placeholder_in_raw_json(raw: &str, project_resource: &str) -> Option<String> {
    static NAME_PROJECT_RE: OnceLock<regex::Regex> = OnceLock::new();
    static NAME_PROJECT_WITH_SUFFIX_RE: OnceLock<regex::Regex> = OnceLock::new();
    static ESCAPED_NAME_PROJECT_RE: OnceLock<regex::Regex> = OnceLock::new();
    static ESCAPED_NAME_PROJECT_WITH_SUFFIX_RE: OnceLock<regex::Regex> = OnceLock::new();
    static NAME_PROJECT_MISSING_ID_LOC_RE: OnceLock<regex::Regex> = OnceLock::new();
    static ESCAPED_NAME_PROJECT_MISSING_ID_LOC_RE: OnceLock<regex::Regex> = OnceLock::new();
    let pid = project_resource
        .trim()
        .strip_prefix("projects/")
        .unwrap_or(project_resource.trim())
        .trim_matches('/');
    if pid.is_empty() {
        return None;
    }

    let re_suffix = NAME_PROJECT_WITH_SUFFIX_RE.get_or_init(|| {
        regex::Regex::new(r#""name"\s*:\s*"projects//([^"]*)""#).unwrap()
    });
    let mut changed = false;
    let stage1 = re_suffix
        .replace_all(raw, |caps: &regex::Captures| {
            changed = true;
            let suffix = caps
                .get(1)
                .map(|m| m.as_str())
                .unwrap_or("")
                .trim_start_matches('/');
            if suffix.is_empty() {
                format!(r#""name":"projects/{}""#, pid)
            } else {
                format!(r#""name":"projects/{}/{}""#, pid, suffix)
            }
        })
        .to_string();

    let re_exact = NAME_PROJECT_RE
        .get_or_init(|| regex::Regex::new(r#""name"\s*:\s*"projects/""#).unwrap());
    let stage2 = if re_exact.is_match(&stage1) {
        changed = true;
        let replacement = format!(r#""name":"projects/{}""#, pid);
        re_exact.replace_all(&stage1, replacement.as_str()).to_string()
    } else {
        stage1
    };

    let re_missing_loc = NAME_PROJECT_MISSING_ID_LOC_RE
        .get_or_init(|| regex::Regex::new(r#""name"\s*:\s*"projects/(locations|regions|zones|global)([^"]*)""#).unwrap());
    let stage3 = re_missing_loc
        .replace_all(&stage2, |caps: &regex::Captures| {
            changed = true;
            let seg = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let tail = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            format!(r#""name":"projects/{}/{}{}""#, pid, seg, tail)
        })
        .to_string();

    let re_esc_suffix = ESCAPED_NAME_PROJECT_WITH_SUFFIX_RE.get_or_init(|| {
        regex::Regex::new(r#"\\\"name\\\"\s*:\s*\\\"projects(?:/|\\/){2}([^\\"]*)\\\""#).unwrap()
    });
    let stage4 = re_esc_suffix
        .replace_all(&stage3, |caps: &regex::Captures| {
            changed = true;
            let suffix = caps
                .get(1)
                .map(|m| m.as_str())
                .unwrap_or("")
                .trim_start_matches('/')
                .trim_start_matches('\\');
            if suffix.is_empty() {
                format!(r#"\"name\":\"projects/{}\""#, pid)
            } else {
                format!(r#"\"name\":\"projects/{}/{}\""#, pid, suffix)
            }
        })
        .to_string();

    let re_esc_exact = ESCAPED_NAME_PROJECT_RE
        .get_or_init(|| regex::Regex::new(r#"\\\"name\\\"\s*:\s*\\\"projects(?:/|\\/)\\\""#).unwrap());
    let stage5 = if re_esc_exact.is_match(&stage4) {
        changed = true;
        let replacement = format!(r#"\"name\":\"projects/{}\""#, pid);
        re_esc_exact.replace_all(&stage4, replacement.as_str()).to_string()
    } else {
        stage4
    };

    let re_esc_missing_loc = ESCAPED_NAME_PROJECT_MISSING_ID_LOC_RE
        .get_or_init(|| regex::Regex::new(r#"\\\"name\\\"\s*:\s*\\\"projects(?:/|\\/)(locations|regions|zones|global)([^\\"]*)\\\""#).unwrap());
    let stage6 = re_esc_missing_loc
        .replace_all(&stage5, |caps: &regex::Captures| {
            changed = true;
            let seg = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let tail = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            format!(r#"\"name\":\"projects/{}/{}{}\""#, pid, seg, tail)
        })
        .to_string();

    if changed { Some(stage6) } else { None }
}

fn patch_name_project_placeholder_in_body(
    body_bytes: &Bytes,
    project_resource: Option<&str>,
) -> (Bytes, bool) {
    let Some(project_resource) = project_resource else {
        return (body_bytes.clone(), false);
    };
    let Ok(raw) = std::str::from_utf8(body_bytes) else {
        return (body_bytes.clone(), false);
    };
    if let Some(patched) = patch_name_project_placeholder_in_raw_json(raw, project_resource) {
        return (Bytes::from(patched.into_bytes()), true);
    }
    (body_bytes.clone(), false)
}

#[derive(Clone)]
struct CapacityRetryAttempt {
    body: Bytes,
    model_for_usage: Option<String>,
}

fn normalize_model_for_compare(raw: &str) -> String {
    let mut s = raw.trim().trim_matches('"').to_ascii_lowercase();
    if let Some((head, _)) = s.split_once('?') {
        s = head.to_string();
    }
    if let Some((head, _)) = s.split_once(':') {
        s = head.to_string();
    }
    if let Some(idx) = s.rfind("/models/") {
        s = s[(idx + "/models/".len())..].to_string();
    }
    if let Some(stripped) = s.strip_prefix("models/") {
        s = stripped.to_string();
    }
    s.trim_matches('/').trim().to_string()
}

fn canonical_capacity_model(model: &str) -> Option<&'static str> {
    match normalize_model_for_compare(model).as_str() {
        CAPACITY_MODEL_OPUS => Some(CAPACITY_MODEL_OPUS),
        CAPACITY_MODEL_SONNET => Some(CAPACITY_MODEL_SONNET),
        _ => None,
    }
}

fn is_model_field_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "model" | "modelname" | "model_name" | "modelid" | "model_id"
    )
}

fn rewrite_model_value_with_prefix(original: &str, target_model: &str) -> String {
    let trimmed = original.trim();
    let lower = trimmed.to_ascii_lowercase();

    if let Some(idx) = lower.rfind("/models/") {
        let marker_end = idx + "/models/".len();
        let prefix = &trimmed[..marker_end];
        let rest = &trimmed[marker_end..];
        let suffix_idx = rest
            .find(['?', ':', '/'])
            .unwrap_or(rest.len());
        let suffix = &rest[suffix_idx..];
        return format!("{}{}{}", prefix, target_model, suffix);
    }

    if lower.starts_with("models/") {
        format!("models/{}", target_model)
    } else {
        target_model.to_string()
    }
}

fn replace_model_fields(value: &mut serde_json::Value, source_norm: &str, target_model: &str) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            let mut changed = false;
            for (k, v) in map.iter_mut() {
                if is_model_field_key(k) {
                    if let serde_json::Value::String(s) = v {
                        if normalize_model_for_compare(s) == source_norm {
                            *s = rewrite_model_value_with_prefix(s, target_model);
                            changed = true;
                        }
                    }
                }
                if replace_model_fields(v, source_norm, target_model) {
                    changed = true;
                }
            }
            changed
        }
        serde_json::Value::Array(arr) => arr
            .iter_mut()
            .any(|item| replace_model_fields(item, source_norm, target_model)),
        _ => false,
    }
}

fn build_body_with_switched_model(
    body_bytes: &Bytes,
    source_model: &str,
    target_model: &str,
) -> Option<Bytes> {
    let mut value = serde_json::from_slice::<serde_json::Value>(body_bytes).ok()?;
    let source_norm = normalize_model_for_compare(source_model);
    if source_norm.is_empty() {
        return None;
    }
    if !replace_model_fields(&mut value, &source_norm, target_model) {
        return None;
    }
    serde_json::to_vec(&value).ok().map(Bytes::from)
}

fn is_model_capacity_exhausted_error(status: reqwest::StatusCode, resp_body: &[u8]) -> bool {
    if status.as_u16() != 503 {
        return false;
    }

    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(resp_body) {
        if let Some(details) = v
            .get("error")
            .and_then(|e| e.get("details"))
            .and_then(|d| d.as_array())
        {
            for detail in details {
                if detail
                    .get("reason")
                    .and_then(|r| r.as_str())
                    .map(|r| r.eq_ignore_ascii_case("MODEL_CAPACITY_EXHAUSTED"))
                    .unwrap_or(false)
                {
                    return true;
                }
            }
        }
    }

    let lower = String::from_utf8_lossy(resp_body).to_ascii_lowercase();
    lower.contains("model_capacity_exhausted")
        || (lower.contains("no capacity available for model") && lower.contains("unavailable"))
}

fn is_retryable_internal_error(status: reqwest::StatusCode, resp_body: &[u8]) -> bool {
    if status.as_u16() != 500 {
        return false;
    }

    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(resp_body) {
        if v
            .get("error")
            .and_then(|e| e.get("status"))
            .and_then(|s| s.as_str())
            .map(|s| s.eq_ignore_ascii_case("INTERNAL"))
            .unwrap_or(false)
        {
            return true;
        }
        if v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .map(|m| m.to_ascii_lowercase().contains("internal error"))
            .unwrap_or(false)
        {
            return true;
        }
    }

    let lower = String::from_utf8_lossy(resp_body).to_ascii_lowercase();
    lower.contains("\"status\":\"internal\"") || lower.contains("internal error encountered")
}

fn build_capacity_retry_plan(body_bytes: &Bytes, model_hint: Option<&str>) -> Vec<CapacityRetryAttempt> {
    let inferred_model = model_hint
        .map(|m| m.to_string())
        .or_else(|| extract_model_from_body(body_bytes.as_ref()));
    let Some(initial_model) = inferred_model.as_deref().and_then(canonical_capacity_model) else {
        return vec![CapacityRetryAttempt {
            body: body_bytes.clone(),
            model_for_usage: inferred_model,
        }];
    };

    let mut plan = vec![
        CapacityRetryAttempt {
            body: body_bytes.clone(),
            model_for_usage: Some(initial_model.to_string()),
        },
        CapacityRetryAttempt {
            body: body_bytes.clone(),
            model_for_usage: Some(initial_model.to_string()),
        },
    ];

    let fallback_model = if initial_model == CAPACITY_MODEL_OPUS {
        CAPACITY_MODEL_SONNET
    } else {
        CAPACITY_MODEL_OPUS
    };
    if let Some(switched_body) = build_body_with_switched_model(body_bytes, initial_model, fallback_model)
    {
        plan.push(CapacityRetryAttempt {
            body: switched_body.clone(),
            model_for_usage: Some(fallback_model.to_string()),
        });
        plan.push(CapacityRetryAttempt {
            body: switched_body,
            model_for_usage: Some(fallback_model.to_string()),
        });
    }

    plan
}

fn maybe_patch_project_in_body(
    path_query: &str,
    body_bytes: &Bytes,
    project_resource: Option<&str>,
) -> Bytes {
    if !should_fix_project_placeholder(path_query) || body_bytes.is_empty() {
        return body_bytes.clone();
    }
    let Some(project_resource) = project_resource else {
        return body_bytes.clone();
    };
    let pid = project_resource
        .strip_prefix("projects/")
        .unwrap_or(project_resource);
    if pid.is_empty() {
        return body_bytes.clone();
    }

    let mut value = match serde_json::from_slice::<serde_json::Value>(body_bytes) {
        Ok(v) => v,
        Err(_) => {
            if let Ok(raw) = std::str::from_utf8(body_bytes) {
                if let Some(patched) =
                    patch_name_project_placeholder_in_raw_json(raw, project_resource)
                {
                    let result = Bytes::from(patched.into_bytes());
                    return ensure_project_in_body_raw(&result, pid);
                }
            }
            // For non-JSON bodies, avoid aggressive global replacement by default.
            // Keep payload close to original unless explicitly forced.
            if is_truthy_env("AG_PROXY_FORCE_PROJECT_REWRITE") {
                return ensure_project_in_body_raw(body_bytes, pid);
            }
            return body_bytes.clone();
        }
    };

    let has_empty_placeholders = contains_empty_project_placeholders(&value);
    let force_project_rewrite = is_truthy_env("AG_PROXY_FORCE_PROJECT_REWRITE");
    if has_empty_placeholders || force_project_rewrite {
        // Step 1: Replace empty project placeholders
        let _ = replace_empty_project_placeholders(&mut value, project_resource);

        // Step 2: Replace mismatched project IDs in resource paths.
        // Only do this by default when placeholders were detected.
        replace_project_in_resource_paths(&mut value, pid);
    }

    let serialized = serde_json::to_vec(&value)
        .map(Bytes::from)
        .unwrap_or_else(|_| body_bytes.clone());
    let (patched_body, changed) =
        patch_name_project_placeholder_in_body(&serialized, Some(project_resource));
    if changed {
        return patched_body;
    }
    serialized
}

/// Replace project ID in resource path strings like "projects/OLD_ID/locations/..."
/// with the correct project ID for the selected account.
fn replace_project_in_resource_paths(value: &mut serde_json::Value, correct_pid: &str) {
    static PROJECT_PATH_RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = PROJECT_PATH_RE.get_or_init(|| {
        regex::Regex::new(r#"projects/([^/\s"]+)(/locations/)"#).unwrap()
    });

    match value {
        serde_json::Value::String(s) => {
            if s.contains("projects/") && s.contains("/locations/") {
                let replaced = re.replace_all(s, |_caps: &regex::Captures| {
                    format!("projects/{}/locations/", correct_pid)
                });
                if replaced != *s {
                    *s = replaced.into_owned();
                }
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values_mut() {
                replace_project_in_resource_paths(v, correct_pid);
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                replace_project_in_resource_paths(item, correct_pid);
            }
        }
        _ => {}
    }
}

/// Fallback: replace project in raw body bytes when JSON parsing fails.
fn ensure_project_in_body_raw(body_bytes: &Bytes, correct_pid: &str) -> Bytes {
    static RAW_PROJECT_RE: OnceLock<regex::Regex> = OnceLock::new();
    let Ok(raw) = std::str::from_utf8(body_bytes) else {
        return body_bytes.clone();
    };
    if !raw.contains("projects/") {
        return body_bytes.clone();
    }
    let re = RAW_PROJECT_RE.get_or_init(|| {
        regex::Regex::new(r#"projects/([^/\s"]*)/locations/"#).unwrap()
    });
    let replaced = re.replace_all(raw, |_caps: &regex::Captures| {
        format!("projects/{}/locations/", correct_pid)
    });
    if replaced != raw {
        Bytes::from(replaced.into_owned().into_bytes())
    } else {
        body_bytes.clone()
    }
}

pub(crate) async fn fetch_project_resource_with_token(access_token: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .user_agent("antigravity")
        .build()
        .ok()?;
    let payload = serde_json::json!({
        "metadata": {
            "ideType": "ANTIGRAVITY"
        }
    });
    let upstreams = [TARGET_HOST_1, TARGET_HOST_2, TARGET_HOST_3];

    for host in upstreams {
        let target_url = format!("https://{}/v1internal:loadCodeAssist", host);
        let resp = match client
            .post(&target_url)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .header("x-client-name", "antigravity")
            .header(
                "x-goog-api-client",
                "gl-node/18.18.2 fire/0.8.6 grpc/1.10.x",
            )
            .json(&payload)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[project-discovery] fetch failed host={}: {}", host, e);
                continue;
            }
        };
        let status = resp.status();
        if !status.is_success() {
            eprintln!("[project-discovery] non-2xx from host={} status={}", host, status);
            continue;
        }
        let value = match resp.json::<serde_json::Value>().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[project-discovery] JSON parse failed host={}: {}", host, e);
                continue;
            }
        };
        if let Some(resource) = extract_project_resource_from_load_code_assist(&value) {
            eprintln!("[project-discovery] discovered project: {}", resource);
            return Some(resource);
        } else {
            eprintln!(
                "[project-discovery] no cloudaicompanionProject in response from host={}, keys={:?}",
                host,
                value.as_object().map(|o| o.keys().collect::<Vec<_>>()).unwrap_or_default()
            );
        }
    }
    eprintln!("[project-discovery] all upstreams failed, returning None");
    None
}

async fn resolve_account_project_resource(
    app: &tauri::AppHandle,
    accounts: &Arc<Mutex<Vec<Account>>>,
    selected_idx: usize,
    selected_email: &str,
    selected_project: &str,
    access_token: &str,
) -> Option<String> {
    if let Some(resource) = normalize_project_resource(selected_project) {
        return Some(resource);
    }

    emit_log(
        app,
        &format!(
            "Account [{}] missing project_id, attempting auto-discovery...",
            selected_email
        ),
        "warning",
        None,
    );

    let discovered = match fetch_project_resource_with_token(access_token).await {
        Some(d) => d,
        None => {
            emit_log(
                app,
                &format!(
                    "Account [{}] project_id auto-discovery failed, please set project_id in account config",
                    selected_email
                ),
                "error",
                None,
            );
            return None;
        }
    };
    let project_id = discovered
        .strip_prefix("projects/")
        .unwrap_or(discovered.as_str())
        .to_string();

    emit_log(
        app,
        &format!(
            "Account [{}] project_id auto-discovered: {}",
            selected_email, discovered
        ),
        "success",
        None,
    );

    let mut account_to_persist = None;
    {
        let mut accounts_lock = accounts.lock().unwrap();
        if selected_idx < accounts_lock.len() {
            accounts_lock[selected_idx].project = project_id;
            account_to_persist = Some(accounts_lock[selected_idx].clone());
        }
    }
    if let Some(account) = account_to_persist {
        let _ = crate::account::persist_account(&account);
    }

    Some(discovered)
}

fn should_skip_forward_header(name: &HeaderName) -> bool {
    let n = name.as_str();
    n.eq_ignore_ascii_case("host")
        || n.eq_ignore_ascii_case("content-length")
        || n.eq_ignore_ascii_case("connection")
        || n.eq_ignore_ascii_case("transfer-encoding")
        || n.eq_ignore_ascii_case("authorization")
}

fn is_truthy_env(var_name: &str) -> bool {
    matches!(
        std::env::var(var_name)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn build_upstream_request(
    client: &reqwest::Client,
    method: &http::Method,
    target_url: &str,
    incoming_headers: &HeaderMap,
    body_bytes: &Bytes,
    access_token: &str,
    header_passthrough: bool,
    preserve_incoming_user_agent: bool,
    http_protocol_mode: &str,
    authorization_override: Option<&str>,
) -> reqwest::RequestBuilder {
    let mut req_builder = client.request(method.clone(), target_url);
    if let Some(version) = upstream_request_version_for_mode(http_protocol_mode, target_url) {
        req_builder = req_builder.version(version);
    }

    for (name, value) in incoming_headers.iter() {
        if should_skip_forward_header(name) {
            continue;
        }
        if !header_passthrough
            && !preserve_incoming_user_agent
            && name.as_str().eq_ignore_ascii_case("user-agent")
        {
            continue;
        }
        req_builder = req_builder.header(name, value);
    }

    if !header_passthrough && !preserve_incoming_user_agent {
        req_builder = req_builder.header("User-Agent", "antigravity");
    }

    if let Some(auth) = authorization_override {
        req_builder = req_builder.header("Authorization", auth);
    } else {
        req_builder = req_builder.header("Authorization", format!("Bearer {}", access_token));
    }

    req_builder.body(body_bytes.clone())
}

fn verbose_header_logging_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        matches!(
            std::env::var("AG_PROXY_VERBOSE_HEADER_LOG")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn mask_header_value(name: &str, value: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower == "authorization" {
        if let Some((scheme, _)) = value.split_once(' ') {
            return format!("{} ***", scheme);
        }
        return "***".to_string();
    }
    if lower.contains("cookie")
        || lower.contains("token")
        || lower.contains("api-key")
        || lower.contains("apikey")
    {
        return "***".to_string();
    }
    value.to_string()
}

fn format_incoming_headers_for_log(headers: &HeaderMap) -> String {
    headers
        .iter()
        .map(|(name, value)| {
            let key = name.as_str();
            let raw = value.to_str().unwrap_or("<binary>");
            format!("{}: {}", key, mask_header_value(key, raw))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// NOTE: format_incoming_headers_for_flow removed — identical to format_incoming_headers_for_log

fn format_upstream_headers(headers: &reqwest::header::HeaderMap) -> String {
    headers
        .iter()
        .map(|(name, value)| {
            let key = name.as_str();
            let raw = value.to_str().unwrap_or("<binary>");
            format!("{}: {}", key, mask_header_value(key, raw))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// NOTE: format_upstream_headers_for_flow removed — identical to format_upstream_headers

fn emit_upstream_perf_log(
    app: &tauri::AppHandle,
    method: &http::Method,
    path_query: &str,
    target: &str,
    mode: &str,
    status: Option<u16>,
    elapsed_ms: u128,
) {
    // Only emit perf log for non-2xx or errors to reduce noise
    let is_success = status.map(|s| (200..300).contains(&(s as i32))).unwrap_or(false);
    if is_success {
        return;
    }
    let status_text = status
        .map(|v| v.to_string())
        .unwrap_or_else(|| "ERR".to_string());
    emit_log(
        app,
        &format!(
            "Upstream perf [{} {}] mode={} target={} status={} elapsed={}ms",
            method, path_query, mode, target, status_text, elapsed_ms
        ),
        "warning",
        None,
    );
}

#[derive(Default, Clone)]
struct FlowDetailBundle {
    summary: Option<String>,
    client_to_local: Option<String>,
    local_to_upstream: Option<String>,
    upstream_to_local: Option<String>,
    local_to_client: Option<String>,
}

fn emit_request_summary_log(
    app: &tauri::AppHandle,
    method: &http::Method,
    path_query: &str,
    mode: &str,
    account: &str,
    target: &str,
    status: u16,
    elapsed_ms: u128,
    flow_details: FlowDetailBundle,
    flow_id: &str,
    flow_timestamp: &str,
) {
    let log_type = if (200..300).contains(&(status as i32)) { "dim" } else { "warning" };
    emit_log(
        app,
        &format!(
            "Request summary [{} {}] mode={} account=[{}] target={} status={} elapsed={}ms",
            method, path_query, mode, account, target, status, elapsed_ms
        ),
        log_type,
        None,
    );

    // Also emit structured request-flow event for the visual tracing panel
    use crate::models::{FlowHop, RequestFlowPayload};
    let is_success = (200..300).contains(&(status as i32));
    let is_gateway = mode == "client_gateway" || mode == "gateway";
    let is_ls = mode == "official_ls";

    let fwd_status = Some(status);
    let ret_status = Some(status);
    let d_client_to_local = flow_details.client_to_local.clone();
    let d_local_to_upstream = flow_details.local_to_upstream.clone();
    let d_upstream_to_local = flow_details.upstream_to_local.clone();
    let d_local_to_client = flow_details.local_to_client.clone();

    let (forward_hops, return_hops) = if is_gateway || is_ls {
        (
            vec![
                FlowHop {
                    node: "IDE".into(),
                    status: Some(200),
                    detail: None,
                },
                FlowHop {
                    node: "本地代理".into(),
                    status: Some(200),
                    detail: d_client_to_local.clone(),
                },
                FlowHop {
                    node: if is_ls { "官方LS".into() } else { "网关".into() },
                    status: Some(200),
                    detail: d_local_to_upstream.clone(),
                },
                FlowHop {
                    node: "上游".into(),
                    status: fwd_status,
                    detail: d_local_to_upstream
                        .clone()
                        .or_else(|| Some(format!("target={}", target))),
                },
            ],
            vec![
                FlowHop {
                    node: "上游".into(),
                    status: ret_status,
                    detail: d_upstream_to_local.clone(),
                },
                FlowHop {
                    node: if is_ls { "官方LS".into() } else { "网关".into() },
                    status: ret_status,
                    detail: d_upstream_to_local.clone(),
                },
                FlowHop {
                    node: "本地代理".into(),
                    status: ret_status,
                    detail: d_local_to_client
                        .clone()
                        .or_else(|| d_upstream_to_local.clone()),
                },
                FlowHop {
                    node: "IDE".into(),
                    status: ret_status,
                    detail: d_local_to_client.clone(),
                },
            ],
        )
    } else {
        (
            vec![
                FlowHop {
                    node: "IDE".into(),
                    status: Some(200),
                    detail: None,
                },
                FlowHop {
                    node: "本地代理".into(),
                    status: Some(200),
                    detail: d_client_to_local,
                },
                FlowHop {
                    node: "上游".into(),
                    status: fwd_status,
                    detail: d_local_to_upstream.or_else(|| Some(format!("target={}", target))),
                },
            ],
            vec![
                FlowHop {
                    node: "上游".into(),
                    status: ret_status,
                    detail: d_upstream_to_local.clone(),
                },
                FlowHop {
                    node: "本地代理".into(),
                    status: ret_status,
                    detail: d_local_to_client
                        .clone()
                        .or_else(|| d_upstream_to_local.clone()),
                },
                FlowHop {
                    node: "IDE".into(),
                    status: ret_status,
                    detail: d_local_to_client,
                },
            ],
        )
    };

    let phase = if is_success { "completed" } else { "error" };

    let flow = RequestFlowPayload {
        id: flow_id.to_string(),
        timestamp: flow_timestamp.to_string(),
        method: method.to_string(),
        path: path_query.to_string(),
        account: account.to_string(),
        mode: if is_gateway { "网关".into() } else if is_ls { "official_ls".into() } else { "direct".into() },
        phase: phase.to_string(),
        target: Some(target.to_string()),
        forward_hops,
        return_hops,
        final_status: Some(status),
        elapsed_ms,
        detail: flow_details.summary.or_else(|| {
            if is_success {
                None
            } else {
                Some(format!("target={} status={}", target, status))
            }
        }),
    };
    emit_request_flow(app, &flow);
}

fn format_http_version(version: http::Version) -> &'static str {
    match version {
        http::Version::HTTP_09 => "HTTP/0.9",
        http::Version::HTTP_10 => "HTTP/1.0",
        http::Version::HTTP_11 => "HTTP/1.1",
        http::Version::HTTP_2 => "HTTP/2",
        _ => "HTTP/?",
    }
}

const INTERNAL_UPSTREAM_PROTOCOL_HEADER: &str = "x-ag-upstream-protocol";

fn normalize_http_version_text(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_uppercase().as_str() {
        "HTTP/0.9" => Some("HTTP/0.9"),
        "HTTP/1.0" => Some("HTTP/1.0"),
        "HTTP/1.1" => Some("HTTP/1.1"),
        "HTTP/2" | "HTTP/2.0" => Some("HTTP/2"),
        _ => None,
    }
}

fn resolve_observed_upstream_protocol(resp: &reqwest::Response, transport_mode: &str) -> String {
    if transport_mode == "official_ls" {
        if let Some(v) = resp
            .headers()
            .get(INTERNAL_UPSTREAM_PROTOCOL_HEADER)
            .and_then(|h| h.to_str().ok())
            .and_then(normalize_http_version_text)
        {
            return v.to_string();
        }
    }
    format_http_version(resp.version()).to_string()
}

fn rewrite_request_preview_protocol(preview: &str, protocol: &str) -> String {
    let mut parts = preview.splitn(4, ' ');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(method), Some(path), Some(version), Some(rest)) if version.starts_with("HTTP/") => {
            format!("{} {} {} {}", method, path, protocol, rest)
        }
        _ => preview.to_string(),
    }
}

fn format_upstream_request_preview(
    req: &reqwest::Request,
    fallback_host: &str,
    body_len: usize,
) -> String {
    let url = req.url();
    let path = url.path();
    let query = url.query().map(|q| format!("?{}", q)).unwrap_or_default();
    let path_query = format!("{}{}", path, query);

    let host = if let Some(h) = url.host_str() {
        if let Some(port) = url.port() {
            format!("{}:{}", h, port)
        } else {
            h.to_string()
        }
    } else {
        fallback_host.to_string()
    };
    let version = format_http_version(req.version());

    let headers = format_upstream_headers(req.headers());
    if headers.is_empty() {
        format!(
            "{} {} {} host: {} content-length: {}",
            req.method(),
            path_query,
            version,
            host,
            body_len
        )
    } else {
        format!(
            "{} {} {} host: {} content-length: {} {}",
            req.method(),
            path_query,
            version,
            host,
            body_len,
            headers
        )
    }
}

// NOTE: format_upstream_request_preview_for_flow removed — identical to format_upstream_request_preview


fn format_flow_body_bytes(body: &[u8]) -> String {
    if body.is_empty() {
        return "<empty>".to_string();
    }

    match std::str::from_utf8(body) {
        Ok(text) => text.to_string(),
        Err(_) => format!("<binary body omitted: {} bytes>", body.len()),
    }
}

fn estimate_flow_body_tokens(body: &[u8]) -> usize {
    if body.is_empty() {
        return 0;
    }

    match std::str::from_utf8(body) {
        Ok(text) => {
            let mut ascii_non_ws = 0usize;
            let mut non_ascii_non_ws = 0usize;
            for ch in text.chars() {
                if ch.is_whitespace() {
                    continue;
                }
                if ch.is_ascii() {
                    ascii_non_ws += 1;
                } else {
                    non_ascii_non_ws += 1;
                }
            }
            let ascii_tokens = ascii_non_ws.div_ceil(4);
            let estimated = ascii_tokens.saturating_add(non_ascii_non_ws);
            estimated.max(1)
        }
        Err(_) => body.len().div_ceil(4).max(1),
    }
}

fn header_value_or_dash(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn req_header_value_or_dash(headers: &reqwest::header::HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn short_body_hash_hex(body: &[u8]) -> String {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x00000100000001B3;

    let mut hash = OFFSET_BASIS;
    for b in body {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{:016x}", hash)
}

fn first_body_diff_offset(original: &[u8], forwarded: &[u8]) -> Option<usize> {
    let limit = original.len().min(forwarded.len());
    for i in 0..limit {
        if original[i] != forwarded[i] {
            return Some(i);
        }
    }
    if original.len() != forwarded.len() {
        Some(limit)
    } else {
        None
    }
}

fn build_request_compare_details(
    method: &http::Method,
    path_query: &str,
    target_label: &str,
    original_target_url: &str,
    patched_target_url: &str,
    attempt_idx: usize,
    attempt_total: usize,
    attempt_model: &str,
    incoming_headers: &HeaderMap,
    built_req: &reqwest::Request,
    original_body: &[u8],
    forwarded_body: &[u8],
    header_passthrough: bool,
    preserve_incoming_user_agent: bool,
    stream_auth_passthrough: bool,
    project_name_placeholder_fixed: bool,
    base_project_body_rewritten: bool,
) -> String {
    let upstream_path_query = built_req
        .url()
        .query()
        .map(|q| format!("{}?{}", built_req.url().path(), q))
        .unwrap_or_else(|| built_req.url().path().to_string());

    let compare_json = serde_json::json!({
        "kind": "request_compare",
        "method": method.to_string(),
        "path": path_query,
        "target": target_label,
        "target_url": patched_target_url,
        "upstream_path": upstream_path_query,
        "attempt": attempt_idx + 1,
        "attempt_total": attempt_total,
        "model": attempt_model,
        "body_original_bytes": original_body.len(),
        "body_forward_bytes": forwarded_body.len(),
        "body_original_hash": short_body_hash_hex(original_body),
        "body_forward_hash": short_body_hash_hex(forwarded_body),
        "body_changed": original_body != forwarded_body,
        "first_diff_at": first_body_diff_offset(original_body, forwarded_body),
        "rewrite": {
            "project_in_path_rewritten": patched_target_url != original_target_url,
            "project_body_rewritten_base": base_project_body_rewritten,
            "project_name_placeholder_fixed": project_name_placeholder_fixed,
            "header_passthrough": header_passthrough,
            "preserve_incoming_user_agent": preserve_incoming_user_agent,
            "stream_auth_passthrough": stream_auth_passthrough
        },
        "headers": {
            "incoming_user_agent": header_value_or_dash(incoming_headers, "user-agent"),
            "upstream_user_agent": req_header_value_or_dash(built_req.headers(), "user-agent"),
            "incoming_content_type": header_value_or_dash(incoming_headers, "content-type"),
            "upstream_content_type": req_header_value_or_dash(built_req.headers(), "content-type"),
            "incoming_host": header_value_or_dash(incoming_headers, "host"),
            "upstream_host": built_req.url().host_str().unwrap_or("-")
        }
    });

    serde_json::to_string(&compare_json).unwrap_or_else(|_| "{}".to_string())
}

fn non_empty_or_dash(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "-".to_string()
    } else {
        trimmed.to_string()
    }
}

fn opt_non_empty_or_dash(value: Option<&str>) -> String {
    value
        .map(non_empty_or_dash)
        .unwrap_or_else(|| "-".to_string())
}

fn format_upstream_setting_for_diag(upstream_server: &str, upstream_custom_url: &str) -> String {
    if normalize_upstream_server(upstream_server) == "custom" {
        format!("custom({})", non_empty_or_dash(upstream_custom_url))
    } else {
        "sandbox -> daily -> prod".to_string()
    }
}

fn build_flow_diag_block(
    official_ls_enabled: bool,
    official_ls_running: bool,
    http_protocol_mode: &str,
    upstream_server: &str,
    upstream_custom_url: &str,
    request_model: Option<&str>,
    effective_model: Option<&str>,
    account_project_raw: &str,
    resolved_project_resource: Option<&str>,
    capacity_failover_enabled: bool,
    upstream_attempts: &[String],
) -> String {
    let upstream_attempts_text = if upstream_attempts.is_empty() {
        "-".to_string()
    } else {
        upstream_attempts.join("\n")
    };
    let ls_mode = if official_ls_enabled {
        if official_ls_running { "official_ls (active)" } else { "official_ls (not running, fallback)" }
    } else {
        "direct"
    };
    format!(
        "DIAG\nmode: {}\nhttp_protocol_mode: {}\nupstream_setting: {}\nrequest_model: {}\neffective_model: {}\naccount_project_raw: {}\nresolved_project_resource: {}\ncapacity_failover_enabled: {}\nupstream_attempts:\n{}",
        ls_mode,
        non_empty_or_dash(http_protocol_mode),
        format_upstream_setting_for_diag(upstream_server, upstream_custom_url),
        opt_non_empty_or_dash(request_model),
        opt_non_empty_or_dash(effective_model),
        non_empty_or_dash(account_project_raw),
        opt_non_empty_or_dash(resolved_project_resource),
        if capacity_failover_enabled { "true" } else { "false" },
        upstream_attempts_text
    )
}

// ==================== Context window limits per model ====================

/// Return the maximum context window (in tokens) for a given model name.
/// Used by the /context-info endpoint to let the IDE ring indicator know how full the window is.
fn context_window_limit(model: &str) -> u64 {
    let m = model.to_lowercase();
    // Gemini 2.x models — 1M or 2M depending on variant
    if m.contains("gemini-2") || m.contains("gemini-exp") {
        if m.contains("flash") {
            return 1_048_576; // 1M
        }
        return 2_097_152; // 2M for pro / ultra
    }
    // Gemini 1.x models
    if m.contains("gemini-1") || m.contains("gemini-pro") {
        return 1_048_576; // 1M
    }
    // Gemini generic fallback
    if m.contains("gemini") {
        return 1_048_576;
    }
    // Claude models — 200K
    if m.contains("claude") {
        return 200_000;
    }
    // Default for unknown Google models / code assist
    200_000
}

// ==================== Token usage recording ====================

struct UsageRecorder {
    collected: Vec<u8>,
    app: tauri::AppHandle,
    email: String,
    model: String,
    flow_id: String,
}

impl Drop for UsageRecorder {
    fn drop(&mut self) {
        use tauri::Manager;
        if self.collected.is_empty() {
            return;
        }
        if let Some(app_state) = self.app.try_state::<crate::models::AppState>() {
            if let Some((input, output, cache_read, cache_creation, total)) =
                crate::token_stats::extract_usage_from_sse(&self.collected)
            {
                let email = std::mem::take(&mut self.email);
                let model = std::mem::take(&mut self.model);
                let flow_id = std::mem::take(&mut self.flow_id);
                let model_for_ctx = model.clone();
                emit_log(
                    &self.app,
                    &format!(
                        "TokenStats [{}] model=[{}] in={} out={} cache_read={} cache_create={} total={}",
                        email, model, input, output, cache_read, cache_creation, total
                    ),
                    "dim",
                    None,
                );
                app_state
                    .token_stats
                    .record(crate::token_stats::TokenUsageRecord {
                        timestamp: Utc::now().timestamp(),
                        account_email: email,
                        model,
                        input_tokens: input,
                        output_tokens: output,
                        cache_read_tokens: cache_read,
                        cache_creation_tokens: cache_creation,
                        total_tokens: total,
                    });
                // Emit usage to frontend so flow entry can display real token counts
                if !flow_id.is_empty() {
                    let _ = self.app.emit("flow-usage", serde_json::json!({
                        "flow_id": flow_id,
                        "input_tokens": input,
                        "output_tokens": output,
                        "cache_read_tokens": cache_read,
                        "cache_creation_tokens": cache_creation,
                        "total_tokens": total,
                    }));
                }
                // Update last_context_usage for the /context-info endpoint
                // Push new entry and prune old ones using configurable window
                let window_secs = *app_state.context_ring_window_secs.lock().unwrap();
                if let Ok(mut ctx) = app_state.last_context_usage.lock() {
                    let now = Utc::now().timestamp();
                    ctx.push((input, model_for_ctx.clone(), now));
                    ctx.retain(|&(_, _, ts)| now - ts < window_secs as i64);
                }
                // Always update the latest entry (never expires, prevents 0 bug)
                if let Ok(mut latest) = app_state.context_usage_latest.lock() {
                    *latest = (input, model_for_ctx);
                }
            }
        }
    }
}

fn record_non_sse_usage(
    app: &tauri::AppHandle,
    resp_body: &[u8],
    email: &str,
    model: &Option<String>,
) {
    use tauri::Manager;
    if resp_body.is_empty() {
        return;
    }
    if let Some(app_state) = app.try_state::<crate::models::AppState>() {
        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(resp_body) {
            if let Some((input, output, cache_read, cache_creation, total)) =
                crate::token_stats::parse_usage_auto(&json)
            {
                let model_str = model.clone().unwrap_or_default();
                emit_log(
                    app,
                    &format!(
                        "TokenStats [{}] model=[{}] in={} out={} cache_read={} cache_create={} total={}",
                        email, model_str, input, output, cache_read, cache_creation, total
                    ),
                    "dim",
                    None,
                );
                app_state
                    .token_stats
                    .record(crate::token_stats::TokenUsageRecord {
                        timestamp: Utc::now().timestamp(),
                        account_email: email.to_string(),
                        model: model_str,
                        input_tokens: input,
                        output_tokens: output,
                        cache_read_tokens: cache_read,
                        cache_creation_tokens: cache_creation,
                        total_tokens: total,
                    });
            }
        }
    }
}

fn build_http_client(protocol_mode: &str) -> reqwest::Client {
    let mut builder = reqwest::Client::builder();
    match normalize_http_protocol_mode(protocol_mode).as_str() {
        "http10" => {
            builder = builder.http1_only();
        }
        "http1" => {
            builder = builder.http1_only();
        }
        "http2" => {
            // Force HTTP/2 when talking to HTTPS upstreams.
            builder = builder.http2_prior_knowledge();
        }
        _ => {}
    }
    builder.build().unwrap()
}

pub(crate) fn upstream_request_version_for_mode(
    protocol_mode: &str,
    target_url: &str,
) -> Option<http::Version> {
    let normalized = normalize_http_protocol_mode(protocol_mode);
    let is_https = target_url.trim_start().to_ascii_lowercase().starts_with("https://");
    match normalized.as_str() {
        "http10" => Some(http::Version::HTTP_10),
        "http1" => Some(http::Version::HTTP_11),
        "http2" if is_https => Some(http::Version::HTTP_2),
        _ => None,
    }
}

pub(crate) fn get_shared_http_client_for_target(
    protocol_mode: &str,
    target_url: &str,
) -> &'static reqwest::Client {
    static AUTO_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    static HTTP10_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    static HTTP1_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    static HTTP2_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

    let normalized = normalize_http_protocol_mode(protocol_mode);
    let is_https = target_url.trim_start().to_ascii_lowercase().starts_with("https://");
    let effective = if normalized == "http2" && !is_https {
        "auto"
    } else {
        normalized.as_str()
    };

    match effective {
        "http10" => HTTP10_CLIENT.get_or_init(|| build_http_client("http10")),
        "http1" => HTTP1_CLIENT.get_or_init(|| build_http_client("http1")),
        "http2" => HTTP2_CLIENT.get_or_init(|| build_http_client("http2")),
        _ => AUTO_CLIENT.get_or_init(|| build_http_client("auto")),
    }
}

// ==================== Proxy request handling ====================

async fn handle_proxy_request(
    app: tauri::AppHandle,
    req: http::Request<Incoming>,
    accounts: Arc<Mutex<Vec<Account>>>,
    current_idx: Arc<Mutex<i32>>,
    providers: Arc<Mutex<Vec<AiProvider>>>,
    routing_strategy_arc: Arc<Mutex<String>>,
    header_passthrough_arc: Arc<Mutex<bool>>,
    official_ls_enabled_arc: Arc<Mutex<bool>>,
    upstream_server_arc: Arc<Mutex<String>>,
    upstream_custom_url_arc: Arc<Mutex<String>>,
    http_protocol_mode_arc: Arc<Mutex<String>>,
    capacity_failover_enabled_arc: Arc<Mutex<bool>>,
    quota_cache_arc: Arc<Mutex<HashMap<String, QuotaData>>>,
    quota_threshold_arc: Arc<Mutex<i32>>,
) -> Result<http::Response<BoxBody>, hyper::Error> {
    use http_body_util::BodyExt;

    let request_started = Instant::now();
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path_query = uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_default();
    let incoming_headers = req.headers().clone();
    let incoming_authorization = incoming_headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string());
    let stream_auth_passthrough = path_query.contains(":streamGenerateContent")
        && is_truthy_env("AG_PROXY_STREAM_AUTH_PASSTHROUGH")
        && incoming_authorization
            .as_deref()
            .map(|s| s.to_ascii_lowercase().starts_with("bearer "))
            .unwrap_or(false);
    let is_stream_generate_request = path_query.contains(":streamGenerateContent");
    let incoming_auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let incoming_header_dump = format_incoming_headers_for_log(&incoming_headers);
    if verbose_header_logging_enabled() {
        emit_log(
            &app,
            &format!(
                "IDE headers [{} {}] {}",
                method, path_query, incoming_header_dump
            ),
            "info",
            None,
        );
    }

    let body_bytes = match req.into_body().collect().await {
        Ok(b) => b.to_bytes(),
        Err(e) => {
            emit_log(&app, &format!("Failed to read request body: {}", e), "error", None);
            return Ok(http::Response::builder()
                .status(400)
                .body(full_body(Bytes::from(format!(
                    "Failed to read request body: {}",
                    e
                ))))
                .unwrap());
        }
    };
    let request_body_tokens_for_flow = estimate_flow_body_tokens(body_bytes.as_ref());
    let request_flow_base = format!(
            "REQUEST\nmethod: {}\npath: {}\nheaders (IDE original):\n{}\n\nbody: {} tokens",
        method, path_query, incoming_header_dump, request_body_tokens_for_flow
    );
    // Will be set once we build the upstream request, to show actual sent headers
    let mut upstream_request_preview_for_flow: Option<String> = None;

    let routing_strategy = routing_strategy_arc.lock().unwrap().clone();
    let header_passthrough = *header_passthrough_arc.lock().unwrap();
    let official_ls_enabled = *official_ls_enabled_arc.lock().unwrap();
    let upstream_server = upstream_server_arc.lock().unwrap().clone();
    let upstream_custom_url = upstream_custom_url_arc.lock().unwrap().clone();
    let http_protocol_mode =
        normalize_http_protocol_mode(&http_protocol_mode_arc.lock().unwrap().clone());
    let capacity_failover_enabled = *capacity_failover_enabled_arc.lock().unwrap();
    let official_ls_running = crate::ls_bridge::get_official_ls_https_base_url().await.is_some();
    let effective_transport_mode = if official_ls_enabled && official_ls_running {
        "official_ls".to_string()
    } else {
        "direct".to_string()
    };

    // Generate unique flow ID for real-time tracking across phases
    let flow_id = uuid::Uuid::new_v4().to_string();
    let flow_timestamp = Utc::now().format("%H:%M:%S").to_string();

    // ---- /context-info endpoint for IDE injected ring indicator ----
    if path_query == "/context-info" || path_query.starts_with("/context-info?") {
        use tauri::Manager;
        let (input_tokens, model_name) = {
            if let Some(app_state) = app.try_state::<crate::models::AppState>() {
                let window_secs = *app_state.context_ring_window_secs.lock().unwrap();
                let mut entries = app_state.last_context_usage.lock().unwrap();
                let now = Utc::now().timestamp();
                entries.retain(|&(_, _, ts)| now - ts < window_secs as i64);
                if let Some(max_entry) = entries.iter().max_by_key(|e| e.0) {
                    (max_entry.0, max_entry.1.clone())
                } else {
                    // Window empty — use the latest entry as fallback (prevents 0 bug)
                    app_state.context_usage_latest.lock().unwrap().clone()
                }
            } else {
                (0u64, String::new())
            }
        };
        let max_context = context_window_limit(&model_name);
        let json_body = serde_json::json!({
            "input_tokens": input_tokens,
            "model": model_name,
            "max_context": max_context,
        });
        return Ok(http::Response::builder()
            .status(200)
            .header("Content-Type", "application/json")
            .header("Access-Control-Allow-Origin", "*")
            .body(full_body(Bytes::from(json_body.to_string())))
            .unwrap());
    }

    // ---- /auto-accept-config endpoint for IDE injected auto-accept script ----
    if path_query == "/auto-accept-config" || path_query.starts_with("/auto-accept-config?") {
        use tauri::Manager;
        let config_json = if let Some(app_state) = app.try_state::<crate::models::AppState>() {
            app_state.auto_accept_config.lock().unwrap().clone()
        } else {
            "{}".to_string()
        };
        return Ok(http::Response::builder()
            .status(200)
            .header("Content-Type", "application/json")
            .header("Access-Control-Allow-Origin", "*")
            .body(full_body(Bytes::from(config_json)))
            .unwrap());
    }

    if path_query.starts_with("/api/update/") {
        return Ok(http::Response::builder()
            .status(204)
            .body(full_body(Bytes::new()))
            .unwrap());
    }

    if path_query.starts_with("/oauth2/v2/userinfo") {
        let client = match reqwest::Client::builder().user_agent("antigravity").build() {
            Ok(c) => c,
            Err(e) => {
                emit_log(
                    &app,
                    &format!("userinfo client init failed: {}", e),
                    "error",
                    None,
                );
                return Ok(http::Response::builder()
                    .status(500)
                    .body(full_body(Bytes::from(format!(
                        "userinfo client init failed: {}",
                        e
                    ))))
                    .unwrap());
            }
        };

        let mut forward_req = client.request(method.clone(), USERINFO_URL);
        if let Some(auth) = incoming_auth_header.as_deref() {
            forward_req = forward_req.header("Authorization", auth);
        }
        if !body_bytes.is_empty() {
            forward_req = forward_req.body(body_bytes.clone());
        }

        let resp = match forward_req.send().await {
            Ok(r) => r,
            Err(e) => {
                emit_log(&app, &format!("userinfo forward failed: {}", e), "error", None);
                return Ok(http::Response::builder()
                    .status(502)
                    .body(full_body(Bytes::from(format!("userinfo forward failed: {}", e))))
                    .unwrap());
            }
        };

        let status = resp.status();
        let resp_status =
            http::StatusCode::from_u16(status.as_u16()).unwrap_or(http::StatusCode::BAD_GATEWAY);
        let mut builder = http::Response::builder().status(resp_status);
        for (key, value) in resp.headers() {
            builder = builder.header(key.as_str(), value.as_bytes());
        }
        let resp_body = resp.bytes().await.unwrap_or_default();
        if !status.is_success() {
            let preview: String = String::from_utf8_lossy(&resp_body)
                .chars()
                .take(220)
                .collect();
            emit_log(
                &app,
                &format!("Google userinfo forward error ({}): {}", status, preview),
                "warning",
                None,
            );
        }
        return Ok(builder.body(full_body(resp_body)).unwrap());
    }

    let model_name =
        extract_model_from_body(&body_bytes).or_else(|| extract_model_from_path(&path_query));
    if let Some(ref model) = model_name {
        let providers_list = providers.lock().unwrap().clone();
        if let Some((provider, target_model)) = find_provider_for_model(&providers_list, model) {
            emit_log(
                &app,
                &format!(
                    "Model mapping: [{}] -> provider [{}] model [{}]",
                    model, provider.name, target_model
                ),
                "dim",
                None,
            );

            let max_retries = 3u32;
            let mut last_err = String::new();
            for attempt in 1..=max_retries {
                if attempt > 1 {
                    let backoff_ms = 500u64 * attempt as u64;
                    emit_log(
                        &app,
                        &format!(
                            "Provider retry {}/{} [{}], backoff={}ms",
                            attempt, max_retries, provider.name, backoff_ms
                        ),
                        "warning",
                        None,
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                }

                match forward_to_provider(&app, &provider, &target_model, &body_bytes).await {
                    Ok(resp) => {
                        if attempt > 1 {
                            emit_log(
                                &app,
                                &format!(
                                    "Provider retry succeeded [{}] attempt {}",
                                    provider.name, attempt
                                ),
                                "success",
                                None,
                            );
                        }
                        return Ok(resp.map(|b: http_body_util::Full<Bytes>| {
                            b.map_err(|never| -> Box<dyn std::error::Error + Send + Sync> {
                                match never {}
                            })
                            .boxed()
                        }));
                    }
                    Err(e) => {
                        last_err = e.clone();
                        emit_log(
                            &app,
                            &format!(
                                "Provider request failed [{}] {}/{}: {}",
                                provider.name, attempt, max_retries, e
                            ),
                            "error",
                            None,
                        );
                    }
                }
            }

            emit_log(
                &app,
                &format!(
                    "Provider chain failed [{}], retries={}, error={}",
                    provider.name, max_retries, last_err
                ),
                "error",
                None,
            );
            return Ok(http::Response::builder()
                .status(502)
                .header("Content-Type", "application/json")
                .body(full_body(Bytes::from(format!(
                    "{{\"error\":{{\"message\":\"Provider [{}] failed after {} retries: {}\",\"code\":502}}}}",
                    provider.name,
                    max_retries,
                    last_err.replace('"', "'")
                ))))
                .unwrap());
        }
    }

    // Always mock non-essential telemetry/auxiliary v1internal endpoints.
    // These are just telemetry calls (recordTrajectoryAnalytics, recordCodeAssistMetrics, etc.)
    // that the IDE doesn't need for core functionality. Forwarding them often fails with
    // 400 "Invalid project resource name" when the account's project ID isn't set up.
    let is_log_upload = path_query == "/log" || path_query.starts_with("/log?");
    let is_telemetry = path_query.contains("/v1internal:recordCodeAssistMetrics")
        || path_query.contains("/v1internal:recordTrajectoryAnalytics")
        || path_query.contains("/v1internal:fetchUserInfo")
        || path_query.contains("/v1internal:fetchAdminControls")
        || path_query.contains("/v1internal/cascadeNuxes")
        || is_log_upload;

    if is_telemetry {
        if is_log_upload {
            // IDE /log is telemetry only. Swallow it to avoid useless upstream 404 spam.
            return Ok(http::Response::builder()
                .status(204)
                .body(full_body(Bytes::new()))
                .unwrap());
        }
        return Ok(http::Response::builder()
            .status(200)
            .header("Content-Type", "application/json")
            .body(full_body(Bytes::from("{}")))
            .unwrap());
    }

    let has_accounts = {
        let accs = accounts.lock().unwrap();
        !accs.is_empty()
    };

    if !has_accounts {
        let is_aux = path_query.contains("/v1internal:loadCodeAssist")
            || path_query.contains("/v1internal:fetchAvailableModels");

        if is_aux {
            return Ok(http::Response::builder()
                .status(200)
                .header("Content-Type", "application/json")
                .body(full_body(Bytes::from("{}")))
                .unwrap());
        }
    }

    let forward_targets = match resolve_forward_targets(
        &path_query,
        official_ls_enabled,
        &upstream_server,
        &upstream_custom_url,
    ).await {
        Ok(v) => v,
        Err(e) => {
            return Ok(http::Response::builder()
                .status(400)
                .body(full_body(Bytes::from(format!(
                    "Failed to resolve upstream targets: {}",
                    e
                ))))
                .unwrap());
        }
    };
    let total_accounts = {
        let accounts_lock = accounts.lock().unwrap();
        accounts_lock.len()
    };
    let mut attempted_accounts: HashSet<usize> = HashSet::new();
    let url_needs_project = path_query.contains("projects//");
    let body_needs_project =
        should_fix_project_placeholder(&path_query) && body_contains_empty_project_placeholders(&body_bytes);
    // Optional compatibility switch: force project patching on critical endpoints.
    // Disabled by default to keep request payloads closer to the original IDE request.
    let path_always_needs_project = is_truthy_env("AG_PROXY_FORCE_PROJECT_REWRITE")
        && (path_query.contains(":streamGenerateContent")
            || path_query.contains(":generateContent")
            || path_query.contains(":countTokens")
            || path_query.contains(":loadCodeAssist"));
    let request_needs_project = url_needs_project || body_needs_project || path_always_needs_project;
    let mut skipped_for_missing_project: Vec<String> = Vec::new();
    let mut attempted_upstream_forward = false;

    for _ in 0..total_accounts {
        let (selected_idx, selected_email, selected_project) = {
            let accounts_lock = accounts.lock().unwrap();
            let quota_cache = quota_cache_arc.lock().unwrap();
            let quota_threshold = *quota_threshold_arc.lock().unwrap();
            let mut idx_lock = current_idx.lock().unwrap();

            let Some(idx) = pick_account_index(
                &accounts_lock,
                *idx_lock,
                &routing_strategy,
                &quota_cache,
                quota_threshold,
            ) else {
                break;
            };

            *idx_lock = idx as i32;
            (
                idx,
                accounts_lock[idx].email.clone(),
                accounts_lock[idx].project.clone(),
            )
        };

        // Notify frontend about account switch so dashboard updates in real-time
        let _ = app.emit("account-switched", selected_idx as i32);

        if !attempted_accounts.insert(selected_idx) {
            advance_current_idx(&current_idx, total_accounts);
            continue;
        }

        let token = match get_valid_token_for_index(accounts.clone(), selected_idx).await {
            Ok(t) => t,
            Err(e) => {
                emit_log(
                    &app,
                    &format!("Account auth failed [{}]: {}", selected_email, e),
                    "warning",
                    None,
                );
                advance_current_idx(&current_idx, total_accounts);
                continue;
            }
        };

        let project_resource = if request_needs_project {
            resolve_account_project_resource(
                &app,
                &accounts,
                selected_idx,
                &selected_email,
                &selected_project,
                &token,
            )
            .await
        } else {
            normalize_project_resource(&selected_project)
        };

        // If request needs a project but we couldn't resolve one, skip this account
        if request_needs_project && project_resource.is_none() {
            emit_log(
                &app,
                &format!(
                    "Skip account [{}] path [{}]: missing project_id (url_needs_project={}, body_needs_project={})",
                    selected_email, path_query, url_needs_project, body_needs_project
                ),
                "warning",
                None,
            );
            skipped_for_missing_project.push(selected_email.clone());
            advance_current_idx(&current_idx, total_accounts);
            continue;
        }

        let body_for_upstream =
            maybe_patch_project_in_body(&path_query, &body_bytes, project_resource.as_deref());

        let mut auth_or_rate_limited = false;

        let mut pending_non_success: Option<http::Response<BoxBody>> = None;
        let mut pending_non_success_details: Option<FlowDetailBundle> = None;
        let mut upstream_attempts_for_flow: Vec<String> = Vec::new();
        let mut upstream_attempt_seq: usize = 0;
        let requested_model_norm = model_name
            .as_deref()
            .map(normalize_model_for_compare)
            .unwrap_or_default();
        let mut blocked_capacity_model_norm: Option<String> = None;

        'target_loop: for target in &forward_targets {
            attempted_upstream_forward = true;
            let is_primary_ls_target =
                effective_transport_mode == "official_ls" && target.label == "official_ls";
            // Also fix empty project ID in URL path (e.g., /v1/projects//locations/...)
            let patched_url = if let Some(pr) = project_resource.as_deref() {
                let pid = pr.strip_prefix("projects/").unwrap_or(pr);
                if target.target_url.contains("projects//") {
                    target.target_url.replace("projects//", &format!("projects/{}/", pid))
                } else {
                    target.target_url.clone()
                }
            } else {
                target.target_url.clone()
            };
            let client = get_shared_http_client_for_target(&http_protocol_mode, &patched_url);
            let capacity_attempts = if capacity_failover_enabled {
                build_capacity_retry_plan(&body_for_upstream, model_name.as_deref())
            } else {
                vec![CapacityRetryAttempt {
                    body: body_for_upstream.clone(),
                    model_for_usage: model_name.clone(),
                }]
            };
            let capacity_attempts_len = capacity_attempts.len();

            for attempt_idx in 0..capacity_attempts_len {
                let attempt = &capacity_attempts[attempt_idx];
                let mut attempt_body = if request_needs_project {
                    maybe_patch_project_in_body(
                        &path_query,
                        &attempt.body,
                        project_resource.as_deref(),
                    )
                } else {
                    attempt.body.clone()
                };
                let (attempt_body_patched, patched_name_placeholder) =
                    patch_name_project_placeholder_in_body(
                        &attempt_body,
                        project_resource.as_deref(),
                    );
                if patched_name_placeholder {
                    upstream_attempts_for_flow.push(format!(
                        "#{} project_name_placeholder_fixed=true",
                        upstream_attempt_seq + 1
                    ));
                    attempt_body = attempt_body_patched;
                }
                let is_last_capacity_attempt = attempt_idx + 1 >= capacity_attempts_len;
                let attempt_model_for_usage =
                    attempt.model_for_usage.clone().or_else(|| model_name.clone());
                upstream_attempt_seq += 1;
                let attempt_model_text = attempt_model_for_usage
                    .clone()
                    .unwrap_or_else(|| "-".to_string());
                let attempt_model_norm = attempt_model_for_usage
                    .as_deref()
                    .map(normalize_model_for_compare)
                    .unwrap_or_default();
                if let Some(blocked_norm) = blocked_capacity_model_norm.as_deref() {
                    if !attempt_model_norm.is_empty() && attempt_model_norm == *blocked_norm {
                        upstream_attempts_for_flow.push(format!(
                            "#{} target={} attempt={}/{} model={} skipped=model_not_available",
                            upstream_attempt_seq,
                            target.label,
                            attempt_idx + 1,
                            capacity_attempts_len,
                            attempt_model_text.as_str()
                        ));
                        continue;
                    }
                }

                let forward_req = build_upstream_request(
                    client,
                    &method,
                    &patched_url,
                    &incoming_headers,
                    &attempt_body,
                    &token,
                    header_passthrough,
                    is_stream_generate_request,
                    &http_protocol_mode,
                    if stream_auth_passthrough {
                        incoming_authorization.as_deref()
                    } else {
                        None
                    },
                );

                let built_req = match forward_req.build() {
                    Ok(r) => r,
                    Err(e) => {
                        upstream_attempts_for_flow.push(format!(
                            "#{} target={} attempt={}/{} model={} build_error={}",
                            upstream_attempt_seq,
                            target.label,
                            attempt_idx + 1,
                            capacity_attempts_len,
                            attempt_model_text.as_str(),
                            e
                        ));
                        emit_log(
                            &app,
                            &format!(
                                "上游请求异常 [{}] (attempt {}/{}): {}",
                                target.label,
                                attempt_idx + 1,
                                capacity_attempts_len,
                                e
                            ),
                            "warning",
                            None,
                        );
                        break;
                    }
                };

                let upstream_preview =
                    format_upstream_request_preview(&built_req, &target.label, attempt_body.len());
                upstream_request_preview_for_flow = Some(upstream_preview.clone());

                if is_stream_generate_request {
                    let compare_details = build_request_compare_details(
                        &method,
                        &path_query,
                        &target.label,
                        &target.target_url,
                        &patched_url,
                        attempt_idx,
                        capacity_attempts_len,
                        &attempt_model_text,
                        &incoming_headers,
                        &built_req,
                        body_bytes.as_ref(),
                        attempt_body.as_ref(),
                        header_passthrough,
                        is_stream_generate_request,
                        stream_auth_passthrough,
                        patched_name_placeholder,
                        body_for_upstream.as_ref() != body_bytes.as_ref(),
                    );
                    let compare_message = format!(
                        "Request compare [{} {}] target={} attempt={}/{}",
                        method,
                        path_query,
                        target.label,
                        attempt_idx + 1,
                        capacity_attempts_len
                    );
                    emit_log(&app, &compare_message, "dim", Some(compare_details.as_str()));
                }

                if verbose_header_logging_enabled() {
                    emit_log(
                        &app,
                        &format!(
                            "上游请求 [{} {}] -> {} {}",
                            method,
                            path_query,
                            target.label,
                            upstream_preview
                        ),
                        "info",
                        None,
                    );
                }

                let upstream_started = Instant::now();
                let resp = match client.execute(built_req).await {
                    Ok(r) => r,
                    Err(e) => {
                        upstream_attempts_for_flow.push(format!(
                            "#{} target={} attempt={}/{} model={} send_error={}",
                            upstream_attempt_seq,
                            target.label,
                            attempt_idx + 1,
                            capacity_attempts_len,
                            attempt_model_text.as_str(),
                            e
                        ));
                        emit_upstream_perf_log(
                            &app,
                            &method,
                            &path_query,
                            &target.label,
                            &effective_transport_mode,
                            None,
                            upstream_started.elapsed().as_millis(),
                        );
                        emit_log(
                            &app,
                            &format!(
                                "上游请求异常 [{}] (attempt {}/{}): {}",
                                target.label,
                                attempt_idx + 1,
                                capacity_attempts_len,
                                e
                            ),
                            "warning",
                            None,
                        );
                        break;
                    }
                };

                let status = resp.status();
                let upstream_protocol =
                    resolve_observed_upstream_protocol(&resp, &effective_transport_mode);
                let upstream_elapsed_ms = upstream_started.elapsed().as_millis();
                upstream_attempts_for_flow.push(format!(
                    "#{} target={} attempt={}/{} model={} status={} protocol={} elapsed_ms={}",
                    upstream_attempt_seq,
                    target.label,
                    attempt_idx + 1,
                    capacity_attempts_len,
                    attempt_model_text.as_str(),
                    status.as_u16(),
                    upstream_protocol,
                    upstream_elapsed_ms
                ));
                if status == 401 || status == 403 || status == 429 {
                    let err_body = resp.text().await.unwrap_or_default();
                    let preview: String = err_body.chars().take(220).collect();
                    let kind = classify_error_kind_from_status(status.as_u16(), &path_query, &preview);
                    emit_upstream_perf_log(
                        &app,
                        &method,
                        &path_query,
                        &target.label,
                        &effective_transport_mode,
                        Some(status.as_u16()),
                        upstream_started.elapsed().as_millis(),
                    );
                    emit_log(
                        &app,
                        &format!(
                            "鉴权/限流 [{}] status={} kind={} account=[{}] path=[{}] body={}",
                            target.label,
                            status.as_u16(),
                            kind,
                            selected_email,
                            path_query,
                            preview
                        ),
                        "warning",
                        None,
                    );
                    // Note: do NOT mark account as errored here.
                    // Proxy forwarding failures (401/403/429) are usually platform-side issues,
                    // not account-specific. Only quota refresh errors in quota.rs mark accounts.
                    auth_or_rate_limited = true;
                    advance_current_idx(&current_idx, total_accounts);
                    break 'target_loop;
                }

                let resp_status = http::StatusCode::from_u16(status.as_u16()).unwrap();
                let mut builder = http::Response::builder().status(resp_status);
                for (key, value) in resp.headers() {
                    if key
                        .as_str()
                        .eq_ignore_ascii_case(INTERNAL_UPSTREAM_PROTOCOL_HEADER)
                    {
                        continue;
                    }
                    builder = builder.header(key.as_str(), value.as_bytes());
                }

                if status.is_success() {
                    emit_upstream_perf_log(
                        &app,
                        &method,
                        &path_query,
                        &target.label,
                        &effective_transport_mode,
                        Some(status.as_u16()),
                        upstream_started.elapsed().as_millis(),
                    );
                    if routing_strategy == "round-robin" {
                        advance_current_idx(&current_idx, total_accounts);
                    }
                    clear_account_quota_error_marker(&accounts, selected_idx);
                    let is_sse = path_query.contains("alt=sse")
                        || resp
                            .headers()
                            .get("content-type")
                            .and_then(|v| v.to_str().ok())
                            .map(|v| v.contains("text/event-stream"))
                            .unwrap_or(false);

                    if is_sse {
                        let upstream_section = upstream_request_preview_for_flow
                            .as_deref()
                            .map(|s| rewrite_request_preview_protocol(s, &upstream_protocol))
                            .unwrap_or_else(|| "<unavailable>".to_string());
                        let diag_block = build_flow_diag_block(
                            official_ls_enabled,
                            official_ls_running,
                            &http_protocol_mode,
                            &upstream_server,
                            &upstream_custom_url,
                            model_name.as_deref(),
                            attempt_model_for_usage.as_deref(),
                            &selected_project,
                            project_resource.as_deref(),
                            capacity_failover_enabled,
                            &upstream_attempts_for_flow,
                        );
                        let flow_summary = format!(
                            "{}

UPSTREAM REQUEST (sent):
{}

RESULT
mode: {}
account: {}
target: {}
status: {}
protocol: {}
elapsed_ms: {}
response: stream (SSE)

{}",
                            request_flow_base,
                            &upstream_section,
                            effective_transport_mode,
                            selected_email,
                            target.label,
                            status.as_u16(),
                            upstream_protocol,
                            request_started.elapsed().as_millis(),
                            diag_block
                        );
                        let upstream_response_detail = format!(
                            "UPSTREAM RESPONSE\nmode: {}\naccount: {}\ntarget: {}\nstatus: {}\nprotocol: {}\nelapsed_ms: {}\nresponse: stream (SSE)",
                            effective_transport_mode,
                            selected_email,
                            target.label,
                            status.as_u16(),
                            upstream_protocol,
                            request_started.elapsed().as_millis()
                        );
                        let local_response_detail = format!(
                            "LOCAL -> IDE RESPONSE\nstatus: {}\nelapsed_ms: {}\nresponse: stream (SSE)",
                            status.as_u16(),
                            request_started.elapsed().as_millis()
                        );
                        emit_request_summary_log(
                            &app,
                            &method,
                            &path_query,
                            &effective_transport_mode,
                            &selected_email,
                            &target.label,
                            status.as_u16(),
                            request_started.elapsed().as_millis(),
                            FlowDetailBundle {
                                summary: Some(flow_summary),
                                client_to_local: Some(request_flow_base.clone()),
                                local_to_upstream: Some(format!(
                                    "UPSTREAM REQUEST\n{}",
                                    &upstream_section
                                )),
                                upstream_to_local: Some(upstream_response_detail),
                                local_to_client: Some(local_response_detail),
                            },
                            &flow_id,
                            &flow_timestamp,
                        );
                        let recorder = Arc::new(Mutex::new(UsageRecorder {
                            collected: Vec::new(),
                            app: app.clone(),
                            email: selected_email.clone(),
                            model: attempt_model_for_usage.clone().unwrap_or_default(),
                            flow_id: flow_id.clone(),
                        }));
                        let recorder_ref = recorder.clone();
                        let byte_stream = resp.bytes_stream().map(move |result| {
                            if let Ok(ref bytes) = result {
                                if let Ok(mut rec) = recorder_ref.lock() {
                                    rec.collected.extend_from_slice(bytes);
                                }
                            }
                            result
                                .map(Frame::data)
                                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
                        });
                        let stream_body = StreamBody::new(byte_stream);
                        return Ok(builder.body(BodyExt::boxed(stream_body)).unwrap());
                    }

                    let resp_body = resp.bytes().await.unwrap_or_default();
                    record_non_sse_usage(&app, &resp_body, &selected_email, &attempt_model_for_usage);
                    let resp_body_for_flow = format_flow_body_bytes(resp_body.as_ref());
                    let upstream_section = upstream_request_preview_for_flow
                        .as_deref()
                        .map(|s| rewrite_request_preview_protocol(s, &upstream_protocol))
                        .unwrap_or_else(|| "<unavailable>".to_string());
                    let diag_block = build_flow_diag_block(
                        official_ls_enabled,
                        official_ls_running,
                        &http_protocol_mode,
                        &upstream_server,
                        &upstream_custom_url,
                        model_name.as_deref(),
                        attempt_model_for_usage.as_deref(),
                        &selected_project,
                        project_resource.as_deref(),
                        capacity_failover_enabled,
                        &upstream_attempts_for_flow,
                    );
                    let flow_summary = format!(
                        "{}

UPSTREAM REQUEST (sent):
{}

RESULT
mode: {}
account: {}
target: {}
status: {}
protocol: {}
elapsed_ms: {}
response_body:
{}

{}",
                        request_flow_base,
                        &upstream_section,
                        effective_transport_mode,
                        selected_email,
                        target.label,
                        status.as_u16(),
                        upstream_protocol,
                        request_started.elapsed().as_millis(),
                        resp_body_for_flow,
                        diag_block
                    );
                    let upstream_response_detail = format!(
                        "UPSTREAM RESPONSE\nmode: {}\naccount: {}\ntarget: {}\nstatus: {}\nprotocol: {}\nelapsed_ms: {}\nresponse_body:\n{}",
                        effective_transport_mode,
                        selected_email,
                        target.label,
                        status.as_u16(),
                        upstream_protocol,
                        request_started.elapsed().as_millis(),
                        resp_body_for_flow
                    );
                    let local_response_detail = format!(
                        "LOCAL -> IDE RESPONSE\nstatus: {}\nelapsed_ms: {}\nresponse_body:\n{}",
                        status.as_u16(),
                        request_started.elapsed().as_millis(),
                        resp_body_for_flow
                    );
                    emit_request_summary_log(
                        &app,
                        &method,
                        &path_query,
                        &effective_transport_mode,
                        &selected_email,
                        &target.label,
                        status.as_u16(),
                        request_started.elapsed().as_millis(),
                        FlowDetailBundle {
                            summary: Some(flow_summary),
                            client_to_local: Some(request_flow_base.clone()),
                            local_to_upstream: Some(format!(
                                "UPSTREAM REQUEST\n{}",
                                &upstream_section
                            )),
                            upstream_to_local: Some(upstream_response_detail),
                            local_to_client: Some(local_response_detail),
                        },
                        &flow_id,
                        &flow_timestamp,
                    );
                    return Ok(builder.body(full_body(resp_body)).unwrap());
                }

                let content_encoding = resp
                    .headers()
                    .get("content-encoding")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let content_type = resp
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let resp_body = resp.bytes().await.unwrap_or_default();
                if capacity_failover_enabled
                    && !is_last_capacity_attempt
                    && is_model_capacity_exhausted_error(status, resp_body.as_ref())
                {
                    let current_model = attempt_model_for_usage
                        .clone()
                        .unwrap_or_else(|| "-".to_string());
                    let next_model = capacity_attempts
                        .get(attempt_idx + 1)
                        .and_then(|v| v.model_for_usage.clone())
                        .or_else(|| model_name.clone())
                        .unwrap_or_else(|| "-".to_string());
                    upstream_attempts_for_flow.push(format!(
                        "#{} capacity_retry_next={} next_model={}",
                        upstream_attempt_seq,
                        attempt_model_text.as_str(),
                        next_model
                    ));
                    emit_log(
                        &app,
                        &format!(
                            "容量重试 [{}] attempt {}/{} model {} -> {}",
                            target.label,
                            attempt_idx + 1,
                            capacity_attempts_len,
                            current_model,
                            next_model
                        ),
                        "warning",
                        None,
                    );
                    continue;
                }
                if capacity_failover_enabled
                    && path_query.contains(":streamGenerateContent")
                    && !is_last_capacity_attempt
                    && is_retryable_internal_error(status, resp_body.as_ref())
                {
                    let current_model = attempt_model_for_usage
                        .clone()
                        .unwrap_or_else(|| "-".to_string());
                    let next_model = capacity_attempts
                        .get(attempt_idx + 1)
                        .and_then(|v| v.model_for_usage.clone())
                        .or_else(|| model_name.clone())
                        .unwrap_or_else(|| "-".to_string());
                    upstream_attempts_for_flow.push(format!(
                        "#{} internal_retry_next={} next_model={}",
                        upstream_attempt_seq,
                        attempt_model_text.as_str(),
                        next_model
                    ));
                    let backoff_ms = 250u64.saturating_mul((attempt_idx + 1) as u64);
                    emit_log(
                        &app,
                        &format!(
                            "内部错误重试 [{}] attempt {}/{} model {} -> {} backoff={}ms",
                            target.label,
                            attempt_idx + 1,
                            capacity_attempts_len,
                            current_model,
                            next_model,
                            backoff_ms
                        ),
                        "warning",
                        None,
                    );
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                    continue;
                }
                if capacity_failover_enabled
                    && path_query.contains(":streamGenerateContent")
                    && status.as_u16() == 404
                    && !requested_model_norm.is_empty()
                    && !attempt_model_norm.is_empty()
                    && attempt_model_norm != requested_model_norm
                {
                    blocked_capacity_model_norm = Some(attempt_model_norm.clone());
                    upstream_attempts_for_flow.push(format!(
                        "#{} fallback_model_not_found={} action=skip_this_model_for_request",
                        upstream_attempt_seq,
                        attempt_model_text.as_str()
                    ));
                    emit_log(
                        &app,
                        &format!(
                            "模型回退命中404，标记本次请求不再使用该模型 [{}] model={}",
                            target.label,
                            attempt_model_text.as_str()
                        ),
                        "warning",
                        None,
                    );
                    continue;
                }

                let is_text_like = content_type.starts_with("text/")
                    || content_type.contains("json")
                    || content_type.contains("xml");
                let preview = if !content_encoding.is_empty() {
                    format!(
                        "<compressed body omitted: encoding={}, bytes={}>",
                        content_encoding,
                        resp_body.len()
                    )
                } else if !is_text_like {
                    format!(
                        "<non-text body omitted: content-type={}, bytes={}>",
                        content_type,
                        resp_body.len()
                    )
                } else {
                    String::from_utf8_lossy(&resp_body)
                        .chars()
                        .take(220)
                        .collect()
                };
                let kind = classify_error_kind_from_status(status.as_u16(), &path_query, &preview);
                emit_upstream_perf_log(
                    &app,
                    &method,
                    &path_query,
                    &target.label,
                    &effective_transport_mode,
                    Some(status.as_u16()),
                    upstream_started.elapsed().as_millis(),
                );
                // Note: do NOT mark account as errored here.
                // Proxy forwarding failures are usually platform-side issues,
                // not account-specific. Only quota refresh errors in quota.rs mark accounts.
                emit_log(
                    &app,
                    &format!(
                        "{} [{} {}] -> {} ({} {}), body={}",
                        if is_primary_ls_target {
                            "LS返回非2xx，停止当前目标回退"
                        } else {
                            "转发非2xx，尝试下一个目标"
                        },
                        method,
                        path_query,
                        target.label,
                        status,
                        kind,
                        preview
                    ),
                    "warning",
                    None,
                );
                let resp_body_for_flow = format_flow_body_bytes(resp_body.as_ref());
                let upstream_section = upstream_request_preview_for_flow
                    .as_deref()
                    .map(|s| rewrite_request_preview_protocol(s, &upstream_protocol))
                    .unwrap_or_else(|| "<unavailable>".to_string());
                let diag_block = build_flow_diag_block(
                    official_ls_enabled,
                    official_ls_running,
                    &http_protocol_mode,
                    &upstream_server,
                    &upstream_custom_url,
                    model_name.as_deref(),
                    attempt_model_for_usage.as_deref(),
                    &selected_project,
                    project_resource.as_deref(),
                    capacity_failover_enabled,
                    &upstream_attempts_for_flow,
                );
                let summary = format!(
                    "{}

UPSTREAM REQUEST (sent):
{}

RESULT
mode: {}
account: {}
target: {}
status: {}
protocol: {}
elapsed_ms: {}
response_body:
{}

{}",
                    request_flow_base,
                    &upstream_section,
                    effective_transport_mode,
                    selected_email,
                    target.label,
                    status.as_u16(),
                    upstream_protocol,
                    request_started.elapsed().as_millis(),
                    resp_body_for_flow,
                    diag_block
                );
                let upstream_response_detail = format!(
                    "UPSTREAM RESPONSE\nmode: {}\naccount: {}\ntarget: {}\nstatus: {}\nprotocol: {}\nelapsed_ms: {}\nresponse_body:\n{}",
                    effective_transport_mode,
                    selected_email,
                    target.label,
                    status.as_u16(),
                    upstream_protocol,
                    request_started.elapsed().as_millis(),
                    resp_body_for_flow
                );
                let local_response_detail = format!(
                    "LOCAL -> IDE RESPONSE\nstatus: {}\nelapsed_ms: {}\nresponse_body:\n{}",
                    status.as_u16(),
                    request_started.elapsed().as_millis(),
                    resp_body_for_flow
                );
                let non_success_details = FlowDetailBundle {
                    summary: Some(summary),
                    client_to_local: Some(request_flow_base.clone()),
                    local_to_upstream: Some(format!("UPSTREAM REQUEST\n{}", &upstream_section)),
                    upstream_to_local: Some(upstream_response_detail),
                    local_to_client: Some(local_response_detail),
                };
                let non_success_response = builder.body(full_body(resp_body)).unwrap();

                if is_primary_ls_target {
                    let allow_ls_fallback = matches!(status.as_u16(), 404 | 500 | 502 | 503 | 504);
                    if allow_ls_fallback {
                        emit_log(
                            &app,
                            &format!(
                                "网关在 [{} {}] 返回 {}，回退直连上游目标",
                                status.as_u16(),
                                method,
                                path_query
                            ),
                            "warning",
                            None,
                        );
                    } else {
                        emit_request_summary_log(
                            &app,
                            &method,
                            &path_query,
                            &effective_transport_mode,
                            &selected_email,
                            &target.label,
                            status.as_u16(),
                            request_started.elapsed().as_millis(),
                            non_success_details,
                            &flow_id,
                            &flow_timestamp,
                        );
                        return Ok(non_success_response);
                    }
                }

                pending_non_success_details = Some(non_success_details);
                pending_non_success = Some(non_success_response);
                break;
            }
        }
        if auth_or_rate_limited {
            continue;
        }

        if let Some(resp) = pending_non_success {
            emit_request_summary_log(
                &app,
                &method,
                &path_query,
                &effective_transport_mode,
                &selected_email,
                "non-success",
                resp.status().as_u16(),
                request_started.elapsed().as_millis(),
                pending_non_success_details.unwrap_or_default(),
                &flow_id,
                &flow_timestamp,
            );
            return Ok(resp);
        }

        emit_log(
            &app,
            &format!(
                "账号 [{}] 的所有上游目标均失败，路径 [{}]",
                selected_email, path_query
            ),
            "error",
            None,
        );
        let diag_block = build_flow_diag_block(
            official_ls_enabled,
            official_ls_running,
            &http_protocol_mode,
            &upstream_server,
            &upstream_custom_url,
            model_name.as_deref(),
            None,
            &selected_project,
            project_resource.as_deref(),
            capacity_failover_enabled,
            &upstream_attempts_for_flow,
        );
        emit_request_summary_log(
            &app,
            &method,
            &path_query,
            &effective_transport_mode,
            &selected_email,
            "all-targets-failed",
            502,
            request_started.elapsed().as_millis(),
            FlowDetailBundle {
                summary: Some(format!(
                    "{}

UPSTREAM REQUEST (sent):
{}

RESULT
mode: {}
account: {}
target: all-targets-failed
status: 502
elapsed_ms: {}
reason: 所有上游目标请求失败

{}",
                    request_flow_base,
                    upstream_request_preview_for_flow.as_deref().unwrap_or("<unavailable>"),
                    effective_transport_mode,
                    selected_email,
                    request_started.elapsed().as_millis(),
                    diag_block
                )),
                client_to_local: Some(request_flow_base.clone()),
                local_to_upstream: Some(format!(
                    "UPSTREAM REQUEST\n{}",
                    upstream_request_preview_for_flow.as_deref().unwrap_or("<unavailable>")
                )),
                upstream_to_local: Some(format!(
                    "UPSTREAM RESPONSE
mode: {}
account: {}
target: all-targets-failed
status: 502
elapsed_ms: {}
reason: 所有上游目标请求失败",
                    effective_transport_mode,
                    selected_email,
                    request_started.elapsed().as_millis()
                )),
                local_to_client: Some(format!(
                    "LOCAL -> IDE RESPONSE
status: 502
elapsed_ms: {}
reason: 所有上游目标请求失败",
                    request_started.elapsed().as_millis()
                )),
            },
            &flow_id,
            &flow_timestamp,
        );
        return Ok(http::Response::builder()
            .status(502)
            .body(full_body(Bytes::from("所有上游目标请求失败")))
            .unwrap());
    }

    if request_needs_project && !attempted_upstream_forward && !skipped_for_missing_project.is_empty() {
        let reason = format!(
            "当前请求需要 project_id，但所有账号均缺失: {}",
            skipped_for_missing_project.join(", ")
        );
        emit_log(
            &app,
            &format!("请求失败 [{} {}]: {}", method, path_query, reason),
            "error",
            None,
        );
        emit_request_summary_log(
            &app,
            &method,
            &path_query,
            &effective_transport_mode,
            "-",
            "local-project-validation",
            400,
            request_started.elapsed().as_millis(),
            FlowDetailBundle {
                summary: Some(format!(
                    "{}\n\nRESULT\nmode: {}\naccount: -\ntarget: local-project-validation\nstatus: 400\nelapsed_ms: {}\nreason: {}",
                    request_flow_base,
                    effective_transport_mode,
                    request_started.elapsed().as_millis(),
                    reason
                )),
                client_to_local: Some(request_flow_base.clone()),
                local_to_upstream: None,
                upstream_to_local: Some(format!(
                    "UPSTREAM RESPONSE\nmode: {}\naccount: -\ntarget: local-project-validation\nstatus: 400\nelapsed_ms: {}\nreason: {}",
                    effective_transport_mode,
                    request_started.elapsed().as_millis(),
                    reason
                )),
                local_to_client: Some(format!(
                    "LOCAL -> IDE RESPONSE\nstatus: 400\nelapsed_ms: {}\nresponse_body:\n{{\"error\":{{\"code\":400,\"message\":\"缺少 project id: 请求包含 projects/ 占位符\",\"status\":\"INVALID_ARGUMENT\"}}}}",
                    request_started.elapsed().as_millis()
                )),
            },
            &flow_id,
            &flow_timestamp,
        );
        return Ok(http::Response::builder()
            .status(400)
            .header("Content-Type", "application/json")
            .body(full_body(Bytes::from(
                "{\"error\":{\"code\":400,\"message\":\"缺少 project id: 请求包含 projects/ 占位符\",\"status\":\"INVALID_ARGUMENT\"}}",
            )))
            .unwrap());
    }

    emit_log(
        &app,
        &format!(
            "路径 [{} {}] 没有可用账号或全部账号被阻止",
            method, path_query
        ),
        "error",
        None,
    );
    emit_request_summary_log(
        &app,
        &method,
        &path_query,
        &effective_transport_mode,
        "-",
        "no-available-account",
        502,
        request_started.elapsed().as_millis(),
        FlowDetailBundle {
            summary: Some(format!(
                "{}

RESULT
mode: {}
account: -
target: no-available-account
status: 502
elapsed_ms: {}
reason: no available account or all accounts blocked",
                request_flow_base,
                effective_transport_mode,
                request_started.elapsed().as_millis()
            )),
            client_to_local: Some(request_flow_base),
            local_to_upstream: None,
            upstream_to_local: Some(format!(
                "UPSTREAM RESPONSE
mode: {}
account: -
target: no-available-account
status: 502
elapsed_ms: {}
reason: no available account or all accounts blocked",
                effective_transport_mode,
                request_started.elapsed().as_millis()
            )),
            local_to_client: Some(format!(
                "LOCAL -> IDE RESPONSE
status: 502
elapsed_ms: {}
reason: no available account or all accounts blocked",
                request_started.elapsed().as_millis()
            )),
        },
        &flow_id,
        &flow_timestamp,
    );
    Ok(http::Response::builder()
        .status(502)
                .body(full_body(Bytes::from("没有可用账号或鉴权失败")))
        .unwrap())
}

// Start local TLS proxy server.
#[tauri::command]
pub async fn start_proxy(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    if *state.proxy_running.lock().unwrap() {
        return Err("代理已在运行".to_string());
    }

    let official_ls_enabled_precheck = *state.official_ls_enabled.lock().unwrap();
    if official_ls_enabled_precheck {
        match crate::ls_bridge::is_official_ls_binary_available() {
            true => {
                emit_log(
                    &app,
                    "official_ls pre-check passed: binary found",
                    "info",
                    None,
                );
            }
            false => {
                emit_log(
                    &app,
                    "official_ls pre-check: binary not found, will use direct upstream",
                    "warning",
                    None,
                );
            }
        }
    }

    let (cert_path, key_path) = ensure_cert_exists()?;

    let cert_file = fs::File::open(cert_path).map_err(|e| e.to_string())?;
    let key_file = fs::File::open(key_path).map_err(|e| e.to_string())?;
    let mut cert_reader = std::io::BufReader::new(cert_file);
    let mut key_reader = std::io::BufReader::new(key_file);

    let certs = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| e.to_string())?
        .ok_or("private key not found in cert files".to_string())?;
    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| e.to_string())?;
    // Advertise both h2 and http/1.1 via ALPN so that IDE/Node.js clients
    // can negotiate HTTP/2 or HTTP/1.1 during TLS handshake.
    server_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(server_config));
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();

    let accounts = state.accounts.clone();
    let current_idx = state.current_idx.clone();
    let proxy_running = state.proxy_running.clone();
    let providers = state.providers.clone();
    let routing_strategy = state.routing_strategy.clone();
    let header_passthrough = state.header_passthrough.clone();
    let official_ls_enabled = state.official_ls_enabled.clone();
    let upstream_server = state.upstream_server.clone();
    let upstream_custom_url = state.upstream_custom_url.clone();
    let http_protocol_mode = state.http_protocol_mode.clone();
    let capacity_failover_enabled = state.capacity_failover_enabled.clone();
    let quota_cache = state.quota_cache.clone();
    let quota_threshold = state.quota_threshold.clone();

    let listen_port = load_port_config().unwrap_or(9527);

    // Use socket2 to set SO_REUSEADDR, allowing bind even if port is in ghost/TIME_WAIT state
    let listener = {
        use socket2::{Socket, Domain, Type, Protocol};
        let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))
            .map_err(|e| format!("创建 socket 失败: {}", e))?;
        socket.set_reuse_address(true)
            .map_err(|e| format!("设置 SO_REUSEADDR 失败: {}", e))?;
        let addr: std::net::SocketAddr = format!("127.0.0.1:{}", listen_port).parse().unwrap();
        socket.bind(&addr.into())
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AddrInUse {
                    format!("端口 {} 已被占用且无法复用。\n请在设置中点击「释放端口」或更换端口后重试。", listen_port)
                } else {
                    format!("无法绑定 127.0.0.1:{}: {}", listen_port, e)
                }
            })?;
        socket.listen(128)
            .map_err(|e| format!("监听失败: {}", e))?;
        socket.set_nonblocking(true)
            .map_err(|e| format!("设置非阻塞失败: {}", e))?;
        let std_listener: std::net::TcpListener = socket.into();
        tokio::net::TcpListener::from_std(std_listener)
            .map_err(|e| format!("转换 tokio listener 失败: {}", e))?
    };

    tokio::spawn(async move {
        *proxy_running.lock().unwrap() = true;
        emit_log(&app, "代理已启动", "success", None);

        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    emit_log(&app, "代理停止中", "warning", None);
                    break;
                }
                accept_res = listener.accept() => {
                    if let Ok((stream, _)) = accept_res {
                        let acceptor = acceptor.clone();
                        let accounts = accounts.clone();
                        let current_idx = current_idx.clone();
                        let app_inner = app.clone();

                        let providers = providers.clone();
                        let routing_strategy = routing_strategy.clone();
                        let header_passthrough = header_passthrough.clone();
                        let official_ls_enabled = official_ls_enabled.clone();
                        let upstream_server = upstream_server.clone();
                        let upstream_custom_url = upstream_custom_url.clone();
                        let http_protocol_mode = http_protocol_mode.clone();
                        let capacity_failover_enabled = capacity_failover_enabled.clone();
                        let quota_cache = quota_cache.clone();
                        let quota_threshold = quota_threshold.clone();

                        tokio::spawn(async move {
                            match acceptor.accept(stream).await {
                                Ok(tls_stream) => {
                                let app_for_service = app_inner.clone();
                                let providers_inner = providers.clone();
                                let routing_strategy_inner = routing_strategy.clone();
                                let header_passthrough_inner = header_passthrough.clone();
                                let official_ls_enabled_inner = official_ls_enabled.clone();
                                let upstream_server_inner = upstream_server.clone();
                                let upstream_custom_url_inner = upstream_custom_url.clone();
                                let http_protocol_mode_inner = http_protocol_mode.clone();
                                let capacity_failover_enabled_inner = capacity_failover_enabled.clone();
                                let quota_cache_inner = quota_cache.clone();
                                let quota_threshold_inner = quota_threshold.clone();

                                let service = service_fn(move |req| {
                                    handle_proxy_request(
                                        app_for_service.clone(),
                                        req,
                                        accounts.clone(),
                                        current_idx.clone(),
                                        providers_inner.clone(),
                                        routing_strategy_inner.clone(),
                                        header_passthrough_inner.clone(),
                                        official_ls_enabled_inner.clone(),
                                        upstream_server_inner.clone(),
                                        upstream_custom_url_inner.clone(),
                                        http_protocol_mode_inner.clone(),
                                        capacity_failover_enabled_inner.clone(),
                                        quota_cache_inner.clone(),
                                        quota_threshold_inner.clone(),
                                    )
                                });

                                if let Err(err) = Builder::new(TokioExecutor::new())
                                    .serve_connection(TokioIo::new(tls_stream), service)
                                    .await
                                {
                                    emit_log(&app_inner, &format!("连接处理失败: {:?}", err), "error", None);
                                }
                                }
                                Err(tls_err) => {
                                    emit_log(&app_inner, &format!("TLS 握手失败: {:?}", tls_err), "error", None);
                                }
                            }
                        });
                    }
                }
            }
        }

        *proxy_running.lock().unwrap() = false;
        emit_log(&app, "代理已停止", "info", None);
    });

    *state.proxy_shutdown_tx.lock().unwrap() = Some(shutdown_tx);
    Ok(format!("代理已启动，监听 127.0.0.1:{}", listen_port))
}

#[tauri::command]
pub fn stop_proxy(state: State<'_, AppState>) -> Result<String, String> {
    if let Some(tx) = state.proxy_shutdown_tx.lock().unwrap().take() {
        let _ = tx.send(());
        Ok("已发送代理停止信号".to_string())
    } else {
        Err("代理未运行".to_string())
    }
}

#[tauri::command]
pub fn save_port_config(proxy_port: u16) -> Result<String, String> {
    if !(1024..=65535).contains(&proxy_port) {
        return Err(format!("无效代理端口: {}", proxy_port));
    }
    let config_path = get_app_data_dir().join("port_config.txt");
    fs::write(&config_path, proxy_port.to_string()).map_err(|e| e.to_string())?;
    Ok(format!("代理端口已保存: {}", proxy_port))
}

#[tauri::command]
pub fn load_port_config() -> Result<u16, String> {
    let config_path = get_app_data_dir().join("port_config.txt");
    let content = fs::read_to_string(&config_path).unwrap_or_else(|_| "9527".to_string());
    let port = content.trim().parse::<u16>().map_err(|e| e.to_string())?;
    if !(1024..=65535).contains(&port) {
        return Err(format!("保存的端口无效: {}", port));
    }
    Ok(port)
}

#[tauri::command]
pub fn kill_port_process(port: u16) -> Result<String, String> {
    // Use netstat to find PID using the port
    let output = std::process::Command::new("netstat")
        .args(&["-ano"])
        .output()
        .map_err(|e| format!("无法执行 netstat: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let target = format!("127.0.0.1:{}", port);
    let target2 = format!("0.0.0.0:{}", port);

    let mut found_pids = Vec::new();
    let mut killed_pids = Vec::new();
    let mut ghost_port = false;

    for line in stdout.lines() {
        if (line.contains(&target) || line.contains(&target2)) && line.contains("LISTENING") {
            if let Some(pid_str) = line.split_whitespace().last() {
                if let Ok(pid) = pid_str.parse::<u32>() {
                    if pid > 0 && !found_pids.contains(&pid) {
                        found_pids.push(pid);
                        // Try to kill the process
                        let kill_result = std::process::Command::new("taskkill")
                            .args(&["/F", "/PID", &pid.to_string()])
                            .output();
                        match kill_result {
                            Ok(r) if r.status.success() => {
                                killed_pids.push(pid);
                            }
                            _ => {
                                // Process might be dead but port is still held (ghost port)
                                ghost_port = true;
                            }
                        }
                    }
                }
            }
        }
    }

    if found_pids.is_empty() {
        return Ok(format!("端口 {} 当前没有被占用", port));
    }

    if !killed_pids.is_empty() {
        // Wait a bit for port to be released
        std::thread::sleep(std::time::Duration::from_millis(500));
        return Ok(format!("已结束占用端口 {} 的进程 (PID: {:?})", port, killed_pids));
    }

    if ghost_port {
        // Ghost port: process is dead but port is stuck. Try resetting winnat.
        let stop = std::process::Command::new("net")
            .args(&["stop", "winnat"])
            .output();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let start = std::process::Command::new("net")
            .args(&["start", "winnat"])
            .output();

        match (stop, start) {
            (Ok(s), Ok(r)) if s.status.success() && r.status.success() => {
                Ok(format!("端口 {} 的幽灵占用已通过重置网络栈释放 (PID {} 已不存在)", port, found_pids[0]))
            }
            _ => {
                Err(format!(
                    "端口 {} 被已退出的进程 (PID {:?}) 锁定，自动释放失败。\n请以管理员身份在命令行执行：\nnet stop winnat && net start winnat\n然后重试。",
                    port, found_pids
                ))
            }
        }
    } else {
        Err(format!("端口 {} 被占用但无法释放 (PID: {:?})", port, found_pids))
    }
}

#[tauri::command]
pub fn get_routing_strategy(state: State<'_, AppState>) -> String {
    state.routing_strategy.lock().unwrap().clone()
}

#[tauri::command]
pub fn set_routing_strategy(
    strategy: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let normalized = match strategy.to_lowercase().trim() {
        "round-robin" | "roundrobin" | "rr" => "round-robin".to_string(),
        "fill" | "f" | "fill-first" | "fillfirst" | "ff" | "" => "fill".to_string(),
        _ => return Err(format!("无效路由策略: {}", strategy)),
    };
    *state.routing_strategy.lock().unwrap() = normalized.clone();
    let config_path = get_app_data_dir().join("routing_strategy.txt");
    fs::write(&config_path, &normalized).ok();
    Ok(normalized)
}

#[tauri::command]
pub fn set_header_passthrough(enabled: bool, state: State<'_, AppState>) -> Result<bool, String> {
    *state.header_passthrough.lock().unwrap() = enabled;
    let config_path = get_app_data_dir().join("header_passthrough.txt");
    fs::write(&config_path, if enabled { "1" } else { "0" }).ok();
    Ok(enabled)
}

#[tauri::command]
pub fn get_header_passthrough(state: State<'_, AppState>) -> bool {
    *state.header_passthrough.lock().unwrap()
}

#[tauri::command]
pub fn set_upstream_server(server: String, state: State<'_, AppState>) -> Result<String, String> {
    let normalized = normalize_upstream_server(&server);
    *state.upstream_server.lock().unwrap() = normalized.clone();
    let config_path = get_app_data_dir().join("upstream_server.txt");
    fs::write(&config_path, &normalized).ok();
    Ok(normalized)
}

#[tauri::command]
pub fn get_upstream_server(state: State<'_, AppState>) -> String {
    state.upstream_server.lock().unwrap().clone()
}

#[tauri::command]
pub fn set_upstream_custom_url(
    custom_url: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let normalized = custom_url.trim().trim_end_matches('/').to_string();
    *state.upstream_custom_url.lock().unwrap() = normalized.clone();
    let config_path = get_app_data_dir().join("upstream_custom_url.txt");
    fs::write(&config_path, &normalized).ok();
    Ok(normalized)
}

#[tauri::command]
pub fn get_upstream_custom_url(state: State<'_, AppState>) -> String {
    state.upstream_custom_url.lock().unwrap().clone()
}

#[tauri::command]
pub fn set_http_protocol_mode(mode: String, state: State<'_, AppState>) -> Result<String, String> {
    let normalized = normalize_http_protocol_mode(&mode);
    *state.http_protocol_mode.lock().unwrap() = normalized.clone();
    let config_path = get_app_data_dir().join("http_protocol_mode.txt");
    fs::write(&config_path, &normalized).ok();
    Ok(normalized)
}

#[tauri::command]
pub fn get_http_protocol_mode(state: State<'_, AppState>) -> String {
    state.http_protocol_mode.lock().unwrap().clone()
}

#[tauri::command]
pub fn set_capacity_failover_enabled(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    *state.capacity_failover_enabled.lock().unwrap() = enabled;
    let config_path = get_app_data_dir().join("capacity_failover_enabled.txt");
    fs::write(&config_path, if enabled { "1" } else { "0" }).ok();
    Ok(enabled)
}

#[tauri::command]
pub fn get_capacity_failover_enabled(state: State<'_, AppState>) -> bool {
    *state.capacity_failover_enabled.lock().unwrap()
}

#[tauri::command]
pub fn get_token_stats(
    state: State<'_, AppState>,
) -> Result<crate::token_stats::GlobalStats, String> {
    Ok(state.token_stats.get_global_stats())
}

#[tauri::command]
pub fn reset_token_stats(state: State<'_, AppState>) -> Result<String, String> {
    state.token_stats.reset();
    Ok("Token 统计已重置".to_string())
}

#[tauri::command]
pub fn flush_token_stats(state: State<'_, AppState>) -> Result<String, String> {
    state.token_stats.flush();
    Ok("Token 统计已写入磁盘".to_string())
}

#[tauri::command]
pub fn get_quota_threshold(state: State<'_, AppState>) -> i32 {
    *state.quota_threshold.lock().unwrap()
}

#[tauri::command]
pub fn set_quota_threshold(threshold: i32, state: State<'_, AppState>) -> Result<i32, String> {
    let valid = match threshold {
        0 | 20 | 40 | 60 | 80 => threshold,
        _ => {
            return Err(format!(
                "无效额度阈值: {} (允许: 0/20/40/60/80)",
                threshold
            ))
        }
    };
    *state.quota_threshold.lock().unwrap() = valid;
    let config_path = get_app_data_dir().join("quota_threshold.txt");
    fs::write(&config_path, valid.to_string()).ok();
    Ok(valid)
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_legacy_targets_returns_three_upstreams() {
        let targets = build_legacy_forward_targets(
            "/v1internal:loadCodeAssist",
            "sandbox",
            "",
        )
        .unwrap();
        assert_eq!(targets.len(), 3);
        assert_eq!(targets[0].label, TARGET_HOST_1);
        assert_eq!(targets[1].label, TARGET_HOST_2);
        assert_eq!(targets[2].label, TARGET_HOST_3);
    }

    #[test]
    fn build_legacy_targets_custom_uses_custom_only() {
        let targets = build_legacy_forward_targets(
            "/v1internal:loadCodeAssist",
            "custom",
            "http://127.0.0.1:18080",
        )
        .unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].label, "custom");
        assert_eq!(
            targets[0].target_url,
            "http://127.0.0.1:18080/v1internal:loadCodeAssist"
        );
    }
}
