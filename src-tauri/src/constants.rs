use std::fs;

use crate::utils::get_app_data_dir;

// ==================== Constants ====================

pub const CLIENT_ID: &str = "";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const CLIENT_SECRET_ENV_KEY: &str = "AG_PROXY_GOOGLE_CLIENT_SECRET";
/// Fallback is intentionally empty — configure via env var or config file.
pub const CLIENT_SECRET_FALLBACK: &str = "";

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

/// Resolve OAuth Client Secret at runtime:
/// 1) Environment variable `AG_PROXY_GOOGLE_CLIENT_SECRET`
/// 2) Local config file `{app_data_dir}/google_client_secret.txt`
/// 3) Built-in fallback (backward compatible)
pub fn get_client_secret() -> Result<String, String> {
    if let Ok(raw) = std::env::var(CLIENT_SECRET_ENV_KEY) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let secret_file = get_app_data_dir().join("google_client_secret.txt");
    if let Ok(raw) = fs::read_to_string(&secret_file) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    if !CLIENT_SECRET_FALLBACK.trim().is_empty() {
        return Ok(CLIENT_SECRET_FALLBACK.to_string());
    }

    Err(format!(
        "Google OAuth client secret not configured. Set env var {}, or write secret to {}.",
        CLIENT_SECRET_ENV_KEY,
        secret_file.display()
    ))
}
