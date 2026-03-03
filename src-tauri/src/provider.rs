use bytes::Bytes;
use http_body_util::Full;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tauri::State;

use crate::models::{AiProvider, AppState};
use crate::utils::{emit_log, get_app_data_dir};

fn get_providers_path() -> PathBuf {
    get_app_data_dir().join("providers.json")
}

#[tauri::command]
pub fn save_providers(state: State<'_, AppState>, providers: String) -> Result<String, String> {
    let parsed: Vec<AiProvider> = serde_json::from_str(&providers).map_err(|e| e.to_string())?;
    *state.providers.lock().unwrap() = parsed.clone();
    let path = get_providers_path();
    fs::write(&path, serde_json::to_string_pretty(&parsed).unwrap()).map_err(|e| e.to_string())?;
    Ok("供应商配置已保存".to_string())
}

#[tauri::command]
pub fn load_saved_providers(state: State<'_, AppState>) -> Result<String, String> {
    let path = get_providers_path();
    if path.exists() {
        let data = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let parsed: Vec<AiProvider> = serde_json::from_str(&data).unwrap_or_default();
        *state.providers.lock().unwrap() = parsed.clone();
        Ok(serde_json::to_string(&parsed).unwrap_or_default())
    } else {
        Ok("[]".to_string())
    }
}

fn clean_schema_for_openai(value: &mut serde_json::Value) {
    if let serde_json::Value::Object(map) = value {
        if let Some(serde_json::Value::String(s)) = map.get_mut("type") {
            *s = s.to_lowercase();
        }
        map.remove("format");

        if let Some(serde_json::Value::Object(props)) = map.get_mut("properties") {
            for v in props.values_mut() {
                clean_schema_for_openai(v);
            }
        }
        if let Some(items) = map.get_mut("items") {
            clean_schema_for_openai(items);
        }
        for key in ["anyOf", "oneOf", "allOf"] {
            if let Some(serde_json::Value::Array(items)) = map.get_mut(key) {
                for item in items.iter_mut() {
                    clean_schema_for_openai(item);
                }
            }
        }
    } else if let serde_json::Value::Array(arr) = value {
        for item in arr {
            clean_schema_for_openai(item);
        }
    }
}

pub fn extract_model_from_body(body: &[u8]) -> Option<String> {
    fn collect_model_fields(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(map) => {
                for (k, val) in map {
                    let key = k.to_ascii_lowercase();
                    if (key == "model"
                        || key == "modelname"
                        || key == "model_name"
                        || key == "modelid"
                        || key == "model_id")
                        && val.is_string()
                    {
                        if let Some(s) = val.as_str() {
                            let trimmed = s.trim();
                            if !trimmed.is_empty() {
                                out.push(trimmed.to_string());
                            }
                        }
                    }
                    collect_model_fields(val, out);
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    collect_model_fields(item, out);
                }
            }
            _ => {}
        }
    }

    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let mut candidates: Vec<String> = Vec::new();

    if let Some(s) = v.get("model").and_then(|m| m.as_str()) {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            candidates.push(trimmed.to_string());
        }
    }
    if let Some(s) = v
        .get("request")
        .and_then(|r| r.get("model"))
        .and_then(|m| m.as_str())
    {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            candidates.push(trimmed.to_string());
        }
    }

    collect_model_fields(&v, &mut candidates);
    candidates.into_iter().next()
}

pub fn extract_model_from_path(path_query: &str) -> Option<String> {
    let model_raw = path_query
        .split("/models/")
        .nth(1)?
        .split(':')
        .next()?
        .trim_matches('/')
        .trim();
    if model_raw.is_empty() {
        return None;
    }
    Some(model_raw.to_string())
}

fn normalize_model_name(raw: &str) -> String {
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

pub fn find_provider_for_model(
    providers: &[AiProvider],
    model: &str,
) -> Option<(AiProvider, String)> {
    let model_trimmed = model.trim();
    if model_trimmed.is_empty() {
        return None;
    }
    let model_norm = normalize_model_name(model_trimmed);

    for provider in providers {
        if !provider.enabled {
            continue;
        }
        for (from_model, target_model) in &provider.model_map {
            let from_trimmed = from_model.trim();
            if from_trimmed.is_empty() {
                continue;
            }
            let from_norm = normalize_model_name(from_trimmed);
            if from_norm.is_empty() {
                continue;
            }
            let exact_match =
                model_trimmed.eq_ignore_ascii_case(from_trimmed) || model_norm == from_norm;
            let prefix_match = model_norm.starts_with(&(from_norm.clone() + "-"))
                || model_norm.starts_with(&(from_norm.clone() + "@"));
            if exact_match || prefix_match {
                return Some((provider.clone(), target_model.clone()));
            }
        }
    }
    None
}

fn convert_antigravity_to_openai(body: &[u8], target_model: &str) -> Result<Vec<u8>, String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|e| e.to_string())?;

    let request = v.get("request").unwrap_or(&v);
    let mut openai_req = serde_json::json!({
        "model": target_model,
        "stream": false,
    });

    if let Some(gen_config) = request.get("generationConfig") {
        if let Some(t) = gen_config.get("temperature").and_then(|v| v.as_f64()) {
            openai_req["temperature"] = serde_json::json!(t);
        }
        if let Some(p) = gen_config.get("topP").and_then(|v| v.as_f64()) {
            openai_req["top_p"] = serde_json::json!(p);
        }
        if let Some(m) = gen_config.get("maxOutputTokens").and_then(|v| v.as_i64()) {
            openai_req["max_tokens"] = serde_json::json!(m);
        }
        if let Some(tc) = gen_config.get("thinkingConfig") {
            if let Some(level) = tc.get("thinkingLevel").and_then(|v| v.as_str()) {
                openai_req["reasoning_effort"] = serde_json::json!(level);
            }
        }
    }

    let mut messages: Vec<serde_json::Value> = Vec::new();
    if let Some(si) = request.get("systemInstruction") {
        if let Some(parts) = si.get("parts").and_then(|p| p.as_array()) {
            let text: Vec<String> = parts
                .iter()
                .filter_map(|p| {
                    p.get("text")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                })
                .collect();
            if !text.is_empty() {
                messages.push(serde_json::json!({
                    "role": "system",
                    "content": text.join("\n")
                }));
            }
        }
    }

    let mut fc_name_to_id: HashMap<String, Vec<String>> = HashMap::new();
    let mut fc_id_counter: u32 = 0;

    if let Some(contents) = request.get("contents").and_then(|c| c.as_array()) {
        for content in contents {
            if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                for part in parts {
                    if let Some(fc) = part.get("functionCall") {
                        let name = fc
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let id = fc
                            .get("id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| {
                                fc_id_counter += 1;
                                format!("call_{}", fc_id_counter)
                            });
                        fc_name_to_id.entry(name).or_default().push(id);
                    }
                }
            }
        }
    }

    let mut fc_name_consume_idx: HashMap<String, usize> = HashMap::new();
    let mut fc_id_counter2: u32 = 0;

    if let Some(contents) = request.get("contents").and_then(|c| c.as_array()) {
        for content in contents {
            let role = content
                .get("role")
                .and_then(|r| r.as_str())
                .unwrap_or("user");
            let openai_role = match role {
                "model" => "assistant",
                _ => role,
            };

            if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                let mut text_parts: Vec<String> = Vec::new();
                let mut tool_calls: Vec<serde_json::Value> = Vec::new();
                let mut tool_messages: Vec<serde_json::Value> = Vec::new();

                for part in parts {
                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                        let is_thought = part
                            .get("thought")
                            .and_then(|t| t.as_bool())
                            .unwrap_or(false);
                        if !is_thought {
                            text_parts.push(text.to_string());
                        }
                    }

                    if let Some(fc) = part.get("functionCall") {
                        let name = fc.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
                        let id = fc
                            .get("id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| {
                                fc_id_counter2 += 1;
                                format!("call_{}", fc_id_counter2)
                            });

                        let args = fc
                            .get("args")
                            .map(|a| a.to_string())
                            .unwrap_or_else(|| "{}".to_string());
                        tool_calls.push(serde_json::json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": args
                            }
                        }));
                    }

                    if let Some(fr) = part.get("functionResponse") {
                        let name = fr
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();

                        let id = fr
                            .get("id")
                            .and_then(|i| i.as_str())
                            .map(|s| s.to_string())
                            .or_else(|| {
                                if !name.is_empty() {
                                    if let Some(ids) = fc_name_to_id.get(&name) {
                                        let idx =
                                            fc_name_consume_idx.entry(name.clone()).or_insert(0);
                                        if *idx < ids.len() {
                                            let matched_id = ids[*idx].clone();
                                            *idx += 1;
                                            return Some(matched_id);
                                        }
                                    }
                                }
                                None
                            })
                            .unwrap_or_else(|| format!("call_unknown_{}", tool_messages.len()));

                        let content_str = if let Some(response) = fr.get("response") {
                            if let Some(result) = response.get("result") {
                                match result {
                                    serde_json::Value::String(s) => s.clone(),
                                    v => v.to_string(),
                                }
                            } else {
                                match response {
                                    serde_json::Value::String(s) => s.clone(),
                                    v => v.to_string(),
                                }
                            }
                        } else {
                            "{}".to_string()
                        };

                        tool_messages.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": id,
                            "content": content_str
                        }));
                    }
                }

                if openai_role == "assistant" {
                    let mut msg = serde_json::json!({ "role": "assistant" });
                    if !text_parts.is_empty() {
                        msg["content"] = serde_json::json!(text_parts.join(""));
                    }
                    if !tool_calls.is_empty() {
                        msg["tool_calls"] = serde_json::json!(tool_calls);
                    }
                    if msg.get("content").is_some() || msg.get("tool_calls").is_some() {
                        messages.push(msg);
                    }
                } else {
                    for tm in tool_messages {
                        messages.push(tm);
                    }
                    if !text_parts.is_empty() {
                        messages.push(serde_json::json!({
                            "role": "user",
                            "content": text_parts.join("")
                        }));
                    }
                }
            }
        }
    }

    let mut validated_messages: Vec<serde_json::Value> = Vec::new();
    let mut i = 0;
    while i < messages.len() {
        let msg = &messages[i];
        validated_messages.push(msg.clone());

        if msg.get("role").and_then(|r| r.as_str()) == Some("assistant") {
            if let Some(tool_calls) = msg.get("tool_calls").and_then(|tc| tc.as_array()) {
                let required_ids: Vec<String> = tool_calls
                    .iter()
                    .filter_map(|tc| {
                        tc.get("id")
                            .and_then(|id| id.as_str())
                            .map(|s| s.to_string())
                    })
                    .collect();

                let mut found_ids: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                let mut j = i + 1;
                while j < messages.len() {
                    let next_msg = &messages[j];
                    let next_role = next_msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
                    if next_role == "tool" {
                        if let Some(tid) = next_msg.get("tool_call_id").and_then(|id| id.as_str()) {
                            found_ids.insert(tid.to_string());
                        }
                    } else if next_role != "tool" {
                        break; // tool 消息块结束，遇到下一个非 tool 角色则停止收集
                    }
                    j += 1;
                }

                for req_id in &required_ids {
                    if !found_ids.contains(req_id) {}
                }

                i += 1;
                while i < messages.len() {
                    let next_role = messages[i]
                        .get("role")
                        .and_then(|r| r.as_str())
                        .unwrap_or("");
                    if next_role == "tool" {
                        validated_messages.push(messages[i].clone());
                        if let Some(tid) =
                            messages[i].get("tool_call_id").and_then(|id| id.as_str())
                        {
                            found_ids.insert(tid.to_string());
                        }
                        i += 1;
                    } else {
                        break;
                    }
                }

                for req_id in &required_ids {
                    if !found_ids.contains(req_id) {
                        validated_messages.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": req_id,
                            "content": "{}"
                        }));
                    }
                }
                continue; // 已处理并推进 i，避免后续再执行一次 i += 1
            }
        }
        i += 1;
    }

    openai_req["messages"] = serde_json::json!(validated_messages);

    if let Some(tools) = request.get("tools").and_then(|t| t.as_array()) {
        let mut openai_tools: Vec<serde_json::Value> = Vec::new();
        for tool in tools {
            if let Some(fds) = tool.get("functionDeclarations").and_then(|f| f.as_array()) {
                for fd in fds {
                    let name = fd.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
                    let desc = fd.get("description").and_then(|d| d.as_str()).unwrap_or("");
                    let mut params = fd
                        .get("parametersJsonSchema")
                        .or_else(|| fd.get("parameters"))
                        .cloned()
                        .unwrap_or(serde_json::json!({"type": "object", "properties": {}}));
                    clean_schema_for_openai(&mut params);
                    openai_tools.push(serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": name,
                            "description": desc,
                            "parameters": params
                        }
                    }));
                }
            }
        }
        if !openai_tools.is_empty() {
            openai_req["tools"] = serde_json::json!(openai_tools);
        }
    }

    serde_json::to_vec(&openai_req).map_err(|e| e.to_string())
}

pub async fn forward_to_provider(
    app: &tauri::AppHandle,
    provider: &AiProvider,
    target_model: &str,
    body_bytes: &[u8],
) -> Result<http::Response<Full<Bytes>>, String> {
    let openai_body = convert_antigravity_to_openai(body_bytes, target_model)?;

    let base = provider.base_url.trim_end_matches('/');
    let target_url = match provider.protocol.as_str() {
        "openai" => format!("{}/chat/completions", base),
        "gemini" => format!(
            "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
            base, target_model
        ),
        "claude" => format!("{}/v1/messages", base),
        _ => format!("{}/chat/completions", base),
    };

    emit_log(
        app,
        &format!(
            "杞彂鍒颁緵搴斿晢 [{}]: {} -> {}",
            provider.name, target_model, target_url
        ),
        "info",
        None,
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;

    let mut req_builder = client
        .post(&target_url)
        .header("Content-Type", "application/json");

    match provider.protocol.as_str() {
        "claude" => {
            req_builder = req_builder
                .header("x-api-key", &provider.api_key)
                .header("anthropic-version", "2023-06-01");
        }
        _ => {
            req_builder =
                req_builder.header("Authorization", format!("Bearer {}", provider.api_key));
        }
    }

    let resp = req_builder
        .body(openai_body)
        .send()
        .await
        .map_err(|e| format!("供应商请求失败: {}", e))?;

    let status_code = resp.status().as_u16();

    if status_code >= 400 {
        let err_body = resp.text().await.unwrap_or_default();
        emit_log(
            app,
            &format!(
                "供应商返回错误({}): {}",
                status_code,
                &err_body[..err_body.len().min(500)]
            ),
            "error",
            Some(&err_body),
        );
        return Err(format!("供应商错误 {}: {}", status_code, err_body));
    }

    {
        let resp_body = resp.bytes().await.map_err(|e| e.to_string())?;
        let resp_str = String::from_utf8_lossy(&resp_body);
        emit_log(
            app,
            &format!(
                "供应商响应完成[{}], {} bytes",
                provider.name,
                resp_body.len()
            ),
            "success",
            None,
        );

        if let Ok(openai_resp) = serde_json::from_str::<serde_json::Value>(&resp_str) {
            let mut all_parts: Vec<serde_json::Value> = Vec::new();

            if let Some(choices) = openai_resp.get("choices").and_then(|c| c.as_array()) {
                for choice in choices {
                    if let Some(msg) = choice.get("message") {
                        if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                            all_parts.push(serde_json::json!({"text": content}));
                        }
                        if let Some(tcs) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                            for tc in tcs {
                                if let Some(func) = tc.get("function") {
                                    let mut fc = serde_json::json!({});
                                    if let Some(name) = func.get("name") {
                                        fc["name"] = name.clone();
                                    }
                                    if let Some(args) =
                                        func.get("arguments").and_then(|a| a.as_str())
                                    {
                                        if let Ok(parsed) =
                                            serde_json::from_str::<serde_json::Value>(args)
                                        {
                                            fc["args"] = parsed;
                                        }
                                    }
                                    if let Some(id) = tc.get("id") {
                                        fc["id"] = id.clone();
                                    }
                                    fc["thoughtSignature"] =
                                        serde_json::json!("skip_thought_signature_validator");
                                    all_parts.push(serde_json::json!({"functionCall": fc}));
                                }
                            }
                        }
                    }
                }
            }

            let finish_reason = openai_resp
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("finish_reason"))
                .and_then(|f| f.as_str())
                .unwrap_or("stop");

            let gemini_fr = match finish_reason {
                "stop" | "tool_calls" => "STOP",
                "length" | "max_tokens" => "MAX_TOKENS",
                _ => "STOP",
            };

            let model_version = openai_resp
                .get("model")
                .and_then(|m| m.as_str())
                .unwrap_or(target_model);

            let mut converted = serde_json::json!({
                "response": {
                    "candidates": [{
                        "content": {
                            "role": "model",
                            "parts": all_parts
                        },
                        "finishReason": gemini_fr
                    }],
                    "modelVersion": model_version,
                    "responseId": openai_resp.get("id").and_then(|i| i.as_str()).unwrap_or("")
                }
            });

            if let Some(usage) = openai_resp.get("usage") {
                let prompt = usage
                    .get("prompt_tokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let completion = usage
                    .get("completion_tokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let total = usage
                    .get("total_tokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                converted["response"]["usageMetadata"] = serde_json::json!({
                    "promptTokenCount": prompt,
                    "candidatesTokenCount": completion,
                    "totalTokenCount": total
                });
            }

            let converted_json = serde_json::to_string(&converted).unwrap_or_default();
            let sse_body = format!("data: {}\n\n", converted_json);

            emit_log(
                app,
                &format!("返回 SSE 响应给 IDE, {} chars", sse_body.len()),
                "info",
                None,
            );

            return Ok(http::Response::builder()
                .status(200)
                .header("Content-Type", "text/event-stream")
                .header("Cache-Control", "no-cache")
                .body(Full::new(Bytes::from(sse_body)))
                .unwrap());
        }

        let sse_fallback = format!("data: {}\n\n", resp_str);
        Ok(http::Response::builder()
            .status(200)
            .header("Content-Type", "text/event-stream")
            .body(Full::new(Bytes::from(sse_fallback)))
            .unwrap())
    }
}

