use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use tauri::{Emitter, Manager};

use crate::models::LogPayload;

const APP_DATA_DIR_NAME: &str = ".antigravity_proxy_manager";

pub fn emit_log(handle: &tauri::AppHandle, message: &str, log_type: &str, details: Option<&str>) {
    if log_type == "error" {
        if let Some(app_state) = handle.try_state::<crate::models::AppState>() {
            app_state.token_stats.record_error();
        }
    }

    let _ = handle.emit(
        "log-event",
        LogPayload {
            message: message.to_string(),
            log_type: log_type.to_string(),
            details: details.map(|s| s.to_string()),
        },
    );
}

pub fn emit_request_flow(handle: &tauri::AppHandle, payload: &crate::models::RequestFlowPayload) {
    let _ = handle.emit("request-flow", payload);
}

pub fn get_app_data_dir() -> PathBuf {
    static APP_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
    APP_DATA_DIR
        .get_or_init(|| {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            let dir = home.join(APP_DATA_DIR_NAME);
            let _ = fs::create_dir_all(&dir);
            dir
        })
        .clone()
}

pub fn get_cert_path() -> PathBuf {
    get_app_data_dir().join("ag-proxy-ca.crt")
}

pub fn get_key_path() -> PathBuf {
    get_app_data_dir().join("ag-proxy-ca.key")
}

/// Check whether `candidate` is a valid IDE root and return the base path for
/// patching (the directory that contains main.js and the vs/ tree).
fn try_resolve_ide_base(candidate: &std::path::Path) -> Option<PathBuf> {
    // Layout 1: <root>/resources/app/out/main.js  (standard packaged)
    let out = candidate.join("resources").join("app").join("out");
    if out.join("main.js").exists() {
        return Some(out);
    }
    // Layout 2: <root>/resources/app/main.js  (alternative)
    let app = candidate.join("resources").join("app");
    if app.join("main.js").exists() {
        return Some(app);
    }
    // Layout 3: candidate itself already IS the base (out/ or app/)
    if candidate.join("main.js").exists()
        && candidate.join("vs").is_dir()
    {
        return Some(candidate.to_path_buf());
    }
    None
}

/// Discover the Antigravity IDE installation base path.
///
/// Search order:
///   1. Windows registry (Uninstall entries written by NSIS installer)
///   2. Known default install locations per OS
///   3. Desktop / Start-menu shortcut targets (Windows .lnk)
///   4. Common drive roots (D:\ .. Z:\) with "Antigravity" directory
///   5. Walk up from current executable path
pub fn get_antigravity_base_path() -> Option<PathBuf> {
    let mut ide_roots: Vec<PathBuf> = Vec::new();

    // ---------- 1. Windows registry ----------
    #[cfg(target_os = "windows")]
    {
        collect_ide_roots_from_registry(&mut ide_roots);
    }

    // ---------- 2. Known default install locations ----------
    #[cfg(target_os = "windows")]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            ide_roots.push(
                PathBuf::from(&local_app_data)
                    .join("Programs")
                    .join("Antigravity"),
            );
        }
        if let Ok(prog_files) = std::env::var("ProgramFiles") {
            ide_roots.push(PathBuf::from(prog_files).join("Antigravity"));
        }
        if let Ok(prog_files_x86) = std::env::var("ProgramFiles(x86)") {
            ide_roots.push(PathBuf::from(prog_files_x86).join("Antigravity"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        ide_roots.push(PathBuf::from("/Applications/Antigravity.app"));
        if let Ok(home) = std::env::var("HOME") {
            ide_roots.push(
                PathBuf::from(&home)
                    .join("Applications")
                    .join("Antigravity.app"),
            );
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(home) = std::env::var("HOME") {
            ide_roots.push(
                PathBuf::from(&home)
                    .join(".local")
                    .join("share")
                    .join("Antigravity"),
            );
        }
        ide_roots.push(PathBuf::from("/usr/share/antigravity"));
        ide_roots.push(PathBuf::from("/opt/antigravity"));
        ide_roots.push(PathBuf::from("/opt/Antigravity"));
    }

    // ---------- 3. Desktop / Start-menu shortcuts (Windows) ----------
    #[cfg(target_os = "windows")]
    {
        collect_ide_roots_from_shortcuts(&mut ide_roots);
    }

    // ---------- 4. Common drive roots (Windows) ----------
    #[cfg(target_os = "windows")]
    {
        for letter in b'C'..=b'Z' {
            let drive = format!("{}:\\", letter as char);
            let candidate = PathBuf::from(&drive).join("Antigravity");
            if candidate.is_dir() {
                ide_roots.push(candidate);
            }
            // Also check <drive>:\Program Files\Antigravity
            let pf = PathBuf::from(&drive)
                .join("Program Files")
                .join("Antigravity");
            if pf.is_dir() {
                ide_roots.push(pf);
            }
        }
    }

    // ---------- 5. Walk up from current executable ----------
    if let Ok(exe_path) = std::env::current_exe() {
        let mut dir = exe_path.as_path();
        for _ in 0..8 {
            if let Some(parent) = dir.parent() {
                dir = parent;
                ide_roots.push(dir.to_path_buf());
            } else {
                break;
            }
        }
    }

    // ---------- Deduplicate and resolve ----------
    // Canonicalize where possible to avoid duplicates.
    let mut seen = std::collections::HashSet::new();
    for root in &ide_roots {
        let key = root
            .canonicalize()
            .unwrap_or_else(|_| root.clone())
            .to_string_lossy()
            .to_lowercase();
        if !seen.insert(key) {
            continue;
        }
        if let Some(base) = try_resolve_ide_base(root) {
            return Some(base);
        }
    }

    None
}

// ---- Windows-specific helpers ----

#[cfg(target_os = "windows")]
fn collect_ide_roots_from_registry(ide_roots: &mut Vec<PathBuf>) {
    use std::process::Command;

    // NSIS installers typically write to:
    //   HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\<AppName>
    //   HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\<AppName>
    // We query both hives for "InstallLocation" or "DisplayIcon" containing "Antigravity".
    let reg_paths = [
        r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    ];

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    for reg_path in &reg_paths {
        // Enumerate sub-keys
        let output = {
            use std::os::windows::process::CommandExt;
            Command::new("reg")
                .creation_flags(CREATE_NO_WINDOW)
                .args(["query", reg_path, "/s", "/f", "Antigravity", "/d"])
                .output()
        };

        if let Ok(output) = output {
            let text = String::from_utf8_lossy(&output.stdout);
            // Look for InstallLocation or DisplayIcon lines
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.contains("InstallLocation") || trimmed.contains("DisplayIcon") {
                    // Format: "    InstallLocation    REG_SZ    C:\path\to\Antigravity"
                    if let Some((_key, value)) = trimmed.split_once("REG_SZ") {
                        let path_str = value.trim().trim_matches('"');
                        if !path_str.is_empty() {
                            let p = PathBuf::from(path_str);
                            // DisplayIcon often points to the .exe; get its parent
                            if p.is_file() {
                                if let Some(parent) = p.parent() {
                                    ide_roots.push(parent.to_path_buf());
                                }
                            } else if p.is_dir() {
                                ide_roots.push(p);
                            }
                        }
                    }
                }
                // Also handle UninstallString which may reveal install path
                if trimmed.contains("UninstallString") {
                    if let Some((_key, value)) = trimmed.split_once("REG_SZ") {
                        let raw = value.trim().trim_matches('"');
                        // Strip exe name from uninstall path
                        let p = PathBuf::from(raw);
                        if let Some(parent) = p.parent() {
                            if parent
                                .to_string_lossy()
                                .to_lowercase()
                                .contains("antigravity")
                            {
                                ide_roots.push(parent.to_path_buf());
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn collect_ide_roots_from_shortcuts(ide_roots: &mut Vec<PathBuf>) {
    // Look for .lnk files on Desktop and Start Menu that point to Antigravity
    let mut shortcut_dirs: Vec<PathBuf> = Vec::new();

    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        shortcut_dirs.push(PathBuf::from(&userprofile).join("Desktop"));
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        shortcut_dirs.push(
            PathBuf::from(&appdata)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs"),
        );
    }
    if let Ok(public) = std::env::var("PUBLIC") {
        shortcut_dirs.push(PathBuf::from(&public).join("Desktop"));
    }
    // ProgramData Start Menu
    shortcut_dirs.push(
        PathBuf::from(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs"),
    );

    for dir in &shortcut_dirs {
        if !dir.is_dir() {
            continue;
        }
        // Scan .lnk files (non-recursive for Desktop, recursive for Start Menu)
        let entries: Vec<PathBuf> = match fs::read_dir(dir) {
            Ok(rd) => rd
                .flatten()
                .map(|e| e.path())
                .collect(),
            Err(_) => continue,
        };
        for entry in entries {
            let name_lower = entry
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();

            if name_lower.contains("antigravity") && name_lower.ends_with(".lnk") {
                // Use PowerShell to resolve shortcut target
                if let Some(target) = resolve_lnk_target(&entry) {
                    let p = PathBuf::from(&target);
                    if p.is_file() {
                        if let Some(parent) = p.parent() {
                            ide_roots.push(parent.to_path_buf());
                        }
                    } else if p.is_dir() {
                        ide_roots.push(p);
                    }
                }
            }

            // Also recurse one level for Start Menu sub-folders
            if entry.is_dir() {
                if let Ok(sub_rd) = fs::read_dir(&entry) {
                    for sub_entry in sub_rd.flatten() {
                        let sub_name = sub_entry
                            .file_name()
                            .to_string_lossy()
                            .to_lowercase();
                        if sub_name.contains("antigravity") && sub_name.ends_with(".lnk") {
                            if let Some(target) = resolve_lnk_target(&sub_entry.path()) {
                                let p = PathBuf::from(&target);
                                if p.is_file() {
                                    if let Some(parent) = p.parent() {
                                        ide_roots.push(parent.to_path_buf());
                                    }
                                } else if p.is_dir() {
                                    ide_roots.push(p);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn resolve_lnk_target(lnk_path: &std::path::Path) -> Option<String> {
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$s.TargetPath",
        lnk_path.to_string_lossy().replace('\'', "''")
    );

    let output = {
        use std::os::windows::process::CommandExt;
        Command::new("powershell")
            .creation_flags(CREATE_NO_WINDOW)
            .args(["-NoProfile", "-Command", &script])
            .output()
            .ok()?
    };

    let target = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string();
    if target.is_empty() {
        None
    } else {
        Some(target)
    }
}

pub type BoxBody =
    http_body_util::combinators::BoxBody<Bytes, Box<dyn std::error::Error + Send + Sync>>;

pub fn full_body(bytes: Bytes) -> BoxBody {
    Full::new(bytes)
        .map_err(|never| -> Box<dyn std::error::Error + Send + Sync> { match never {} })
        .boxed()
}
