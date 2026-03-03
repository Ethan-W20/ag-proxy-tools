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

pub fn get_antigravity_base_path() -> Option<PathBuf> {
    let mut search_paths = Vec::new();

    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        search_paths.push(
            PathBuf::from(local_app_data)
                .join("Programs")
                .join("Antigravity"),
        );
    }

    if let Ok(prog_files) = std::env::var("ProgramFiles") {
        search_paths.push(PathBuf::from(prog_files).join("Antigravity"));
    }

    for path in search_paths {
        let base = path.join("resources").join("app").join("out");
        if base.exists() {
            return Some(base);
        }

        let alt_base = path.join("resources").join("app");
        if alt_base.join("main.js").exists() {
            return Some(alt_base);
        }
    }

    None
}

pub type BoxBody =
    http_body_util::combinators::BoxBody<Bytes, Box<dyn std::error::Error + Send + Sync>>;

pub fn full_body(bytes: Bytes) -> BoxBody {
    Full::new(bytes)
        .map_err(|never| -> Box<dyn std::error::Error + Send + Sync> { match never {} })
        .boxed()
}
