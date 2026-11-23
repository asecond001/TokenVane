use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, watch};
use tokvane_collectors::quota_refresh::{
    activity_refresh_kind, ActivityRefreshThrottle, QuotaRefreshKind,
};
use tokvane_collectors::{
    start_loopback_otlp_server, sync_catalog_and_rate, CollectorCoordinator, CredentialManager,
    OsKeyringStore, OtlpServerHandle, SyncClientHandle,
};
use tokvane_core::domain::{provider_ids, CatalogSyncResult, ProviderConfig};
use tokvane_core::{CoreResult, Storage};

/// Resolve `%LOCALAPPDATA%\TokenVane\tokenvane.db`, copying the legacy
/// `%LOCALAPPDATA%\TokenTracker\tokentracker.db` (and its backgrounds) on first run.
pub fn default_db_path() -> PathBuf {
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let base = PathBuf::from(local_app_data);
        let target_dir = base.join("TokenVane");
        let target_db = target_dir.join("tokenvane.db");
        if !target_db.exists() {
            let legacy_dir = base.join("TokenTracker");
            let legacy_db = legacy_dir.join("tokentracker.db");
            if legacy_db.is_file() {
                let _ = std::fs::create_dir_all(&target_dir);
                let _ = std::fs::copy(&legacy_db, &target_db);
                let legacy_bg = legacy_dir.join("backgrounds");
                let target_bg = target_dir.join("backgrounds");
                if legacy_bg.is_dir() && !target_bg.exists() {
                    let _ = std::fs::create_dir_all(&target_bg);
                    if let Ok(entries) = std::fs::read_dir(&legacy_bg) {
                        for entry in entries.flatten() {
                            let src = entry.path();
                            if src.is_file() {
                                let _ = std::fs::copy(&src, target_bg.join(entry.file_name()));
                            }
                        }
                    }
                }
            }
        }
        return target_db;
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home)
            .join(".tokenvane")
            .join("tokenvane.db");
    }
    PathBuf::from("tokenvane.db")
}

/// Shared Tauri managed application state.
pub struct AppState {
    pub db_path: PathBuf,
    pub storage: Storage,
    pub coordinator: CollectorCoordinator,
    pub credentials: Arc<dyn CredentialManager>,
    pub otlp_handle: Mutex<Option<OtlpServerHandle>>,
    pub watcher_shutdown_tx: Mutex<Option<watch::Sender<bool>>>,
    pub catalog_sync_lock: Arc<tokio::sync::Mutex<()>>,
    pub sync_client: SyncClientHandle,
}

impl AppState {
    pub fn initialize() -> Result<Self, String> {
        let db_path = default_db_path();
        let storage = Storage::open_file(&db_path)
            .map_err(|e| format!("Failed to open TokenVane SQLite database: {e}"))?;
        let coordinator = CollectorCoordinator::new(storage.clone());
        let credentials: Arc<dyn CredentialManager> = Arc::new(OsKeyringStore::new());
        let sync_client = SyncClientHandle::new(storage.clone(), Arc::clone(&credentials))
            .map_err(|e| format!("Failed to initialize cloud sync client: {e}"))?;

        Ok(Self {
            db_path,
            storage,
            coordinator,
            credentials,
            otlp_handle: Mutex::new(None),
            watcher_shutdown_tx: Mutex::new(None),
            catalog_sync_lock: Arc::new(tokio::sync::Mutex::new(())),
            sync_client,
        })
    }

    pub async fn run_catalog_sync(&self, app: &AppHandle, force: bool) -> CoreResult<CatalogSyncResult> {
        run_catalog_sync_shared(&self.storage, &self.catalog_sync_lock, app, force).await
    }

    pub fn start_background_services(&self, app_handle: AppHandle) {
        let (tx, rx) = watch::channel(false);
        let mut catalog_rx = tx.subscribe();
        let mut quota_rx = tx.subscribe();
        let activity_shutdown_rx = tx.subscribe();
        let (activity_tx, activity_rx) = mpsc::unbounded_channel::<Vec<String>>();
        let watcher_app = app_handle.clone();
        let coord = self.coordinator.clone();
        let sync_for_watcher = self.sync_client.clone();
        tauri::async_runtime::spawn(async move {
            coord
                .run_background_watcher_with_hook(rx, move |report| {
                    if report.events_inserted > 0 || report.rate_limits_updated > 0 {
                        let _ = watcher_app.emit("usage-updated", report);
                        sync_for_watcher.notify_local_change();
                    }
                    if !report.active_providers.is_empty() {
                        let _ = activity_tx.send(report.active_providers.clone());
                    }
                })
                .await;
        });

        tauri::async_runtime::spawn(run_activity_quota_refresher(
            self.storage.clone(),
            Arc::clone(&self.credentials),
            app_handle.clone(),
            self.sync_client.clone(),
            activity_rx,
            activity_shutdown_rx,
        ));

        let sync_client = self.sync_client.clone();
        let sync_app = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            sync_client.start_background(move |report| {
                let _ = sync_app.emit("usage-updated", report);
            });
        });

        let quota_storage = self.storage.clone();
        let quota_creds = Arc::clone(&self.credentials);
        let quota_app = app_handle.clone();
        let quota_sync = self.sync_client.clone();
        tauri::async_runtime::spawn(async move {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(4)) => {}
                _ = quota_rx.changed() => {
                    return;
                }
            }

            poll_configured_provider_apis(&quota_storage, quota_creds.as_ref(), &quota_app, &quota_sync).await;

            let mut interval = tokio::time::interval(Duration::from_secs(300));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            interval.tick().await;

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        poll_configured_provider_apis(&quota_storage, quota_creds.as_ref(), &quota_app, &quota_sync).await;
                    }
                    _ = quota_rx.changed() => {
                        if *quota_rx.borrow() {
                            break;
                        }
                    }
                }
            }
        });

        let sync_storage = self.storage.clone();
        let sync_lock = self.catalog_sync_lock.clone();
        tauri::async_runtime::spawn(async move {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(3)) => {}
                _ = catalog_rx.changed() => {
                    return;
                }
            }

            let _ = run_catalog_sync_shared(&sync_storage, &sync_lock, &app_handle, false).await;

            let mut interval = tokio::time::interval(Duration::from_secs(6 * 3_600));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // Consume immediate initial tick since startup sync just ran
            interval.tick().await;

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let _ = run_catalog_sync_shared(&sync_storage, &sync_lock, &app_handle, false).await;
                    }
                    _ = catalog_rx.changed() => {
                        if *catalog_rx.borrow() {
                            break;
                        }
                    }
                }
            }
        });

        if let Ok(mut guard) = self.watcher_shutdown_tx.lock() {
            *guard = Some(tx);
        }

        if let Ok(prefs) = self.storage.get_preferences() {
            if prefs.otlp_enabled {
                if let Err(err) = self.sync_otlp_listener_blocking(true, prefs.otlp_port) {
                    tracing::warn!("OTLP listener startup warning: {err}");
                }
            }
        }
    }

    /// Synchronize the loopback OTLP listener from a non-Tokio thread (such as `.setup`).
    pub fn sync_otlp_listener_blocking(
        &self,
        enabled: bool,
        port: u16,
    ) -> Result<Option<u16>, String> {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(
                "sync_otlp_listener_blocking cannot run inside an active Tokio runtime".to_string(),
            );
        }
        tauri::async_runtime::block_on(self.sync_otlp_listener_async(enabled, port))
    }

    /// Synchronize the loopback OTLP listener asynchronously without holding locks across `.await`.
    pub async fn sync_otlp_listener_async(
        &self,
        enabled: bool,
        port: u16,
    ) -> Result<Option<u16>, String> {
        let existing_to_stop = {
            let mut guard = self
                .otlp_handle
                .lock()
                .map_err(|_| "OTLP handle lock poisoned".to_string())?;
            if let Some(existing) = guard.take() {
                if enabled && existing.port == port {
                    let active_port = existing.port;
                    *guard = Some(existing);
                    return Ok(Some(active_port));
                }
                Some(existing)
            } else {
                None
            }
        };

        if let Some(existing) = existing_to_stop {
            existing.stop().await;
        }

        if !enabled {
            return Ok(None);
        }

        let handle = start_loopback_otlp_server(self.storage.clone(), port)
            .await
            .map_err(|e| format!("Failed to start loopback OTLP server on 127.0.0.1:{port}: {e}"))?;

        let actual_port = handle.port;
        let mut guard = self
            .otlp_handle
            .lock()
            .map_err(|_| "OTLP handle lock poisoned".to_string())?;
        *guard = Some(handle);
        Ok(Some(actual_port))
    }

    pub fn otlp_status(&self) -> (bool, Option<u16>) {
        match self.otlp_handle.lock() {
            Ok(guard) => match guard.as_ref() {
                Some(h) => (true, Some(h.port)),
                None => (false, None),
            },
            Err(_) => (false, None),
        }
    }

    pub fn shutdown_background_services(&self) {
        self.sync_client.shutdown();
        if let Ok(mut guard) = self.watcher_shutdown_tx.lock() {
            if let Some(tx) = guard.take() {
                let _ = tx.send(true);
            }
        }
        if let Ok(mut guard) = self.otlp_handle.lock() {
            if let Some(handle) = guard.take() {
                handle.signal_shutdown();
            }
        }
    }
}

pub async fn run_catalog_sync_shared(
    storage: &Storage,
    lock: &tokio::sync::Mutex<()>,
    app: &AppHandle,
    force: bool,
) -> CoreResult<CatalogSyncResult> {
    let _guard = lock.lock().await;
    let res = sync_catalog_and_rate(storage, force).await?;
    if res.rate_updated || res.subscriptions_updated {
        if let Ok(prefs) = storage.get_preferences() {
            let _ = app.emit("preferences-updated", &prefs);
        }
    }
    if res.rate_updated || res.pricing_updated || res.subscriptions_updated {
        let _ = app.emit("usage-updated", &res);
    }
    Ok(res)
}

async fn poll_configured_provider_apis(
    storage: &Storage,
    credentials: &dyn CredentialManager,
    app: &AppHandle,
    sync_client: &SyncClientHandle,
) {
    let allow_background_cli = storage
        .get_preferences()
        .map(|p| p.background_cli_quota_poll)
        .unwrap_or(true);
    let fullscreen_busy = tokvane_collectors::is_fullscreen_busy();
    let Ok(configs) = storage.list_provider_configs() else {
        return;
    };
    let mut any_updated = false;
    let mut any_rate_limits = false;

    for cfg in configs {
        if !quota_refresh_available(storage, &cfg, allow_background_cli, fullscreen_busy) {
            continue;
        }
        match tokvane_collectors::refresh_provider_balance(storage, &cfg.provider_id, credentials)
            .await
        {
            Ok(outcome) => {
                if !outcome.from_cache {
                    any_updated = true;
                    let _ = storage.set_provider_sync_status(
                        &cfg.provider_id,
                        Some(outcome.snapshot.fetched_at_ms),
                        None,
                    );
                }
                if outcome.rate_limits_updated > 0 {
                    any_rate_limits = true;
                }
            }
            Err(err) => {
                tracing::debug!("Background API poll warning for {}: {err}", cfg.provider_id);
            }
        }
    }

    if any_updated {
        let _ = app.emit("usage-updated", "api_poll");
    }
    if any_rate_limits {
        sync_client.notify_local_change();
    }
}

/// Whether the provider's official quota CLI (`agy`, `claude`) is installed locally.
fn official_cli_installed(provider_id: &str) -> bool {
    match provider_id {
        provider_ids::ANTIGRAVITY => {
            tokvane_collectors::providers::antigravity::resolve_agy_cli_binary().is_some()
        }
        provider_ids::CLAUDE_CODE => {
            tokvane_collectors::providers::claude_cli::resolve_claude_cli_binary().is_some()
        }
        _ => false,
    }
}

fn background_cli_quota_eligible(
    allow_background_cli: bool,
    cli_installed: bool,
    has_prior_snapshot: bool,
    auth_paused: bool,
    fullscreen_busy: bool,
) -> bool {
    allow_background_cli && !fullscreen_busy && cli_installed && has_prior_snapshot && !auth_paused
}

/// Whether an enabled provider's official quota / balance can be refreshed in the background:
/// CLI-backed providers require the user opt-in preference (`background_cli_quota_poll`), no
/// fullscreen/DND state, the binary on disk, at least one prior successful snapshot (so unconfigured
/// CLIs are never probed blindly), and no persisted auth/timeout pause; API-key providers require a
/// stored credential.
///
/// [`activity_refresh_kind`] is the single source of truth for which providers qualify.
fn quota_refresh_available(
    storage: &Storage,
    cfg: &ProviderConfig,
    allow_background_cli: bool,
    fullscreen_busy: bool,
) -> bool {
    if !cfg.enabled {
        return false;
    }
    match activity_refresh_kind(&cfg.provider_id) {
        Some(QuotaRefreshKind::OfficialApiKey) => cfg.has_credential,
        Some(QuotaRefreshKind::OfficialCli) => {
            if !allow_background_cli || fullscreen_busy || !official_cli_installed(&cfg.provider_id) {
                return false;
            }
            let has_prior_snapshot = storage
                .get_provider_balance(&cfg.provider_id)
                .ok()
                .flatten()
                .is_some();
            let auth_paused = storage
                .get_cli_auth_paused_reason(&cfg.provider_id)
                .ok()
                .flatten()
                .is_some();
            background_cli_quota_eligible(
                allow_background_cli,
                true,
                has_prior_snapshot,
                auth_paused,
                fullscreen_busy,
            )
        }
        None => false,
    }
}

/// Whether a provider with fresh log activity can have its official quota refreshed in the background.
fn activity_refresh_eligible(storage: &Storage, provider_id: &str) -> bool {
    if activity_refresh_kind(provider_id).is_none() {
        return false;
    }
    let allow_background_cli = storage
        .get_preferences()
        .map(|p| p.background_cli_quota_poll)
        .unwrap_or(true);
    let fullscreen_busy = tokvane_collectors::is_fullscreen_busy();
    storage
        .list_provider_configs()
        .ok()
        .and_then(|configs| configs.into_iter().find(|c| c.provider_id == provider_id))
        .is_some_and(|cfg| quota_refresh_available(storage, &cfg, allow_background_cli, fullscreen_busy))
}

/// Refresh official quota shortly after a provider shows new local activity (at most once per
/// 60s for API-key providers or 5 minutes for opt-in CLI probes, with one trailing refresh after
/// a burst), so quota bars track live usage instead of waiting for the 5-minute poll.
async fn run_activity_quota_refresher(
    storage: Storage,
    credentials: Arc<dyn CredentialManager>,
    app: AppHandle,
    sync_client: SyncClientHandle,
    mut activity_rx: mpsc::UnboundedReceiver<Vec<String>>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let throttle = Arc::new(Mutex::new(ActivityRefreshThrottle::new()));
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let start_refresh = |provider_id: String| {
        let storage = storage.clone();
        let credentials = Arc::clone(&credentials);
        let app = app.clone();
        let sync_client = sync_client.clone();
        let throttle = Arc::clone(&throttle);
        tauri::async_runtime::spawn(async move {
            let outcome_res = match activity_refresh_kind(&provider_id) {
                Some(QuotaRefreshKind::OfficialCli) => {
                    tokvane_collectors::refresh_provider_balance(
                        &storage,
                        &provider_id,
                        credentials.as_ref(),
                    )
                    .await
                }
                _ => {
                    tokvane_collectors::refresh_provider_balance_force(
                        &storage,
                        &provider_id,
                        credentials.as_ref(),
                    )
                    .await
                }
            };
            match outcome_res {
                Ok(outcome) => {
                    let _ = storage.set_provider_sync_status(
                        &provider_id,
                        Some(outcome.snapshot.fetched_at_ms),
                        None,
                    );
                    let _ = app.emit("usage-updated", "activity_quota_refresh");
                    if outcome.rate_limits_updated > 0 {
                        sync_client.notify_local_change();
                    }
                }
                Err(err) => {
                    tracing::debug!("Activity quota refresh warning for {provider_id}: {err}");
                }
            }
            if let Ok(mut guard) = throttle.lock() {
                guard.finish(&provider_id);
            }
        });
    };

    loop {
        tokio::select! {
            msg = activity_rx.recv() => {
                let Some(provider_ids) = msg else {
                    break;
                };
                let now_ms = chrono::Utc::now().timestamp_millis();
                for provider_id in provider_ids {
                    if !activity_refresh_eligible(&storage, &provider_id) {
                        continue;
                    }
                    let start_now = throttle
                        .lock()
                        .map(|mut guard| guard.on_activity(&provider_id, now_ms))
                        .unwrap_or(false);
                    if start_now {
                        start_refresh(provider_id);
                    }
                }
            }
            _ = tick.tick() => {
                let now_ms = chrono::Utc::now().timestamp_millis();
                let due = throttle
                    .lock()
                    .map(|mut guard| guard.take_due(now_ms))
                    .unwrap_or_default();
                for provider_id in due {
                    if activity_refresh_eligible(&storage, &provider_id) {
                        start_refresh(provider_id);
                    } else if let Ok(mut guard) = throttle.lock() {
                        guard.finish(&provider_id);
                    }
                }
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokvane_core::domain::CollectionMode;

    fn make_cfg(provider_id: &str, enabled: bool, has_credential: bool) -> ProviderConfig {
        ProviderConfig {
            provider_id: provider_id.to_string(),
            enabled,
            collection_mode: CollectionMode::LocalLog,
            custom_log_paths: Vec::new(),
            has_credential,
            last_sync_at_ms: None,
            last_error: None,
            updated_at_ms: 0,
        }
    }

    #[test]
    fn background_cli_quota_poll_requires_opt_in_prior_sync_no_auth_pause_and_not_fullscreen() {
        assert!(background_cli_quota_eligible(true, true, true, false, false));
        assert!(!background_cli_quota_eligible(false, true, true, false, false));
        assert!(!background_cli_quota_eligible(true, false, true, false, false));
        assert!(!background_cli_quota_eligible(true, true, false, false, false));
        assert!(!background_cli_quota_eligible(true, true, true, true, false));
        assert!(!background_cli_quota_eligible(true, true, true, false, true));

        let storage = Storage::open_in_memory().expect("in-memory db");
        let agy_cfg = make_cfg(provider_ids::ANTIGRAVITY, true, false);
        let claude_cfg = make_cfg(provider_ids::CLAUDE_CODE, true, false);
        let kimi_with_key = make_cfg(provider_ids::KIMI_CODE, true, true);
        let kimi_without_key = make_cfg(provider_ids::KIMI_CODE, true, false);
        let kimi_disabled = make_cfg(provider_ids::KIMI_CODE, false, true);

        // Fresh DB has no prior snapshot for CLI providers, so background CLI polling stays off.
        assert!(!quota_refresh_available(&storage, &agy_cfg, true, false));
        assert!(!quota_refresh_available(&storage, &claude_cfg, true, false));
        assert!(!quota_refresh_available(&storage, &agy_cfg, false, false));
        assert!(!quota_refresh_available(&storage, &agy_cfg, true, true));

        // API-key providers remain unaffected by CLI flags or fullscreen state.
        assert!(quota_refresh_available(&storage, &kimi_with_key, false, false));
        assert!(quota_refresh_available(&storage, &kimi_with_key, false, true));
        assert!(quota_refresh_available(&storage, &kimi_with_key, true, false));
        assert!(!quota_refresh_available(&storage, &kimi_without_key, true, false));
        assert!(!quota_refresh_available(&storage, &kimi_disabled, true, false));
    }
}
