use chrono::Utc;
use tauri::State;

use crate::constants::{CLOUD_CODE_BASE_URL, QUOTA_API_URL};
use crate::models::{AppState, ModelQuota, QuotaData, QuotaErrorInfo};
use crate::proxy::{
    classify_error_kind_from_message, do_refresh_token, should_disable_account_for_error_kind,
};
use crate::utils::emit_log;

#[derive(Debug, Clone)]
struct QuotaFetchErrorPayload {
    kind: Option<String>,
    code: Option<u16>,
    message: String,
}

#[derive(Debug, Clone)]
struct QuotaFetchPayload {
    quota: QuotaData,
    error: Option<QuotaFetchErrorPayload>,
}

async fn fetch_project_id(access_token: &str, app: &tauri::AppHandle) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("antigravity")
        .build()
        .ok()?;

    let payload = serde_json::json!({
        "metadata": {
            "ideType": "ANTIGRAVITY"
        }
    });

    let resp = client
        .post(format!("{}/v1internal:loadCodeAssist", CLOUD_CODE_BASE_URL))
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .header("x-client-name", "antigravity")
        .header(
            "x-goog-api-client",
            "gl-node/18.18.2 fire/0.8.6 grpc/1.10.x",
        )
        .json(&payload)
        .send()
        .await;

    match resp {
        Ok(r) => {
            if r.status().is_success() {
                if let Ok(data) = r.json::<serde_json::Value>().await {
                    if let Some(project) = data.get("cloudaicompanionProject") {
                        if let Some(pid) = project.as_str() {
                            return Some(pid.to_string());
                        }
                        if let Some(pid) = project.get("id").and_then(|v| v.as_str()) {
                            return Some(pid.to_string());
                        }
                    }
                }
            } else {
                emit_log(
                    app,
                    &format!("loadCodeAssist 返回 {}", r.status()),
                    "warning",
                    None,
                );
            }
        }
        Err(e) => {
            emit_log(
                app,
                &format!("loadCodeAssist 网络错误: {}", e),
                "warning",
                None,
            );
        }
    }

    None
}

async fn do_fetch_quota(
    access_token: &str,
    email: &str,
    app: &tauri::AppHandle,
) -> Result<QuotaFetchPayload, String> {
    let project_id = fetch_project_id(access_token, app).await;
    let final_pid = project_id.unwrap_or_else(|| "bamboo-precept-lgxtn".to_string());

    emit_log(
        app,
        &format!("开始查询账号额度: {} (project {})", email, final_pid),
        "info",
        None,
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let payload = serde_json::json!({
        "project": final_pid
    });

    let max_retries = 3_u32;
    let mut last_error = String::new();

    for attempt in 1..=max_retries {
        match client
            .post(QUOTA_API_URL)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("User-Agent", "antigravity")
            .header("x-client-name", "antigravity")
            .header(
                "x-goog-api-client",
                "gl-node/18.18.2 fire/0.8.6 grpc/1.10.x",
            )
            .json(&payload)
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status().as_u16();

                if status == 403 {
                    let text = response.text().await.unwrap_or_default();
                    let message = if text.trim().is_empty() {
                        "API 返回 HTTP 403 Forbidden".to_string()
                    } else {
                        text
                    };
                    emit_log(
                        app,
                        &format!("额度 API 对账号 {} 返回 403", email),
                        "warning",
                        None,
                    );
                    return Ok(QuotaFetchPayload {
                        quota: QuotaData {
                            models: Vec::new(),
                            last_updated: Utc::now().timestamp(),
                            is_forbidden: true,
                        },
                        error: Some(QuotaFetchErrorPayload {
                            kind: Some(classify_error_kind_from_message(&message)),
                            code: Some(403),
                            message,
                        }),
                    });
                }

                if status >= 400 {
                    let text = response.text().await.unwrap_or_default();
                    last_error = format!("HTTP {} - {}", status, &text[..text.len().min(200)]);
                    if attempt < max_retries {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        continue;
                    }
                    return Err(format!("额度查询失败: {}", last_error));
                }

                let body: serde_json::Value = response
                    .json()
                    .await
                    .map_err(|e| format!("额度响应解析失败: {}", e))?;

                let mut models = Vec::new();
                let mut group_claude: Option<(i32, String)> = None;
                let mut group_flash: Option<(i32, String)> = None;
                let mut group_pro: Option<(i32, String)> = None;
                let mut group_image: Option<(i32, String)> = None;

                if let Some(models_map) = body.get("models").and_then(|m| m.as_object()) {
                    for (name, info) in models_map {
                        let Some(quota_info) = info.get("quotaInfo") else {
                            continue;
                        };

                        let percentage = quota_info
                            .get("remainingFraction")
                            .and_then(|f| f.as_f64())
                            .map(|f| (f * 100.0) as i32)
                            .unwrap_or(0);
                        let reset_time = quota_info
                            .get("resetTime")
                            .and_then(|r| r.as_str())
                            .unwrap_or("")
                            .to_string();

                        let lower = name.to_ascii_lowercase();
                        if lower.starts_with("gemini-3-pro-image")
                            || (lower.contains("image") && lower.contains("gemini"))
                        {
                            let entry = group_image.get_or_insert((percentage, reset_time.clone()));
                            if percentage < entry.0 {
                                *entry = (percentage, reset_time);
                            }
                        } else if lower.contains("flash") {
                            let entry = group_flash.get_or_insert((percentage, reset_time.clone()));
                            if percentage < entry.0 {
                                *entry = (percentage, reset_time);
                            }
                        } else if lower.contains("pro") && !lower.contains("image") {
                            let entry = group_pro.get_or_insert((percentage, reset_time.clone()));
                            if percentage < entry.0 {
                                *entry = (percentage, reset_time);
                            }
                        } else if lower.contains("claude")
                            || lower.contains("opus")
                            || lower.contains("sonnet")
                            || lower.contains("haiku")
                        {
                            let entry =
                                group_claude.get_or_insert((percentage, reset_time.clone()));
                            if percentage < entry.0 {
                                *entry = (percentage, reset_time);
                            }
                        }
                    }
                }

                if let Some((pct, rt)) = group_claude {
                    models.push(ModelQuota {
                        name: "Claude".to_string(),
                        percentage: pct,
                        reset_time: rt,
                    });
                }
                if let Some((pct, rt)) = group_pro {
                    models.push(ModelQuota {
                        name: "Gemini Pro".to_string(),
                        percentage: pct,
                        reset_time: rt,
                    });
                }
                if let Some((pct, rt)) = group_flash {
                    models.push(ModelQuota {
                        name: "Gemini Flash".to_string(),
                        percentage: pct,
                        reset_time: rt,
                    });
                }
                if let Some((pct, rt)) = group_image {
                    models.push(ModelQuota {
                        name: "Gemini Image".to_string(),
                        percentage: pct,
                        reset_time: rt,
                    });
                }

                emit_log(
                    app,
                    &format!(
                        "额度查询成功: {} ({} 个模型分组)",
                        email,
                        models.len()
                    ),
                    "success",
                    None,
                );

                return Ok(QuotaFetchPayload {
                    quota: QuotaData {
                        models,
                        last_updated: Utc::now().timestamp(),
                        is_forbidden: false,
                    },
                    error: None,
                });
            }
            Err(e) => {
                last_error = format!("网络错误: {}", e);
                if attempt < max_retries {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    }

    Err(format!(
        "额度查询重试 {} 次后仍失败: {}",
        max_retries, last_error
    ))
}

fn is_auth_related_quota_error(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("http 401")
        || lower.contains("http 403")
        || lower.contains("unauthorized")
        || lower.contains("unauthenticated")
        || lower.contains("invalid_token")
        || lower.contains("invalid grant")
}

fn is_persistent_account_issue_kind(kind: &str) -> bool {
    matches!(
        kind,
        "auth_invalid_grant" | "auth_verification_required" | "auth_unauthorized"
    )
}

fn set_account_quota_error(
    accounts: &mut [crate::models::Account],
    idx: usize,
    kind: Option<String>,
    code: Option<u16>,
    message: String,
) {
    if idx >= accounts.len() {
        return;
    }
    accounts[idx].quota_error = Some(QuotaErrorInfo {
        kind,
        code,
        message,
        timestamp: Utc::now().timestamp(),
    });
}

fn clear_account_quota_error(accounts: &mut [crate::models::Account], idx: usize) {
    if idx >= accounts.len() {
        return;
    }
    accounts[idx].quota_error = None;
}

fn mark_account_disabled(accounts: &mut [crate::models::Account], idx: usize, reason: String) {
    if idx >= accounts.len() {
        return;
    }
    accounts[idx].disabled = true;
    accounts[idx].disabled_reason = Some(reason);
    accounts[idx].disabled_at = Some(Utc::now().timestamp());
}

fn clear_account_disabled(accounts: &mut [crate::models::Account], idx: usize) {
    if idx >= accounts.len() {
        return;
    }
    accounts[idx].disabled = false;
    accounts[idx].disabled_reason = None;
    accounts[idx].disabled_at = None;
}

#[tauri::command]
pub async fn fetch_quota(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    index: i32,
) -> Result<QuotaData, String> {
    let idx = index as usize;
    let (email, refresh_token, access_token, expiry) = {
        let accounts = state.accounts.lock().unwrap();
        if idx >= accounts.len() {
            return Err("invalid account index".to_string());
        }
        let acc = &accounts[idx];
        (
            acc.email.clone(),
            acc.refresh_token.clone(),
            acc.access_token.clone(),
            acc.expiry_timestamp,
        )
    };

    let now = Utc::now().timestamp();
    let needs_refresh = access_token.is_empty() || expiry < now + 300;
    let mut token = access_token.clone();
    let mut refresh_error: Option<String> = None;

    if needs_refresh {
        match do_refresh_token(&refresh_token).await {
            Ok((new_token, new_expiry)) => {
                let mut account_to_persist = None;
                {
                    let mut accounts = state.accounts.lock().unwrap();
                    if idx < accounts.len() {
                        accounts[idx].access_token = new_token.clone();
                        accounts[idx].expiry_timestamp = new_expiry;
                        clear_account_disabled(&mut accounts, idx);
                        clear_account_quota_error(&mut accounts, idx);
                        account_to_persist = Some(accounts[idx].clone());
                    }
                }
                if let Some(account) = account_to_persist {
                    let _ = crate::account::persist_account(&account);
                }
                token = new_token;
            }
            Err(e) => {
                refresh_error = Some(e.clone());
                let kind = classify_error_kind_from_message(&e);
                let mut account_to_persist = None;
                {
                    let mut accounts = state.accounts.lock().unwrap();
                    if idx < accounts.len() {
                        // If we still have a cached access token, keep account enabled for fallback fetch.
                        if should_disable_account_for_error_kind(&kind) && token.is_empty() {
                            mark_account_disabled(
                                &mut accounts,
                                idx,
                                format!("invalid_grant: {}", e),
                            );
                        }
                        set_account_quota_error(
                            &mut accounts,
                            idx,
                            Some(kind),
                            None,
                            format!("OAuth error: {}", e),
                        );
                        account_to_persist = Some(accounts[idx].clone());
                    }
                }
                if let Some(account) = account_to_persist {
                    let _ = crate::account::persist_account(&account);
                }
                if token.is_empty() {
                    return Err(e);
                }
                emit_log(
                    &app,
                    &format!(
                        "[{}] Token 刷新失败，回退到缓存 access token: {}",
                        email, e
                    ),
                    "warning",
                    None,
                );
            }
        }
    }

    match do_fetch_quota(&token, &email, &app).await {
        Ok(payload) => {
            let QuotaFetchPayload { quota, error } = payload;
            let mut account_to_persist = None;
            {
                let mut accounts = state.accounts.lock().unwrap();
                if idx < accounts.len() {
                    // A successful quota API round-trip means account is still usable.
                    clear_account_disabled(&mut accounts, idx);
                    if let Some(err) = error {
                        set_account_quota_error(
                            &mut accounts,
                            idx,
                            err.kind,
                            err.code,
                            err.message,
                        );
                    } else {
                        clear_account_quota_error(&mut accounts, idx);
                    }
                    account_to_persist = Some(accounts[idx].clone());
                }
            }
            if let Some(account) = account_to_persist {
                let _ = crate::account::persist_account(&account);
            }
            {
                let mut cache = state.quota_cache.lock().unwrap();
                cache.insert(email.clone(), quota.clone());
            }
            Ok(quota)
        }
        Err(fetch_err) => {
            let mut final_err = fetch_err;
            if let Some(refresh_err) = refresh_error {
                if is_auth_related_quota_error(&final_err) {
                    final_err = format!(
                        "额度查询失败：刷新失败且缓存 token 未授权。refresh: {}; fetch: {}",
                        refresh_err, final_err
                    );
                }
            }

            let mut account_to_persist = None;
            {
                let mut accounts = state.accounts.lock().unwrap();
                if idx < accounts.len() {
                    let kind = classify_error_kind_from_message(&final_err);
                    if should_disable_account_for_error_kind(&kind) {
                        mark_account_disabled(
                            &mut accounts,
                            idx,
                            format!("invalid_grant: {}", final_err),
                        );
                    }
                    set_account_quota_error(
                        &mut accounts,
                        idx,
                        Some(kind),
                        None,
                        final_err.clone(),
                    );
                    account_to_persist = Some(accounts[idx].clone());
                }
            }
            if let Some(account) = account_to_persist {
                let _ = crate::account::persist_account(&account);
            }
            Err(final_err)
        }
    }
}

#[tauri::command]
pub async fn fetch_all_quotas(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<(String, QuotaData)>, String> {
    use futures::future::join_all;
    use std::sync::Arc as StdArc;
    use tokio::sync::Semaphore;

    const MAX_CONCURRENT: usize = 5;

    let account_infos: Vec<(usize, String, String, String, i64)> = {
        let accounts = state.accounts.lock().unwrap();
        let quota_cache = state.quota_cache.lock().unwrap();
        accounts
            .iter()
            .enumerate()
            .filter(|(_, a)| {
                if a.disabled {
                    return false;
                }
                if let Some(q) = quota_cache.get(&a.email) {
                    if q.is_forbidden {
                        return false;
                    }
                }
                true
            })
            .map(|(i, a)| {
                (
                    i,
                    a.email.clone(),
                    a.refresh_token.clone(),
                    a.access_token.clone(),
                    a.expiry_timestamp,
                )
            })
            .collect()
    };

    if account_infos.is_empty() {
        return Err("没有可用账号".to_string());
    }

    emit_log(
        &app,
        &format!(
            "开始查询 {} 个账号额度（最大并发 {}）",
            account_infos.len(),
            MAX_CONCURRENT
        ),
        "info",
        None,
    );

    let semaphore = StdArc::new(Semaphore::new(MAX_CONCURRENT));
    let app_handle = StdArc::new(app.clone());
    let accounts_arc = state.accounts.clone();
    let quota_cache_arc = state.quota_cache.clone();

    let tasks: Vec<_> = account_infos
        .into_iter()
        .map(|(idx, email, refresh_token, access_token, expiry)| {
            let semaphore_ref = semaphore.clone();
            let app_ref = app_handle.clone();
            let accounts_ref = accounts_arc.clone();
            let quota_cache_ref = quota_cache_arc.clone();

            async move {
                let _permit = semaphore_ref.acquire().await.unwrap();

                let now = Utc::now().timestamp();
                let mut token = access_token.clone();
                let mut refresh_error: Option<String> = None;

                if access_token.is_empty() || expiry < now + 300 {
                    match do_refresh_token(&refresh_token).await {
                        Ok((new_token, new_expiry)) => {
                            let mut account_to_persist = None;
                            {
                                let mut accounts = accounts_ref.lock().unwrap();
                                if idx < accounts.len() {
                                    accounts[idx].access_token = new_token.clone();
                                    accounts[idx].expiry_timestamp = new_expiry;
                                    clear_account_disabled(&mut accounts, idx);
                                    clear_account_quota_error(&mut accounts, idx);
                                    account_to_persist = Some(accounts[idx].clone());
                                }
                            }
                            if let Some(account) = account_to_persist {
                                let _ = crate::account::persist_account(&account);
                            }
                            token = new_token;
                        }
                        Err(e) => {
                            refresh_error = Some(e.clone());
                            let kind = classify_error_kind_from_message(&e);
                            emit_log(
                                &app_ref,
                                &format!("[{}] Token 刷新失败: {}", email, e),
                                "warning",
                                None,
                            );

                            if access_token.is_empty() {
                                let mut account_to_persist = None;
                                {
                                    let mut accounts = accounts_ref.lock().unwrap();
                                    if idx < accounts.len() {
                                        if should_disable_account_for_error_kind(&kind) {
                                            mark_account_disabled(
                                                &mut accounts,
                                                idx,
                                                format!("invalid_grant: {}", e),
                                            );
                                        }
                                        if is_persistent_account_issue_kind(&kind) {
                                            set_account_quota_error(
                                                &mut accounts,
                                                idx,
                                                Some(kind),
                                                None,
                                                format!("OAuth error: {}", e),
                                            );
                                        } else {
                                            clear_account_quota_error(&mut accounts, idx);
                                        }
                                        account_to_persist = Some(accounts[idx].clone());
                                    }
                                }
                                if let Some(account) = account_to_persist {
                                    let _ = crate::account::persist_account(&account);
                                }
                                return Err(e);
                            }
                            emit_log(
                                &app_ref,
                                &format!(
                                    "[{}] Token 刷新失败，回退到缓存 access token",
                                    email
                                ),
                                "warning",
                                None,
                            );
                        }
                    }
                }

                match do_fetch_quota(&token, &email, &app_ref).await {
                    Ok(payload) => {
                        let QuotaFetchPayload { quota, error } = payload;
                        let mut account_to_persist = None;
                        {
                            let mut accounts = accounts_ref.lock().unwrap();
                            if idx < accounts.len() {
                                // A successful quota API round-trip means account is still usable.
                                clear_account_disabled(&mut accounts, idx);
                                if let Some(err) = error {
                                    set_account_quota_error(
                                        &mut accounts,
                                        idx,
                                        err.kind,
                                        err.code,
                                        err.message,
                                    );
                                } else {
                                    clear_account_quota_error(&mut accounts, idx);
                                }
                                account_to_persist = Some(accounts[idx].clone());
                            }
                        }
                        if let Some(account) = account_to_persist {
                            let _ = crate::account::persist_account(&account);
                        }
                        {
                            let mut cache = quota_cache_ref.lock().unwrap();
                            cache.insert(email.clone(), quota.clone());
                        }
                        Ok((email, quota))
                    }
                    Err(fetch_err) => {
                        let mut final_err = fetch_err;
                        if let Some(refresh_err) = refresh_error {
                            if is_auth_related_quota_error(&final_err) {
                                final_err = format!(
                                    "额度查询失败：刷新失败且缓存 token 未授权。refresh: {}; fetch: {}",
                                    refresh_err, final_err
                                );
                            }
                        }

                        let mut account_to_persist = None;
                        {
                            let mut accounts = accounts_ref.lock().unwrap();
                            if idx < accounts.len() {
                                let kind = classify_error_kind_from_message(&final_err);
                                if should_disable_account_for_error_kind(&kind) {
                                    mark_account_disabled(
                                        &mut accounts,
                                        idx,
                                        format!("invalid_grant: {}", final_err),
                                    );
                                }
                                if is_persistent_account_issue_kind(&kind) {
                                    set_account_quota_error(
                                        &mut accounts,
                                        idx,
                                        Some(kind),
                                        None,
                                        final_err.clone(),
                                    );
                                } else {
                                    // Batch refresh may fail transiently under concurrency; don't pin account as abnormal.
                                    clear_account_quota_error(&mut accounts, idx);
                                }
                                account_to_persist = Some(accounts[idx].clone());
                            }
                        }
                        if let Some(account) = account_to_persist {
                            let _ = crate::account::persist_account(&account);
                        }

                        emit_log(
                            &app_ref,
                            &format!("[{}] 额度查询失败: {}", email, final_err),
                            "warning",
                            None,
                        );
                        Err(final_err)
                    }
                }
            }
        })
        .collect();

    let all_results = join_all(tasks).await;
    let mut results = Vec::new();
    let mut failed = 0;
    for r in all_results {
        match r {
            Ok(pair) => results.push(pair),
            Err(_) => failed += 1,
        }
    }

    emit_log(
        &app,
        &format!(
            "额度查询完成：成功 {}，失败 {}",
            results.len(),
            failed
        ),
        "success",
        None,
    );

    Ok(results)
}
