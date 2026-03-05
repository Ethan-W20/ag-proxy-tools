// Proxy log formatting and flow trace utilities.
// Extracted from proxy.rs for maintainability.

use crate::models::{FlowHop, RequestFlowPayload};
use crate::utils::{emit_log, emit_request_flow};
use http::header::HeaderMap;

pub(crate) fn emit_upstream_perf_log(
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
pub(crate) struct FlowDetailBundle {
    pub summary: Option<String>,
    pub client_to_local: Option<String>,
    pub local_to_upstream: Option<String>,
    pub upstream_to_local: Option<String>,
    pub local_to_client: Option<String>,
}

pub(crate) fn emit_request_summary_log(
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
    let is_success = (200..300).contains(&(status as i32));
    let is_gateway = mode == "client_gateway" || mode == "gateway";

    let fwd_status = Some(status);
    let ret_status = Some(status);
    let d_client_to_local = flow_details.client_to_local.clone();
    let d_local_to_upstream = flow_details.local_to_upstream.clone();
    let d_upstream_to_local = flow_details.upstream_to_local.clone();
    let d_local_to_client = flow_details.local_to_client.clone();

    let (forward_hops, return_hops) = if is_gateway {
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
                    node: "网关".into(),
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
                    node: "网关".into(),
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
        mode: if is_gateway { "网关".into() } else { "direct".into() },
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

pub(crate) fn format_http_version(version: http::Version) -> &'static str {
    match version {
        http::Version::HTTP_09 => "HTTP/0.9",
        http::Version::HTTP_10 => "HTTP/1.0",
        http::Version::HTTP_11 => "HTTP/1.1",
        http::Version::HTTP_2 => "HTTP/2",
        _ => "HTTP/?",
    }
}

pub(crate) const INTERNAL_UPSTREAM_PROTOCOL_HEADER: &str = "x-ag-upstream-protocol";



pub(crate) fn resolve_observed_upstream_protocol(resp: &reqwest::Response, _transport_mode: &str) -> String {
    format_http_version(resp.version()).to_string()
}

pub(crate) fn rewrite_request_preview_protocol(preview: &str, protocol: &str) -> String {
    let mut parts = preview.splitn(4, ' ');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(method), Some(path), Some(version), Some(rest)) if version.starts_with("HTTP/") => {
            format!("{} {} {} {}", method, path, protocol, rest)
        }
        _ => preview.to_string(),
    }
}

pub(crate) fn format_upstream_request_preview(
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

pub(crate) fn format_flow_body_bytes(body: &[u8]) -> String {
    if body.is_empty() {
        return "<empty>".to_string();
    }

    match std::str::from_utf8(body) {
        Ok(text) => text.to_string(),
        Err(_) => format!("<binary body omitted: {} bytes>", body.len()),
    }
}

pub(crate) fn estimate_flow_body_tokens(body: &[u8]) -> usize {
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

pub(crate) fn header_value_or_dash(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "-".to_string())
}

pub(crate) fn req_header_value_or_dash(headers: &reqwest::header::HeaderMap, name: &str) -> String {
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

pub(crate) fn build_request_compare_details(
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

pub(crate) fn non_empty_or_dash(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "-".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn opt_non_empty_or_dash(value: Option<&str>) -> String {
    value
        .map(non_empty_or_dash)
        .unwrap_or_else(|| "-".to_string())
}

pub(crate) fn format_upstream_setting_for_diag(upstream_server: &str, upstream_custom_url: &str) -> String {
    if crate::proxy::normalize_upstream_server(upstream_server) == "custom" {
        format!("custom({})", non_empty_or_dash(upstream_custom_url))
    } else {
        "sandbox -> daily -> prod".to_string()
    }
}

pub(crate) fn build_flow_diag_block(
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
    format!(
        "DIAG\nmode: direct\nhttp_protocol_mode: {}\nupstream_setting: {}\nrequest_model: {}\neffective_model: {}\naccount_project_raw: {}\nresolved_project_resource: {}\ncapacity_failover_enabled: {}\nupstream_attempts:\n{}",
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

pub(crate) fn format_incoming_headers_for_log(headers: &HeaderMap) -> String {
    headers
        .iter()
        .map(|(key, value)| {
            let val = value.to_str().unwrap_or("<binary>");
            format!("{}: {}", key, mask_header_value(key.as_str(), val))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn format_upstream_headers(headers: &reqwest::header::HeaderMap) -> String {
    headers
        .iter()
        .map(|(key, value)| {
            let val = value.to_str().unwrap_or("<binary>");
            format!("{}: {}", key, mask_header_value(key.as_str(), val))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn verbose_header_logging_enabled() -> bool {
    use std::sync::OnceLock;
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

pub(crate) fn mask_header_value(name: &str, value: &str) -> String {
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