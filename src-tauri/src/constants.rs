use std::fs;
use std::path::{Path, PathBuf};

use crate::utils::get_app_data_dir;

// ==================== Constants ====================

pub const CLIENT_ID_ENV_KEY: &str = "AG_PROXY_GOOGLE_CLIENT_ID";
pub const CLIENT_SECRET_ENV_KEY: &str = "AG_PROXY_GOOGLE_CLIENT_SECRET";

// Default OAuth app credentials used by Antigravity ecosystem clients.
pub const CLIENT_ID_FALLBACK: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
pub const CLIENT_SECRET_FALLBACK: &str = "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";

// Keep compatibility for existing imports.
pub const CLIENT_ID: &str = CLIENT_ID_FALLBACK;
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

// Endpoint order: Sandbox (most stable) -> Daily (fallback) -> Prod (last resort)
// Fixed fallback order, prioritizing stable environments.
pub const TARGET_HOST_1: &str = "daily-cloudcode-pa.sandbox.googleapis.com";
pub const TARGET_HOST_2: &str = "daily-cloudcode-pa.googleapis.com";
pub const TARGET_HOST_3: &str = "cloudcode-pa.googleapis.com";

pub const INJECT_CODE: &str = "process.env.NODE_TLS_REJECT_UNAUTHORIZED='0';";

pub const USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
pub const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";

pub const QUOTA_API_URL: &str =
    "https://cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels";
pub const CLOUD_CODE_BASE_URL: &str = "https://daily-cloudcode-pa.sandbox.googleapis.com";

fn read_non_empty_env(key: &str) -> Option<String> {
    let raw = std::env::var(key).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn read_non_empty_file(path: &Path) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn oauth_file_candidates(file_name: &str) -> Vec<PathBuf> {
    let mut paths = vec![get_app_data_dir().join(file_name)];
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".antigravity_proxy_tools").join(file_name));
        paths.push(home.join(".antigravity_tools").join(file_name));
        paths.push(home.join(".antigravity_cockpit").join(file_name));
        paths.push(home.join(".antigravity").join(file_name));
    }
    paths
}

fn read_oauth_value_from_files(file_name: &str) -> Option<(String, PathBuf)> {
    for path in oauth_file_candidates(file_name) {
        if let Some(value) = read_non_empty_file(&path) {
            return Some((value, path));
        }
    }
    None
}

pub fn get_client_id() -> Result<String, String> {
    if let Some(value) = read_non_empty_env(CLIENT_ID_ENV_KEY) {
        return Ok(value);
    }

    if let Some((value, _path)) = read_oauth_value_from_files("google_client_id.txt") {
        return Ok(value);
    }

    if !CLIENT_ID_FALLBACK.trim().is_empty() {
        return Ok(CLIENT_ID_FALLBACK.to_string());
    }

    let candidate_list = oauth_file_candidates("google_client_id.txt")
        .into_iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");

    Err(format!(
        "Google OAuth client_id not configured. Set env var {}, or write it to one of: {}.",
        CLIENT_ID_ENV_KEY, candidate_list
    ))
}

/// Resolve OAuth Client Secret at runtime:
/// 1) Environment variable `AG_PROXY_GOOGLE_CLIENT_SECRET`
/// 2) Local config file (`~/.antigravity_proxy_manager/google_client_secret.txt`)
/// 3) Legacy config files from old tools
/// 4) Built-in fallback (backward compatible)
pub fn get_client_secret() -> Result<String, String> {
    if let Some(value) = read_non_empty_env(CLIENT_SECRET_ENV_KEY) {
        return Ok(value);
    }

    if let Some((value, _path)) = read_oauth_value_from_files("google_client_secret.txt") {
        return Ok(value);
    }

    if !CLIENT_SECRET_FALLBACK.trim().is_empty() {
        return Ok(CLIENT_SECRET_FALLBACK.to_string());
    }

    let candidate_list = oauth_file_candidates("google_client_secret.txt")
        .into_iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");

    Err(format!(
        "Google OAuth client secret not configured. Set env var {}, or write secret to one of: {}.",
        CLIENT_SECRET_ENV_KEY, candidate_list
    ))
}
