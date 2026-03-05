use chrono::Utc;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use tauri::{Emitter, State};

use crate::constants::{get_client_id, get_client_secret, AUTH_URL, TOKEN_URL, USERINFO_URL};
use crate::models::{Account, AppState, QuotaErrorInfo};
use crate::proxy::do_refresh_token;
use crate::utils::emit_log;

#[derive(Clone, serde::Serialize)]
struct AccountLoadProgressPayload {
    account: Option<Account>,
    loaded: usize,
    total: usize,
    done: bool,
    run_id: u64,
}

const PRIMARY_ACCOUNT_DATA_DIR: &str = ".antigravity_proxy_tools";
const PRIMARY_ACCOUNTS_DIR: &str = "account";

fn get_home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn get_accounts_dir() -> PathBuf {
    let dir = get_home_dir()
        .join(PRIMARY_ACCOUNT_DATA_DIR)
        .join(PRIMARY_ACCOUNTS_DIR);
    fs::create_dir_all(&dir).ok();
    dir
}

fn get_legacy_accounts_dirs() -> Vec<PathBuf> {
    let home = get_home_dir();
    vec![
        home.join(".antigravity_tools").join("accounts"),
        home.join(".antigravity_cockpit").join("accounts"),
        home.join(".antigravity_proxy_manager").join("account"),
        home.join(".antigravity_proxy_manager").join("accounts"),
    ]
}

fn get_accounts_read_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut seen = HashSet::new();

    for dir in std::iter::once(get_accounts_dir()).chain(get_legacy_accounts_dirs()) {
        if seen.insert(dir.clone()) {
            dirs.push(dir);
        }
    }

    dirs
}

fn collect_json_files_from_dirs(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();

    for dir in dirs {
        if !dir.exists() {
            continue;
        }
        let mut dir_paths: Vec<PathBuf> = Vec::new();
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                dir_paths.push(path);
            }
        }
        dir_paths.sort();
        for path in dir_paths {
            if seen.insert(path.clone()) {
                paths.push(path);
            }
        }
    }

    paths
}

fn collect_primary_account_files() -> Vec<PathBuf> {
    let primary = get_accounts_dir();
    collect_json_files_from_dirs(&[primary])
}

fn collect_legacy_account_files() -> Vec<PathBuf> {
    let legacy = get_legacy_accounts_dirs();
    collect_json_files_from_dirs(&legacy)
}

fn save_account_to_disk(account: &Account) -> Result<(), String> {
    let dir = get_accounts_dir();
    let file_name = format!("{}.json", account.email.replace("@", "_at_"));
    let path = dir.join(file_name);
    let json = serde_json::json!({
        "email": account.email,
        "refresh_token": account.refresh_token,
        "access_token": account.access_token,
        "expiry_timestamp": account.expiry_timestamp,
        "project_id": account.project,
        "disabled": account.disabled,
        "disabled_reason": account.disabled_reason,
        "disabled_at": account.disabled_at,
        "quota_error": account.quota_error,
    });
    fs::write(&path, serde_json::to_string_pretty(&json).unwrap())
        .map_err(|e| format!("保存账号文件失败: {}", e))
}

pub(crate) fn persist_account(account: &Account) -> Result<(), String> {
    save_account_to_disk(account)
}

fn parse_quota_error_from_json(json: &serde_json::Value) -> Option<QuotaErrorInfo> {
    let quota_error = json.get("quota_error")?;
    if quota_error.is_null() {
        return None;
    }

    let message = quota_error
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if message.is_empty() {
        return None;
    }

    let kind = quota_error
        .get("kind")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let code = quota_error
        .get("code")
        .and_then(|v| v.as_u64())
        .and_then(|v| u16::try_from(v).ok());
    let timestamp = quota_error
        .get("timestamp")
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| Utc::now().timestamp());

    Some(QuotaErrorInfo {
        kind,
        code,
        message,
        timestamp,
    })
}

fn delete_account_from_disk(email: &str) {
    let file_name = format!("{}.json", email.replace("@", "_at_"));
    for dir in get_accounts_read_dirs() {
        let _ = fs::remove_file(dir.join(&file_name));
    }
}

fn parse_account_from_json(json: &serde_json::Value, fallback_email: &str) -> Option<Account> {
    let refresh_token = json
        .get("refresh_token")
        .and_then(|r| r.as_str())
        .or_else(|| {
            json.get("token")
                .and_then(|t| t.get("refresh_token"))
                .and_then(|r| r.as_str())
        })
        .unwrap_or("")
        .to_string();
    if refresh_token.is_empty() {
        return None;
    }

    let email = json
        .get("email")
        .and_then(|e| e.as_str())
        .unwrap_or(fallback_email)
        .to_string();
    let project = json
        .get("project_id")
        .and_then(|p| p.as_str())
        .or_else(|| {
            json.get("token")
                .and_then(|t| t.get("project_id"))
                .and_then(|p| p.as_str())
        })
        .unwrap_or("")
        .to_string();
    let access_token = json
        .get("access_token")
        .and_then(|a| a.as_str())
        .unwrap_or("")
        .to_string();
    let raw_expiry = json
        .get("expiry_timestamp")
        .and_then(|e| e.as_i64())
        .or_else(|| json.get("timestamp").and_then(|e| e.as_i64()))
        .unwrap_or(0);
    let expiry = if raw_expiry > 10_000_000_000 {
        raw_expiry / 1000
    } else {
        raw_expiry
    };
    let disabled = json
        .get("disabled")
        .and_then(|d| d.as_bool())
        .unwrap_or(false);

    Some(Account {
        id: json
            .get("id")
            .and_then(|i| i.as_str())
            .map(|s| s.to_string()),
        email,
        project,
        refresh_token,
        access_token,
        expiry_timestamp: expiry,
        disabled,
        disabled_reason: if disabled {
            json.get("disabled_reason")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        } else {
            None
        },
        disabled_at: if disabled {
            json.get("disabled_at").and_then(|v| v.as_i64())
        } else {
            None
        },
        quota_error: parse_quota_error_from_json(json),
    })
}
#[tauri::command]
pub fn load_credentials(state: State<'_, AppState>) -> Result<Vec<Account>, String> {
    let mut all_accounts = Vec::new();
    let mut seen_emails = HashSet::new();

    for path in collect_primary_account_files() {
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let json: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let fallback = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .replace("_at_", "@");
        if let Some(acc) = parse_account_from_json(&json, &fallback) {
            if !seen_emails.insert(acc.email.clone()) {
                continue;
            }
            all_accounts.push(acc);
        }
    }

    all_accounts.sort_by(|a, b| a.email.cmp(&b.email));
    let mut accounts_lock = state.accounts.lock().unwrap();
    *accounts_lock = all_accounts.clone();
    if !all_accounts.is_empty() && *state.current_idx.lock().unwrap() < 0 {
        *state.current_idx.lock().unwrap() = 0;
    }
    Ok(all_accounts)
}

#[tauri::command]
pub fn load_credentials_stream(
    run_id: u64,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<u64, String> {
    let accounts_arc = state.accounts.clone();
    let current_idx_arc = state.current_idx.clone();

    tauri::async_runtime::spawn(async move {
        {
            let mut accounts = accounts_arc.lock().unwrap();
            accounts.clear();
        }
        *current_idx_arc.lock().unwrap() = -1;

        let _ = app.emit(
            "accounts-load-progress",
            AccountLoadProgressPayload {
                account: None,
                loaded: 0,
                total: 0,
                done: false,
                run_id,
            },
        );

        let paths: Vec<PathBuf> = collect_primary_account_files();
        let total = paths.len();
        let _ = app.emit(
            "accounts-load-progress",
            AccountLoadProgressPayload {
                account: None,
                loaded: 0,
                total,
                done: false,
                run_id,
            },
        );

        let mut loaded = 0usize;
        let mut seen_emails: HashSet<String> = HashSet::new();

        for path in paths {
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let json: serde_json::Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let fallback = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .replace("_at_", "@");
            let Some(account) = parse_account_from_json(&json, &fallback) else {
                continue;
            };
            if !seen_emails.insert(account.email.clone()) {
                continue;
            }

            loaded += 1;
            {
                let mut accounts = accounts_arc.lock().unwrap();
                accounts.push(account.clone());
            }
            if *current_idx_arc.lock().unwrap() < 0 {
                *current_idx_arc.lock().unwrap() = 0;
            }

            let _ = app.emit(
                "accounts-load-progress",
                AccountLoadProgressPayload {
                    account: Some(account),
                    loaded,
                    total,
                    done: false,
                    run_id,
                },
            );
        }

        let _ = app.emit(
            "accounts-load-progress",
            AccountLoadProgressPayload {
                account: None,
                loaded,
                total,
                done: true,
                run_id,
            },
        );
    });

    Ok(run_id)
}

#[tauri::command]
pub fn switch_account(index: i32, state: State<'_, AppState>) -> Result<String, String> {
    let accounts = state.accounts.lock().unwrap();
    if index < 0 || index >= accounts.len() as i32 {
        return Err("invalid account index".to_string());
    }
    drop(accounts);
    *state.current_idx.lock().unwrap() = index;
    Ok("ok".to_string())
}

#[tauri::command]
pub fn delete_account(index: i32, state: State<'_, AppState>) -> Result<String, String> {
    let mut accounts = state.accounts.lock().unwrap();
    if index < 0 || index >= accounts.len() as i32 {
        return Err("invalid account index".to_string());
    }
    let removed = accounts.remove(index as usize);
    delete_account_from_disk(&removed.email);
    let mut idx = state.current_idx.lock().unwrap();
    if accounts.is_empty() {
        *idx = -1;
    } else if index < *idx {
        *idx -= 1;
    } else if *idx >= accounts.len() as i32 {
        *idx = accounts.len() as i32 - 1;
    }
    Ok("ok".to_string())
}

#[tauri::command]
pub fn toggle_account_disabled(
    index: i32,
    disabled: bool,
    state: State<'_, AppState>,
) -> Result<Vec<Account>, String> {
    let mut accounts = state.accounts.lock().unwrap();
    if index < 0 || index >= accounts.len() as i32 {
        return Err("invalid account index".to_string());
    }
    let account = &mut accounts[index as usize];
    account.disabled = disabled;
    if disabled {
        account.disabled_reason = Some("鎵嬪姩绂佺敤".to_string());
        account.disabled_at = Some(Utc::now().timestamp());
    } else {
        account.disabled_reason = None;
        account.disabled_at = None;
    }
    let account_clone = account.clone();
    drop(accounts);
    let _ = persist_account(&account_clone);
    let accounts = state.accounts.lock().unwrap();
    Ok(accounts.clone())
}

fn import_single_json_file(
    path: &std::path::Path,
    app: &tauri::AppHandle,
    state: &State<'_, AppState>,
) -> bool {
    let file_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    if !file_name.ends_with(".json") {
        return false;
    }
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
        let items: Vec<serde_json::Value> = if json.is_array() {
            json.as_array().cloned().unwrap_or_default()
        } else {
            vec![json]
        };
        let mut any_imported = false;
        for item in &items {
            let fallback = file_name
                .trim_start_matches("antigravity-")
                .trim_end_matches(".json");
            if let Some(acc) = parse_account_from_json(item, fallback) {
                {
                    let accounts = state.accounts.lock().unwrap();
                    if accounts.iter().any(|a| a.email == acc.email) {
                        continue;
                    }
                }
                if let Err(e) = save_account_to_disk(&acc) {
                    emit_log(
                        app,
                        &format!("保存 {} 失败: {}", acc.email, e),
                        "warning",
                        None,
                    );
                    continue;
                }
                emit_log(app, &format!("已导入 {}", acc.email), "success", None);
                state.accounts.lock().unwrap().push(acc);
                any_imported = true;
            }
        }
        return any_imported;
    }
    let mut any_imported = false;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with('{') {
            continue;
        }
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            let fallback = file_name
                .trim_start_matches("antigravity-")
                .trim_end_matches(".json");
            if let Some(acc) = parse_account_from_json(&json, fallback) {
                {
                    let accounts = state.accounts.lock().unwrap();
                    if accounts.iter().any(|a| a.email == acc.email) {
                        continue;
                    }
                }
                if let Err(e) = save_account_to_disk(&acc) {
                    emit_log(
                        app,
                        &format!("保存 {} 失败: {}", acc.email, e),
                        "warning",
                        None,
                    );
                    continue;
                }
                emit_log(app, &format!("已导入 {}", acc.email), "success", None);
                state.accounts.lock().unwrap().push(acc);
                any_imported = true;
            }
        }
    }
    any_imported
}

#[tauri::command]
pub fn import_credential_files(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<i32, String> {
    let files = rfd::FileDialog::new()
        .set_title("选择凭证文件 (JSON)")
        .add_filter("JSON 凭证文件", &["json", "jsonl", "txt"])
        .pick_files();

    let mut json_paths: Vec<std::path::PathBuf> = Vec::new();

    if let Some(selected_files) = files {
        if !selected_files.is_empty() {
            json_paths = selected_files;
        }
    }

    if json_paths.is_empty() {
        let folder = rfd::FileDialog::new()
            .set_title("选择凭证目录 (包含 JSON 文件)")
            .pick_folder();
        if let Some(src_dir) = folder {
            if let Ok(entries) = fs::read_dir(&src_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        json_paths.push(path);
                    }
                }
            }
        } else {
            return Ok(0);
        }
    }

    let before_count = state.accounts.lock().unwrap().len();

    for path in &json_paths {
        import_single_json_file(path, &app, &state);
    }

    let after_count = state.accounts.lock().unwrap().len();
    let actual_new = (after_count - before_count) as i32;

    if actual_new > 0 {
        let mut accounts = state.accounts.lock().unwrap();
        accounts.sort_by(|a, b| a.email.cmp(&b.email));
        if *state.current_idx.lock().unwrap() < 0 {
            *state.current_idx.lock().unwrap() = 0;
        }
    }
    emit_log(
        &app,
        &format!("导入完成: {} 个新账号", actual_new),
        "info",
        None,
    );
    Ok(actual_new)
}

#[tauri::command]
pub fn sync_accounts_from_legacy_projects(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<i32, String> {
    let legacy_files = collect_legacy_account_files();
    if legacy_files.is_empty() {
        emit_log(&app, "No legacy project accounts found", "info", None);
        return Ok(0);
    }

    let mut existing_emails: HashSet<String> = {
        let accounts = state.accounts.lock().unwrap();
        accounts.iter().map(|a| a.email.clone()).collect()
    };
    let mut new_accounts: Vec<Account> = Vec::new();

    for path in legacy_files {
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let json: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let fallback = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .replace("_at_", "@");
        let Some(account) = parse_account_from_json(&json, &fallback) else {
            continue;
        };
        if !existing_emails.insert(account.email.clone()) {
            continue;
        }
        new_accounts.push(account);
    }

    if new_accounts.is_empty() {
        emit_log(&app, "No new accounts to sync from other projects", "info", None);
        return Ok(0);
    }

    let imported = new_accounts.len() as i32;

    for account in &new_accounts {
        let _ = save_account_to_disk(account);
    }

    {
        let mut accounts = state.accounts.lock().unwrap();
        accounts.extend(new_accounts);
        accounts.sort_by(|a, b| a.email.cmp(&b.email));
    }

    if *state.current_idx.lock().unwrap() < 0 {
        let has_accounts = !state.accounts.lock().unwrap().is_empty();
        if has_accounts {
            *state.current_idx.lock().unwrap() = 0;
        }
    }

    emit_log(
        &app,
        &format!("Synced {} account(s) from other projects", imported),
        "success",
        None,
    );
    Ok(imported)
}

#[tauri::command]
pub async fn import_refresh_token(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    refresh_token: String,
) -> Result<String, String> {
    let rt = refresh_token.trim().to_string();
    if rt.is_empty() {
        return Err("Refresh Token 不能为空".to_string());
    }

    emit_log(&app, "正在验证 Refresh Token...", "info", None);
    let (access_token, expiry) = do_refresh_token(&rt).await?;

    emit_log(&app, "正在获取用户信息...", "info", None);
    let client = reqwest::Client::new();
    let resp = client
        .get(USERINFO_URL)
        .bearer_auth(&access_token)
        .send()
        .await
        .map_err(|e| format!("鑾峰彇鐢ㄦ埛淇℃伅澶辫触: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "鑾峰彇鐢ㄦ埛淇℃伅澶辫触: {}",
            resp.text().await.unwrap_or_default()
        ));
    }
    let user_info: serde_json::Value = resp.json().await.map_err(|e| format!("瑙ｆ瀽澶辫触: {}", e))?;
    let email = user_info
        .get("email")
        .and_then(|e| e.as_str())
        .ok_or("鏃犳硶鑾峰彇 email")?
        .to_string();

    {
        let accounts = state.accounts.lock().unwrap();
        if accounts.iter().any(|a| a.email == email) {
            return Err(format!("account {} already exists", email));
        }
    }

    let project_id = match crate::proxy::fetch_project_resource_with_token(&access_token).await {
        Some(resource) => resource.strip_prefix("projects/").unwrap_or(&resource).to_string(),
        None => {
            crate::utils::emit_log(
                &app,
                "Import: could not auto-detect project ID, will retry on first request",
                "warning",
                None,
            );
            String::new()
        }
    };
    let account = Account {
        id: None,
        email: email.clone(),
        project: project_id,
        refresh_token: rt,
        access_token,
        expiry_timestamp: expiry,
        disabled: false,
        disabled_reason: None,
        disabled_at: None,
        quota_error: None,
    };
    save_account_to_disk(&account)?;
    state.accounts.lock().unwrap().push(account);
    {
        let mut accs = state.accounts.lock().unwrap();
        accs.sort_by(|a, b| a.email.cmp(&b.email));
    }
    if *state.current_idx.lock().unwrap() < 0 {
        *state.current_idx.lock().unwrap() = 0;
    }

    emit_log(&app, &format!("璐﹀彿瀵煎叆鎴愬姛: {}", email), "success", None);
    Ok(email)
}

#[tauri::command]
pub async fn start_oauth_login(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let redirect_uri = "http://localhost:19876/callback";
    let client_id = get_client_id()?;
    let scopes = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile";
    let auth_url = url::Url::parse_with_params(
        AUTH_URL,
        &[
            ("client_id", client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("response_type", "code"),
            ("scope", scopes),
            ("access_type", "offline"),
            ("prompt", "consent"),
        ],
    )
    .unwrap()
    .to_string();

    emit_log(&app, "正在打开浏览器授权...", "info", None);
    let _ = open::that(&auth_url);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:19876")
        .await
        .map_err(|e| format!("鏃犳硶鍚姩鍥炶皟鏈嶅姟: {}", e))?;
    emit_log(&app, "绛夊緟鎺堟潈鍥炶皟 (120 绉掕秴鏃?...", "info", None);

    let (stream, _) = tokio::time::timeout(std::time::Duration::from_secs(120), listener.accept())
        .await
        .map_err(|_| "OAuth 鎺堟潈瓒呮椂".to_string())?
        .map_err(|e| format!("杩炴帴澶辫触: {}", e))?;

    let mut buf = vec![0u8; 4096];
    stream
        .readable()
        .await
        .map_err(|e| format!("璇诲彇澶辫触: {}", e))?;
    let n = stream
        .try_read(&mut buf)
        .map_err(|e| format!("璇诲彇澶辫触: {}", e))?;
    let request = String::from_utf8_lossy(&buf[..n]).to_string();

    let code = request
        .lines()
        .next()
        .and_then(|line| {
            let p: Vec<&str> = line.split_whitespace().collect();
            if p.len() >= 2 {
                url::Url::parse(&format!("http://localhost{}", p[1]))
                    .ok()
                    .and_then(|u| {
                        u.query_pairs()
                            .find(|(k, _)| k == "code")
                            .map(|(_, v)| v.to_string())
                    })
            } else {
                None
            }
        })
        .ok_or("鏈幏鍙栧埌 authorization code")?;

    let resp_html = "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body style='font-family:sans-serif;text-align:center;padding:60px'><h1>Authorization successful</h1><p>You can close this window now.</p></body></html>";
    stream.writable().await.ok();
    let _ = stream.try_write(resp_html.as_bytes());
    drop(stream);

    emit_log(&app, "正在换取 Token...", "info", None);
    let client = reqwest::Client::new();
    let client_secret = get_client_secret()?;
    let token_resp = client
        .post(TOKEN_URL)
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .map_err(|e| format!("Token 浜ゆ崲澶辫触: {}", e))?;

    if !token_resp.status().is_success() {
        return Err(format!(
            "Token 浜ゆ崲澶辫触: {}",
            token_resp.text().await.unwrap_or_default()
        ));
    }
    let td: serde_json::Value = token_resp
        .json()
        .await
        .map_err(|e| format!("瑙ｆ瀽澶辫触: {}", e))?;
    let access_token = td
        .get("access_token")
        .and_then(|a| a.as_str())
        .ok_or("缂哄皯 access_token")?
        .to_string();
    let refresh_token = td
        .get("refresh_token")
        .and_then(|r| r.as_str())
        .ok_or("缂哄皯 refresh_token")?
        .to_string();
    let expiry = Utc::now().timestamp()
        + td.get("expires_in")
            .and_then(|e| e.as_i64())
            .unwrap_or(3600);

    let user_resp = client
        .get(USERINFO_URL)
        .bearer_auth(&access_token)
        .send()
        .await
        .map_err(|e| format!("鑾峰彇鐢ㄦ埛淇℃伅澶辫触: {}", e))?;
    let ui: serde_json::Value = user_resp
        .json()
        .await
        .map_err(|e| format!("瑙ｆ瀽澶辫触: {}", e))?;
    let email = ui
        .get("email")
        .and_then(|e| e.as_str())
        .ok_or("缂哄皯 email")?
        .to_string();

    {
        let accounts = state.accounts.lock().unwrap();
        if accounts.iter().any(|a| a.email == email) {
            return Err(format!("account {} already exists", email));
        }
    }

    let project_id = match crate::proxy::fetch_project_resource_with_token(&access_token).await {
        Some(resource) => resource.strip_prefix("projects/").unwrap_or(&resource).to_string(),
        None => {
            crate::utils::emit_log(
                &app,
                "OAuth: could not auto-detect project ID, will retry on first request",
                "warning",
                None,
            );
            String::new()
        }
    };
    let account = Account {
        id: None,
        email: email.clone(),
        project: project_id,
        refresh_token,
        access_token,
        expiry_timestamp: expiry,
        disabled: false,
        disabled_reason: None,
        disabled_at: None,
        quota_error: None,
    };
    save_account_to_disk(&account)?;
    state.accounts.lock().unwrap().push(account);
    {
        let mut accs = state.accounts.lock().unwrap();
        accs.sort_by(|a, b| a.email.cmp(&b.email));
    }
    if *state.current_idx.lock().unwrap() < 0 {
        *state.current_idx.lock().unwrap() = 0;
    }

    emit_log(&app, &format!("OAuth 鐧诲綍鎴愬姛: {}", email), "success", None);
    Ok(email)
}
