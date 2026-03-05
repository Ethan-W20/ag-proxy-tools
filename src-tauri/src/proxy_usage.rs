// Proxy token usage recording.
// Extracted from proxy.rs for maintainability.

use chrono::Utc;
use crate::utils::emit_log;

pub(crate) struct UsageRecorder {
    pub collected: Vec<u8>,
    pub app: tauri::AppHandle,
    pub email: String,
    pub model: String,
    pub flow_id: String,
}

impl Drop for UsageRecorder {
    fn drop(&mut self) {
        use tauri::{Emitter, Manager};
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

pub(crate) fn record_non_sse_usage(
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