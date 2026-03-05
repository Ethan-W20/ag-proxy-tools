pub mod account;
pub mod cert;
pub mod constants;

pub mod models;
pub mod patch;
pub mod protobuf;
pub mod provider;
pub mod proxy;
pub mod proxy_error;
pub mod proxy_log;
pub mod proxy_usage;
pub mod quota;
pub mod token_stats;
pub mod utils;

use models::AppState;
use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Mutex};
use tauri::Manager;
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

                upstream_server: Arc::new(Mutex::new(saved_upstream_server)),
                upstream_custom_url: Arc::new(Mutex::new(saved_upstream_custom_url)),
                http_protocol_mode: Arc::new(Mutex::new(saved_http_protocol_mode)),
                capacity_failover_enabled: Arc::new(Mutex::new(saved_capacity_failover_enabled)),
                token_stats: token_stats::TokenStatsManager::new(),
                quota_threshold: Arc::new(Mutex::new(saved_threshold)),
                quota_cache: Arc::new(Mutex::new(HashMap::new())),
                last_context_usage: Arc::new(Mutex::new(Vec::new())),
                context_usage_latest: Arc::new(Mutex::new((0, String::new()))),
                context_ring_window_secs: Arc::new(Mutex::new(15)),
                auto_accept_config: Arc::new(Mutex::new(r#"{"enabled":true,"patterns":{"retry":false,"run":true,"apply":true,"execute":true,"confirm":false,"allow":true,"accept":true},"bannedCommands":["rm -rf /","rm -rf ~","rm -rf *","format c:","del /f /s /q","rmdir /s /q",":(){:|:&};:","dd if=","mkfs.","> /dev/sda","chmod -R 777 /"]}"#.to_string())),
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
            account::sync_accounts_from_legacy_projects,
            account::import_refresh_token,
            account::start_oauth_login,
            account::toggle_account_disabled,
            proxy::start_proxy,
            proxy::stop_proxy,
            proxy::save_port_config,
            proxy::load_port_config,
            proxy::kill_port_process,
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

            proxy::get_token_stats,
            proxy::reset_token_stats,
            proxy::set_quota_threshold,
            proxy::get_quota_threshold,
            proxy::flush_token_stats,
            patch::check_auto_accept_status,
            patch::apply_auto_accept,
            patch::remove_auto_accept,
            patch::check_context_ring_status,
            patch::apply_context_ring,
            patch::remove_context_ring,
            patch::toggle_context_ring,
            patch::get_context_ring_window,
            patch::set_context_ring_window,
            patch::update_auto_accept_config,
        ])
        .setup(|app| {
            // Stop proxy when the main window is closed
            let app_handle = app.handle().clone();
            if let Some(window) = app.get_webview_window("main") {
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::Destroyed = event {
                        // Send shutdown signal to proxy
                        let state = app_handle.state::<AppState>();
                        if let Some(tx) = state.proxy_shutdown_tx.lock().unwrap().take() {
                            let _ = tx.send(());
                        }
                        *state.proxy_running.lock().unwrap() = false;
                    }
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                // Final cleanup: stop proxy on app exit
                let state = app_handle.state::<AppState>();
                let tx = state.proxy_shutdown_tx.lock().unwrap().take();
                if let Some(tx) = tx {
                    let _ = tx.send(());
                }
            }
        });
}
