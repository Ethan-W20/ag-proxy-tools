use std::fs;

use crate::constants::INJECT_CODE;
use crate::models::PatchStatus;
use crate::utils::get_antigravity_base_path;

fn get_patch_target_files() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "vs/workbench/api/node/extensionHostProcess.js",
            "extensionHostProcess",
        ),
        (
            "vs/workbench/api/worker/extensionHostWorkerMain.js",
            "extensionHostWorker",
        ),
        ("main.js", "main"),
        ("vs/code/node/cliProcessMain.js", "cliProcessMain"),
    ]
}

#[tauri::command]
pub fn apply_patch(target_url: String) -> Result<String, String> {
    let base_path = get_antigravity_base_path().ok_or(
        "找不到 Antigravity IDE 安装路径。请确认是否安装在默认路径：%LOCALAPPDATA%\\Programs\\Antigravity",
    )?;

    let target_url = target_url.trim().trim_end_matches('/').to_string();
    if target_url.is_empty() {
        return Err("目标 URL 不能为空".to_string());
    }

    let url_pattern = regex::Regex::new(r"https://([a-zA-Z0-9.\-]*cloudcode[a-zA-Z0-9.\-]*\.com|127\.0\.0\.1:\d+|[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}(?::\d+)?)")
        .map_err(|e| format!("正则表达式编译失败: {}", e))?;

    let mut patched_count = 0;
    let mut errors = Vec::new();

    for (relative_path, name) in &get_patch_target_files() {
        let file_path = base_path.join(relative_path);
        if !file_path.exists() {
            continue;
        }

        let content = match fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(e) => {
                errors.push(format!("读取 {} 失败: {}", name, e));
                continue;
            }
        };

        let backup_path = file_path.with_extension("js.bak");
        if !backup_path.exists() {
            if let Err(e) = fs::write(&backup_path, &content) {
                errors.push(format!("备份 {} 失败: {}", name, e));
                continue;
            }
        }

        let replaced_content = url_pattern
            .replace_all(&content, target_url.as_str())
            .to_string();

        let mut final_content = replaced_content;
        if !final_content.contains("NODE_TLS_REJECT_UNAUTHORIZED") {
            final_content = format!("{}{}", INJECT_CODE, final_content);
        }

        if final_content != content {
            if let Err(e) = fs::write(&file_path, &final_content) {
                errors.push(format!(
                    "写入 {} 失败: {}。请确认 IDE 已完全关闭，并尝试以管理员权限运行。",
                    name, e
                ));
                continue;
            }
            patched_count += 1;
        } else if content.contains(target_url.as_str()) {
            patched_count += 1;
        }
    }

    if patched_count == 0 {
        if !errors.is_empty() {
            Err(errors.join("\n"))
        } else {
            Err("没有找到任何 IDE 核心文件，或文件已经是最新状态。".to_string())
        }
    } else {
        let msg = if errors.is_empty() {
            format!(
                "成功处理 {} 个补丁文件（目标: {}）",
                patched_count, target_url
            )
        } else {
            format!(
                "部分成功（{} 个），存在以下错误：\n{}",
                patched_count,
                errors.join("\n")
            )
        };
        Ok(msg)
    }
}

#[tauri::command]
pub fn remove_patch() -> Result<String, String> {
    let base_path = get_antigravity_base_path().ok_or("找不到 Antigravity IDE 安装路径")?;

    let mut restored_count = 0;
    for (relative_path, name) in &get_patch_target_files() {
        let file_path = base_path.join(relative_path);
        let backup_path = file_path.with_extension("js.bak");

        if backup_path.exists() {
            fs::copy(&backup_path, &file_path).map_err(|e| format!("恢复 {} 失败: {}", name, e))?;
            fs::remove_file(&backup_path).ok();
            restored_count += 1;
        }
    }

    if restored_count == 0 {
        Err("没有找到可恢复的备份文件".to_string())
    } else {
        Ok(format!("成功恢复 {} 个文件", restored_count))
    }
}

#[tauri::command]
pub fn check_patch_status() -> PatchStatus {
    let base_path: std::path::PathBuf = match get_antigravity_base_path() {
        Some(p) => p,
        None => {
            return PatchStatus {
                applied: false,
                message: "未找到 Antigravity IDE".to_string(),
            }
        }
    };

    let main_js = base_path.join("main.js");
    if let Ok(content) = fs::read_to_string(&main_js) {
        if content.contains("NODE_TLS_REJECT_UNAUTHORIZED") {
            if let Ok(re) = regex::Regex::new(r"https?://127\.0\.0\.1:(\d+)") {
                if let Some(caps) = re.captures(&content) {
                    return PatchStatus {
                        applied: true,
                        message: format!("本地模式 (127.0.0.1:{})", &caps[1]),
                    };
                }
            }
            if let Ok(re) =
                regex::Regex::new(r"https?://([a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}(?::\d+)?)")
            {
                for caps in re.captures_iter(&content) {
                    let host = &caps[1];
                    if !host.contains("cloudcode")
                        && !host.contains("googleapis")
                        && !host.contains("google.com")
                    {
                        return PatchStatus {
                            applied: true,
                            message: format!("自定义 URL ({})", host),
                        };
                    }
                }
            }
            return PatchStatus {
                applied: true,
                message: "补丁已应用".to_string(),
            };
        }
    }

    PatchStatus {
        applied: false,
        message: "未应用补丁".to_string(),
    }
}
