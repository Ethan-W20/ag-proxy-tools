pub mod account;
pub mod cert;
pub mod constants;
pub mod ls_bridge;
pub mod models;
pub mod patch;
pub mod protobuf;
pub mod provider;
pub mod proxy;
pub mod proxy_error;
pub mod quota;
pub mod token_stats;
pub mod utils;

use models::AppState;
use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Mutex};
use utils::get_app_data_dir;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage({
            let raw_strategy = fs::read_to_string(get_app_data_dir().join("routing_strategy.txt"))
                .unwrap_or_else(|_| "fill".to_string());
            let saved_strategy = match raw_strategy.trim().to_lowercase().as_str() {
                "round-robin" | "roundrobin" | "rr" | "performance-first" => {
                    "round-robin".to_string()
                }
                _ => "fill".to_string(),
            };
            let saved_threshold =
                fs::read_to_string(get_app_data_dir().join("quota_threshold.txt"))
                    .unwrap_or_else(|_| "0".to_string())
                    .trim()
                    .parse::<i32>()
                    .unwrap_or(0)
                    .clamp(0, 80);
            let saved_header_passthrough =
                !matches!(
                    fs::read_to_string(get_app_data_dir().join("header_passthrough.txt"))
                    .unwrap_or_else(|_| "1".to_string())
                    .trim()
                    .to_lowercase()
                    .as_str(),
                    "0" | "off" | "false"
                );
            let saved_official_ls_enabled = ls_bridge::is_official_ls_enabled();
            let saved_upstream_server =
                match fs::read_to_string(get_app_data_dir().join("upstream_server.txt"))
                    .unwrap_or_else(|_| "sandbox".to_string())
                    .trim()
                    .to_lowercase()
                    .as_str()
                {
                    "custom" => "custom".to_string(),
                    _ => "sandbox".to_string(),
                };
            let saved_upstream_custom_url =
                fs::read_to_string(get_app_data_dir().join("upstream_custom_url.txt"))
                    .unwrap_or_default()
                    .trim()
                    .trim_end_matches('/')
                    .to_string();
            let saved_http_protocol_mode =
                match fs::read_to_string(get_app_data_dir().join("http_protocol_mode.txt"))
                    .unwrap_or_else(|_| "auto".to_string())
                    .trim()
                    .to_lowercase()
                    .as_str()
                {
                    "http10" | "h10" | "http1.0" | "1.0" => "http10".to_string(),
                    "http1" | "h1" | "http1.1" => "http1".to_string(),
                    "http2" | "h2" => "http2".to_string(),
                    _ => "auto".to_string(),
                };
            let saved_capacity_failover_enabled =
                !matches!(
                    fs::read_to_string(get_app_data_dir().join("capacity_failover_enabled.txt"))
                    .unwrap_or_else(|_| "1".to_string())
                    .trim()
                    .to_lowercase()
                    .as_str(),
                    "0" | "off" | "false"
                );
            AppState {
                accounts: Arc::new(Mutex::new(Vec::new())),
                current_idx: Arc::new(Mutex::new(-1)),
                proxy_running: Arc::new(Mutex::new(false)),
                proxy_shutdown_tx: Mutex::new(None),
                providers: Arc::new(Mutex::new(Vec::new())),
                routing_strategy: Arc::new(Mutex::new(saved_strategy)),
                header_passthrough: Arc::new(Mutex::new(saved_header_passthrough)),
                official_ls_enabled: Arc::new(Mutex::new(saved_official_ls_enabled)),
                upstream_server: Arc::new(Mutex::new(saved_upstream_server)),
                upstream_custom_url: Arc::new(Mutex::new(saved_upstream_custom_url)),
                http_protocol_mode: Arc::new(Mutex::new(saved_http_protocol_mode)),
                capacity_failover_enabled: Arc::new(Mutex::new(saved_capacity_failover_enabled)),
                token_stats: token_stats::TokenStatsManager::new(),
                quota_threshold: Arc::new(Mutex::new(saved_threshold)),
                quota_cache: Arc::new(Mutex::new(HashMap::new())),
            }
        })
        .invoke_handler(tauri::generate_handler![
            patch::apply_patch,
            patch::remove_patch,
            patch::check_patch_status,
            cert::import_cert,
            cert::remove_cert,
            cert::check_cert_status,
            account::load_credentials,
            account::load_credentials_stream,
            account::switch_account,
            account::delete_account,
            account::import_credential_files,
            account::import_refresh_token,
            account::start_oauth_login,
            account::toggle_account_disabled,
            proxy::start_proxy,
            proxy::stop_proxy,
            proxy::save_port_config,
            provider::save_providers,
            provider::load_saved_providers,
            quota::fetch_quota,
            quota::fetch_all_quotas,
            proxy::get_routing_strategy,
            proxy::set_routing_strategy,
            proxy::set_header_passthrough,
            proxy::get_header_passthrough,
            proxy::set_upstream_server,
            proxy::get_upstream_server,
            proxy::set_upstream_custom_url,
            proxy::get_upstream_custom_url,
            proxy::set_http_protocol_mode,
            proxy::get_http_protocol_mode,
            proxy::set_capacity_failover_enabled,
            proxy::get_capacity_failover_enabled,
            ls_bridge::set_official_ls_enabled,
            ls_bridge::get_official_ls_enabled,
            ls_bridge::check_official_ls_binary,
            ls_bridge::start_official_ls,
            ls_bridge::stop_official_ls,
            ls_bridge::get_official_ls_status,
            proxy::get_token_stats,
            proxy::reset_token_stats,
            proxy::set_quota_threshold,
            proxy::get_quota_threshold,
            proxy::flush_token_stats,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
