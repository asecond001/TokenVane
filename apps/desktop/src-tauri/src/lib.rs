#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod commands;
pub mod state;
pub mod tray;

use std::fs::OpenOptions;
use std::io::Write;

use chrono::Utc;
use tauri::{Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

use crate::state::{default_db_path, AppState};
use crate::tray::setup_tray;

fn log_startup_error(context: &str, err: &dyn std::fmt::Display) {
    eprintln!("{context}: {err}");
    if let Some(dir) = default_db_path().parent() {
        let _ = std::fs::create_dir_all(dir);
        let log_path = dir.join("startup-error.log");
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) {
            let _ = writeln!(file, "[{}] {context}: {err}", Utc::now().to_rfc3339());
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let app_state = match AppState::initialize() {
        Ok(s) => s,
        Err(err) => {
            log_startup_error("TokenVane initialization failed", &err);
            return;
        }
    };

    let builder = tauri::Builder::default()
        // Must be registered first: a second launch (autostart + manual start, double-clicking
        // the shortcut) would otherwise run another collector against the same SQLite database,
        // add a duplicate tray icon and fail to bind the OTLP port. Focus the running instance.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tray::show_main_window(app);
        }))
        .manage(app_state)
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::get_bootstrap_state,
            commands::get_overview,
            commands::get_trends,
            commands::get_monthly,
            commands::get_models_pricing,
            commands::upsert_model_pricing_override,
            commands::delete_model_pricing_override,
            commands::recompute_all_costs,
            commands::sync_pricing_and_rate,
            commands::get_subscriptions,
            commands::upsert_subscription_plan,
            commands::set_subscription_plan_enabled,
            commands::delete_subscription_plan,
            commands::get_subscription_templates,
            commands::get_observed_rate_limits,
            commands::get_speed_latency,
            commands::get_tray_summary,
            commands::get_provider_configs,
            commands::update_provider_config,
            commands::get_ingestion_cursors,
            commands::trigger_scan_now,
            commands::set_provider_api_key,
            commands::delete_provider_api_key,
            commands::reconcile_provider_official_api,
            commands::get_preferences,
            commands::update_preferences,
            commands::import_background_image,
            commands::clear_background_image,
            commands::get_background_image_data_url,
            commands::export_monthly_report,
            commands::preview_sync_envelope_v1,
            commands::list_custom_provider_rules,
            commands::upsert_custom_provider_rule,
            commands::delete_custom_provider_rule,
            commands::test_custom_provider_sample_line,
            commands::get_provider_balances,
            commands::refresh_provider_balance,
            commands::launch_provider_cli_login,
            commands::get_payg_accounts,
            commands::upsert_payg_account,
            commands::delete_payg_account,
            commands::list_devices,
            commands::get_sync_status,
            commands::update_sync_config,
            commands::save_cloud_sync_setup,
            commands::provision_cloud_sync_worker,
            commands::save_share_image,
            commands::get_sync_pairing_bundle,
            commands::set_sync_credentials,
            commands::clear_sync_credentials,
            commands::trigger_cloud_sync_now,
            commands::get_remote_subscription_snapshots,
            commands::open_main_workspace,
            commands::quit_application,
            commands::toggle_floating_window,
            commands::get_floating_window_visible,
            commands::set_floating_window_compact,
        ])
        .setup(|app| {
            if let Err(err) = setup_tray(app.handle()) {
                log_startup_error("TokenVane tray setup failed", &err);
                return Err(err.into());
            }
            let state = app.state::<AppState>();
            if let Ok(prefs) = state.storage.get_preferences() {
                commands::apply_theme_to_app(app.handle(), &prefs.theme);
                if prefs.floating_window_enabled {
                    let handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = tray::set_floating_window_visible(&handle, true);
                    });
                }
                if prefs.launch_at_startup {
                    if let Err(err) = app.autolaunch().enable() {
                        tracing::warn!("Failed to re-sync autostart on startup: {err}");
                    }
                }
            }
            state.start_background_services(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    let state = window.state::<AppState>();
                    let minimize = state
                        .storage
                        .get_preferences()
                        .map(|p| p.minimize_to_tray)
                        .unwrap_or(true);
                    if minimize {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
            } else if window.label() == tray::FLOATING_MONITOR_LABEL {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = tray::set_floating_window_and_persist(window.app_handle(), false);
                }
            }
        });

    let app = match builder.build(tauri::generate_context!()) {
        Ok(a) => a,
        Err(err) => {
            log_startup_error("Failed to build TokenVane Tauri runtime", &err);
            return;
        }
    };

    app.run(|app_handle, event| {
        if let RunEvent::Exit = event {
            let state = app_handle.state::<AppState>();
            state.shutdown_background_services();
        }
    });
}
