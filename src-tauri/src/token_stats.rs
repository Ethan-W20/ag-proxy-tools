use crate::utils::get_app_data_dir;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsageRecord {
    pub timestamp: i64, // Unix timestamp
    pub account_email: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,     // cache read tokens (prompt cache hit)
    pub cache_creation_tokens: u64, // cache creation tokens
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AccountStats {
    pub email: String,
    pub total_input: u64,
    pub total_output: u64,
    pub total_cache_read: u64,
    pub total_cache_creation: u64,
    pub total_tokens: u64,
    pub request_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalStats {
    pub total_input: u64,
    pub total_output: u64,
    pub total_cache_read: u64,
    pub total_cache_creation: u64,
    pub total_tokens: u64,
    pub total_requests: u64,
    pub total_errors: u64,
    pub accounts: Vec<AccountStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct PersistentStats {
    accounts: HashMap<String, AccountStats>,
    recent_records: Vec<TokenUsageRecord>,
    total_errors: u64,
}

#[derive(Clone)]
pub struct TokenStatsManager {
    inner: Arc<Mutex<PersistentStats>>,
}

impl Default for TokenStatsManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenStatsManager {
    pub fn new() -> Self {
        let data = Self::load_from_disk().unwrap_or_default();
        Self {
            inner: Arc::new(Mutex::new(data)),
        }
    }

    pub fn record(&self, record: TokenUsageRecord) {
        let mut data = self.inner.lock().unwrap();

        let acc = data
            .accounts
            .entry(record.account_email.clone())
            .or_insert_with(|| AccountStats {
                email: record.account_email.clone(),
                ..Default::default()
            });
        acc.total_input += record.input_tokens;
        acc.total_output += record.output_tokens;
        acc.total_cache_read += record.cache_read_tokens;
        acc.total_cache_creation += record.cache_creation_tokens;
        acc.total_tokens += record.total_tokens;
        acc.request_count += 1;

        data.recent_records.push(record);
        if data.recent_records.len() > 1000 {
            let excess = data.recent_records.len() - 1000;
            data.recent_records.drain(0..excess);
        }

        let total_requests: u64 = data.accounts.values().map(|a| a.request_count).sum();
        if total_requests.is_multiple_of(10) {
            let snapshot = data.clone();
            drop(data); // Release lock before performing IO
            let _ = Self::save_to_disk(&snapshot);
        }
    }

    pub fn get_global_stats(&self) -> GlobalStats {
        let data = self.inner.lock().unwrap();
        let mut stats = GlobalStats::default();

        for acc in data.accounts.values() {
            stats.total_input += acc.total_input;
            stats.total_output += acc.total_output;
            stats.total_cache_read += acc.total_cache_read;
            stats.total_cache_creation += acc.total_cache_creation;
            stats.total_tokens += acc.total_tokens;
            stats.total_requests += acc.request_count;
            stats.accounts.push(acc.clone());
        }
        stats.total_errors = data.total_errors;

        stats
            .accounts
            .sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
        stats
    }

    pub fn record_error(&self) {
        let mut data = self.inner.lock().unwrap();
        data.total_errors = data.total_errors.saturating_add(1);
        let total_errors = data.total_errors;
        if total_errors <= 3 || total_errors.is_multiple_of(5) {
            let snapshot = data.clone();
            drop(data);
            let _ = Self::save_to_disk(&snapshot);
        }
    }

    pub fn get_recent_records(&self, limit: usize) -> Vec<TokenUsageRecord> {
        let data = self.inner.lock().unwrap();
        let len = data.recent_records.len();
        let start = len.saturating_sub(limit);
        data.recent_records[start..].to_vec()
    }

    pub fn reset(&self) {
        let mut data = self.inner.lock().unwrap();
        *data = PersistentStats::default();
        let snapshot = data.clone();
        drop(data);
        let _ = Self::save_to_disk(&snapshot);
    }

    pub fn flush(&self) {
        let data = self.inner.lock().unwrap();
        let snapshot = data.clone();
        drop(data);
        let _ = Self::save_to_disk(&snapshot);
    }

    fn load_from_disk() -> Option<PersistentStats> {
        let path = get_app_data_dir().join("token_stats.json");
        let content = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&content).ok()
    }

    fn save_to_disk(data: &PersistentStats) -> Result<(), String> {
        let path = get_app_data_dir().join("token_stats.json");
        let content = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
        std::fs::write(&path, content).map_err(|e| e.to_string())
    }
}

pub fn parse_usage_from_gemini(json: &serde_json::Value) -> Option<(u64, u64, u64, u64)> {
    let usage = json
        .get("usageMetadata")
        .or_else(|| json.get("response").and_then(|r| r.get("usageMetadata")))?;

    let input = usage
        .get("promptTokenCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output = usage
        .get("candidatesTokenCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cached = usage
        .get("cachedContentTokenCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let total = usage
        .get("totalTokenCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(input + output);

    if total == 0 && input == 0 && output == 0 {
        return None;
    }

    Some((input, output, cached, total))
}

pub fn parse_usage_from_claude(json: &serde_json::Value) -> Option<(u64, u64, u64, u64, u64)> {
    let usage = json.get("usage")?;

    let input = usage
        .get("input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output = usage
        .get("output_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cache_read = usage
        .get("cache_read_input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cache_creation = usage
        .get("cache_creation_input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let total = input + output;

    if total == 0 && input == 0 && output == 0 {
        return None;
    }

    Some((input, output, cache_read, cache_creation, total))
}

pub fn parse_usage_from_openai(json: &serde_json::Value) -> Option<(u64, u64, u64, u64)> {
    let usage = json.get("usage")?;

    let input = usage
        .get("prompt_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output = usage
        .get("completion_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cached = usage
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let total = usage
        .get("total_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(input + output);

    Some((input, output, cached, total))
}

pub fn parse_usage_auto(json: &serde_json::Value) -> Option<(u64, u64, u64, u64, u64)> {
    if let Some((input, output, cached, total)) = parse_usage_from_gemini(json) {
        return Some((input, output, cached, 0, total));
    }

    if let Some(result) = parse_usage_from_claude(json) {
        return Some(result);
    }

    if let Some((input, output, cached, total)) = parse_usage_from_openai(json) {
        return Some((input, output, cached, 0, total));
    }

    None
}

pub fn extract_usage_from_sse(sse_data: &[u8]) -> Option<(u64, u64, u64, u64, u64)> {
    let text = std::str::from_utf8(sse_data).ok()?;

    for line in text.lines().rev() {
        if !line.starts_with("data: ") {
            continue;
        }
        let json_str = line.trim_start_matches("data: ").trim();
        if json_str == "[DONE]" {
            continue;
        }
        if !json_str.contains("usage")
            && !json_str.contains("usageMetadata")
            && !json_str.contains("Usage")
        {
            continue;
        }

        if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
            if let Some(result) = parse_usage_auto(&json) {
                return Some(result);
            }

            if let Some(msg) = json.get("message") {
                if let Some(result) = parse_usage_auto(msg) {
                    return Some(result);
                }
            }

            if let Some(delta) = json.get("delta") {
                if let Some(result) = parse_usage_auto(delta) {
                    return Some(result);
                }
            }

            if let Some(resp) = json.get("response") {
                if let Some(result) = parse_usage_auto(resp) {
                    return Some(result);
                }
            }
        }
    }

    None
}

