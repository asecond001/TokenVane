#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod catalog_sync;
pub(crate) mod cli_process;
pub mod credentials;
pub mod error;
pub mod otlp;
pub mod providers;
pub mod quota_refresh;
pub mod sync_client;
pub mod sync_provision;
pub mod watcher;

pub use catalog_sync::{
    sync_catalog_and_rate, sync_catalog_and_rate_with_endpoints, sync_catalog_and_rate_with_fetcher,
    CatalogSyncEndpoints, ReqwestTextFetcher, TextHttpFetcher,
};
pub use cli_process::is_fullscreen_busy;
pub use credentials::{CredentialManager, MemoryCredentialStore, OsKeyringStore};
pub use error::{CollectorError, CollectorResult};
pub use otlp::{start_loopback_otlp_server, validate_loopback_addr, OtlpServerHandle};
pub use providers::official_api::{
    refresh_cli_quota_with_runner, refresh_provider_balance, refresh_provider_balance_force,
    refresh_provider_balance_with_fetcher, AuthHttpFetcher, BalanceApiEndpoints,
    BalanceRefreshOutcome, ReqwestAuthFetcher, BALANCE_COOLDOWN_MS,
};
pub use sync_client::{
    build_sync_pairing_uri, clear_sync_credentials, has_sync_credentials, normalize_worker_url,
    read_sync_status_view, run_sync_cycle_once, run_sync_cycle_with_transport,
    set_sync_credentials, ReqwestSyncTransport, SseByteStream, SyncBoxFuture, SyncClientHandle,
    SyncHttpTransport, SyncRunReport, SyncStatusView, SYNC_BEARER_TOKEN_KEY,
    SYNC_CF_API_TOKEN_KEY, SYNC_E2EE_PASSPHRASE_KEY,
};
pub use sync_provision::{
    provision_worker, provision_worker_with, provision_worker_with_progress,
    CloudSyncProvisionRequest, CloudflareApiTransport, ProvisionMode, ProvisionProgress,
    ProvisionStep, ProvisionedWorkerArtifacts, WranglerCliRunner, WranglerCmdOutput,
};
pub use watcher::{CollectorCoordinator, ScanReport};
