use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokvane_core::domain::{
    provider_ids, ObservedRateLimitWindow, ProviderBalanceSnapshot,
};
use tokvane_core::pricing::parse_official_balance_payload;
use tokvane_core::Storage;

use crate::credentials::CredentialManager;
use crate::error::{CollectorError, CollectorResult};
use crate::providers::claude_cli::CLAUDE_CLI_ENDPOINT;

pub const DEFAULT_MOONSHOT_BALANCE_URL: &str = "https://api.moonshot.cn/v1/users/me/balance";
pub const DEFAULT_MOONSHOT_GLOBAL_BALANCE_URL: &str =
    "https://api.moonshot.ai/v1/users/me/balance";
pub const DEFAULT_KIMI_CODING_USAGES_URL: &str = "https://api.kimi.com/coding/v1/usages";
pub const DEFAULT_DEEPSEEK_BALANCE_URL: &str = "https://api.deepseek.com/user/balance";
pub const DEFAULT_QCODE_USAGE_URL: &str = "https://api.qcode.cc/api/v1/usage";
pub const DEFAULT_MINIMAX_REMAINS_URL: &str = "https://www.minimaxi.com/v1/token_plan/remains";
pub const ANTIGRAVITY_CLI_ENDPOINT: &str = "agy -p \"/usage\" --output-format json";
pub const BALANCE_COOLDOWN_MS: i64 = 60_000;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Summary returned after reconciling local metrics against an official vendor Usage/Cost API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfficialApiReconcileResult {
    pub provider_id: String,
    pub endpoint: String,
    pub status_code: u16,
    pub buckets_received: usize,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub reported_cost_usd: Option<f64>,
    pub checked_at_ms: i64,
}

/// Configurable endpoints for official public Bearer balance / quota queries (overridable in tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalanceApiEndpoints {
    pub moonshot_url: String,
    pub moonshot_global_url: String,
    pub kimi_coding_url: String,
    pub deepseek_url: String,
    pub qcode_url: String,
    pub minimax_url: String,
}

impl Default for BalanceApiEndpoints {
    fn default() -> Self {
        Self {
            moonshot_url: DEFAULT_MOONSHOT_BALANCE_URL.to_string(),
            moonshot_global_url: DEFAULT_MOONSHOT_GLOBAL_BALANCE_URL.to_string(),
            kimi_coding_url: DEFAULT_KIMI_CODING_USAGES_URL.to_string(),
            deepseek_url: DEFAULT_DEEPSEEK_BALANCE_URL.to_string(),
            qcode_url: DEFAULT_QCODE_USAGE_URL.to_string(),
            minimax_url: DEFAULT_MINIMAX_REMAINS_URL.to_string(),
        }
    }
}

/// Outcome returned when refreshing a provider's official balance / quota snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceRefreshOutcome {
    pub snapshot: ProviderBalanceSnapshot,
    pub rate_limits_updated: usize,
    pub endpoint: String,
    pub from_cache: bool,
    pub cooldown_remaining_secs: u64,
}

/// Minimal authenticated GET abstraction so official balance polling can be tested offline.
pub trait AuthHttpFetcher: Send + Sync {
    fn fetch_bearer_get<'a>(
        &'a self,
        url: &'a str,
        bearer_token: &'a str,
    ) -> BoxFuture<'a, Result<(u16, String), String>>;
}

/// Production HTTP client honoring strict connect/request timeouts.
#[derive(Debug, Clone)]
pub struct ReqwestAuthFetcher {
    client: Client,
}

impl ReqwestAuthFetcher {
    pub fn new() -> CollectorResult<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(12))
            .user_agent(concat!("TokenVane/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { client })
    }
}

impl AuthHttpFetcher for ReqwestAuthFetcher {
    fn fetch_bearer_get<'a>(
        &'a self,
        url: &'a str,
        bearer_token: &'a str,
    ) -> BoxFuture<'a, Result<(u16, String), String>> {
        Box::pin(async move {
            let resp = self
                .client
                .get(url)
                .bearer_auth(bearer_token)
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|e| format!("Network request failed: {e}"))?;
            let status = resp.status().as_u16();
            let text = resp
                .text()
                .await
                .map_err(|e| format!("Failed reading response body: {e}"))?;
            Ok((status, text))
        })
    }
}

fn candidate_endpoints_for_provider(
    provider_id: &str,
    api_key: &str,
    endpoints: &BalanceApiEndpoints,
) -> CollectorResult<Vec<String>> {
    match provider_id {
        provider_ids::KIMI_CODE => {
            let mut list = Vec::new();
            let push_unique = |out: &mut Vec<String>, url: &str| {
                if !url.trim().is_empty() && !out.iter().any(|u| u == url) {
                    out.push(url.to_string());
                }
            };
            if api_key.trim().starts_with("sk-kimi-") {
                push_unique(&mut list, &endpoints.kimi_coding_url);
            } else {
                push_unique(&mut list, &endpoints.moonshot_url);
                push_unique(&mut list, &endpoints.moonshot_global_url);
                push_unique(&mut list, &endpoints.kimi_coding_url);
            }
            Ok(list)
        }
        provider_ids::DEEPSEEK_HARNESS => Ok(vec![endpoints.deepseek_url.clone()]),
        provider_ids::QCODE => Ok(vec![endpoints.qcode_url.clone()]),
        provider_ids::MINIMAX_CODE => Ok(vec![endpoints.minimax_url.clone()]),
        other => Err(CollectorError::Compliance(format!(
            "Provider '{other}' does not expose a documented public REST balance/quota API for simple Bearer keys; use local logs or subscription window rules"
        ))),
    }
}

fn default_endpoint_label(provider_id: &str, endpoints: &BalanceApiEndpoints) -> CollectorResult<String> {
    match provider_id {
        provider_ids::ANTIGRAVITY => Ok(ANTIGRAVITY_CLI_ENDPOINT.to_string()),
        provider_ids::CLAUDE_CODE => Ok(CLAUDE_CLI_ENDPOINT.to_string()),
        provider_ids::KIMI_CODE => Ok(endpoints.moonshot_url.clone()),
        provider_ids::DEEPSEEK_HARNESS => Ok(endpoints.deepseek_url.clone()),
        provider_ids::QCODE => Ok(endpoints.qcode_url.clone()),
        provider_ids::MINIMAX_CODE => Ok(endpoints.minimax_url.clone()),
        other => Err(CollectorError::Compliance(format!(
            "Provider '{other}' does not expose a documented public REST balance/quota API for simple Bearer keys; use local logs or subscription window rules"
        ))),
    }
}

/// Call documented official organization usage/cost APIs using a key stored in OS Credential Manager.
/// Strictly rejects any provider without a documented public billing/usage endpoint.
pub async fn reconcile_official_usage_api(
    provider_id: &str,
    credentials: &dyn CredentialManager,
    lookback_days: i64,
) -> CollectorResult<OfficialApiReconcileResult> {
    let api_key = credentials
        .get_api_key(provider_id)?
        .ok_or_else(|| CollectorError::Credential(format!("No API key configured for {provider_id}")))?;

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let now = Utc::now();
    let start = now - chrono::Duration::days(lookback_days.clamp(1, 31));

    match provider_id {
        provider_ids::CLAUDE_CODE => {
            let url = format!(
                "https://api.anthropic.com/v1/organizations/usage_report/messages?starting_at={}&ending_at={}&bucket_width=1d",
                start.to_rfc3339(),
                now.to_rfc3339()
            );
            let resp = client
                .get(&url)
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .send()
                .await?;
            let status_code = resp.status().as_u16();
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            if status_code >= 400 {
                return Err(CollectorError::Compliance(format!(
                    "Anthropic Admin API returned HTTP {status_code}: {}",
                    body.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("request rejected")
                )));
            }
            let buckets = body
                .get("data")
                .and_then(|d| d.as_array())
                .map(Vec::len)
                .unwrap_or(0);
            let (in_tok, out_tok) = sum_anthropic_buckets(&body);
            Ok(OfficialApiReconcileResult {
                provider_id: provider_id.to_string(),
                endpoint: "https://api.anthropic.com/v1/organizations/usage_report/messages"
                    .to_string(),
                status_code,
                buckets_received: buckets,
                total_input_tokens: in_tok,
                total_output_tokens: out_tok,
                reported_cost_usd: None,
                checked_at_ms: now.timestamp_millis(),
            })
        }
        provider_ids::CODEX => {
            let start_sec = start.timestamp();
            let end_sec = now.timestamp();
            let url = format!(
                "https://api.openai.com/v1/organization/usage/completions?start_time={start_sec}&end_time={end_sec}&bucket_width=1d"
            );
            let resp = client
                .get(&url)
                .bearer_auth(&api_key)
                .send()
                .await?;
            let status_code = resp.status().as_u16();
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            if status_code >= 400 {
                return Err(CollectorError::Compliance(format!(
                    "OpenAI Usage API returned HTTP {status_code}: {}",
                    body.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("request rejected")
                )));
            }
            let buckets = body
                .get("data")
                .and_then(|d| d.as_array())
                .map(Vec::len)
                .unwrap_or(0);
            let (in_tok, out_tok) = sum_openai_buckets(&body);
            Ok(OfficialApiReconcileResult {
                provider_id: provider_id.to_string(),
                endpoint: "https://api.openai.com/v1/organization/usage/completions".to_string(),
                status_code,
                buckets_received: buckets,
                total_input_tokens: in_tok,
                total_output_tokens: out_tok,
                reported_cost_usd: None,
                checked_at_ms: now.timestamp_millis(),
            })
        }
        other => Err(CollectorError::Compliance(format!(
            "Provider '{other}' does not expose a documented public organization usage API"
        ))),
    }
}

/// Query a provider's official public balance/quota endpoint using the Bearer API key in `credentials`,
/// honoring a strict 60-second cooldown and persisting results into SQLite `provider_balances`.
pub async fn refresh_provider_balance(
    storage: &Storage,
    provider_id: &str,
    credentials: &dyn CredentialManager,
) -> CollectorResult<BalanceRefreshOutcome> {
    let fetcher = ReqwestAuthFetcher::new()?;
    refresh_provider_balance_with_fetcher_inner(
        storage,
        provider_id,
        credentials,
        &fetcher,
        &BalanceApiEndpoints::default(),
        Utc::now().timestamp_millis(),
        false,
    )
    .await
}

/// Immediately force-refresh a provider's balance/quota bypassing the 60-second cooldown
/// (e.g. right after saving a new API key).
pub async fn refresh_provider_balance_force(
    storage: &Storage,
    provider_id: &str,
    credentials: &dyn CredentialManager,
) -> CollectorResult<BalanceRefreshOutcome> {
    let fetcher = ReqwestAuthFetcher::new()?;
    refresh_provider_balance_with_fetcher_inner(
        storage,
        provider_id,
        credentials,
        &fetcher,
        &BalanceApiEndpoints::default(),
        Utc::now().timestamp_millis(),
        true,
    )
    .await
}

/// Deterministic testable entry point supporting injected HTTP fetcher and endpoints.
pub async fn refresh_provider_balance_with_fetcher(
    storage: &Storage,
    provider_id: &str,
    credentials: &dyn CredentialManager,
    fetcher: &dyn AuthHttpFetcher,
    endpoints: &BalanceApiEndpoints,
    now_ms: i64,
) -> CollectorResult<BalanceRefreshOutcome> {
    refresh_provider_balance_with_fetcher_inner(
        storage,
        provider_id,
        credentials,
        fetcher,
        endpoints,
        now_ms,
        false,
    )
    .await
}

/// Testable Antigravity `agy` CLI quota polling entry point with 60-second cooldown & persistence.
pub async fn refresh_antigravity_cli_quota_with_runner<F, Fut>(
    storage: &Storage,
    now_ms: i64,
    bypass_cooldown: bool,
    runner: F,
) -> CollectorResult<BalanceRefreshOutcome>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = CollectorResult<(ProviderBalanceSnapshot, Vec<ObservedRateLimitWindow>)>>,
{
    refresh_cli_quota_with_runner(
        storage,
        provider_ids::ANTIGRAVITY,
        ANTIGRAVITY_CLI_ENDPOINT,
        now_ms,
        bypass_cooldown,
        runner,
    )
    .await
}

fn is_cli_auth_or_timeout_error(provider_id: &str, msg: &str) -> bool {
    match provider_id {
        provider_ids::ANTIGRAVITY => {
            crate::providers::antigravity::is_agy_auth_or_interactive_error(msg)
        }
        provider_ids::CLAUDE_CODE => {
            crate::providers::claude_cli::is_claude_auth_or_timeout_error(msg)
        }
        _ => false,
    }
}

/// Testable official CLI quota polling entry point (Antigravity `agy`, Claude Code `claude`) with
/// 60-second cooldown. On success the provider's `official_cli` windows are atomically replaced and
/// any persisted auth pause is cleared; on auth/timeout failure the pause reason is persisted in
/// SQLite so background polling stays quiet across restarts until the user manually syncs/logs in.
pub async fn refresh_cli_quota_with_runner<F, Fut>(
    storage: &Storage,
    provider_id: &str,
    endpoint: &str,
    now_ms: i64,
    bypass_cooldown: bool,
    runner: F,
) -> CollectorResult<BalanceRefreshOutcome>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = CollectorResult<(ProviderBalanceSnapshot, Vec<ObservedRateLimitWindow>)>>,
{
    if let Some(reason) = storage.get_cli_auth_paused_reason(provider_id)? {
        return Err(CollectorError::Compliance(reason));
    }

    if !bypass_cooldown {
        if let Some(cached) = storage.get_provider_balance(provider_id)? {
            let age_ms = now_ms.saturating_sub(cached.fetched_at_ms);
            if (0..BALANCE_COOLDOWN_MS).contains(&age_ms) {
                let remaining_ms = BALANCE_COOLDOWN_MS - age_ms;
                let remaining_secs = (remaining_ms.max(1) as u64).div_ceil(1000);
                return Ok(BalanceRefreshOutcome {
                    snapshot: cached,
                    rate_limits_updated: 0,
                    endpoint: endpoint.to_string(),
                    from_cache: true,
                    cooldown_remaining_secs: remaining_secs,
                });
            }
        }
    }

    match runner().await {
        Ok((snapshot, rate_limits)) => {
            let rate_limits_updated = rate_limits.len();
            storage.replace_provider_source_rate_limits(provider_id, "official_cli", &rate_limits)?;
            storage.upsert_provider_balance(&snapshot)?;
            let _ = storage.set_cli_auth_paused_reason(provider_id, None);
            let _ = storage.set_provider_sync_status(provider_id, Some(now_ms), None);
            Ok(BalanceRefreshOutcome {
                snapshot,
                rate_limits_updated,
                endpoint: endpoint.to_string(),
                from_cache: false,
                cooldown_remaining_secs: 0,
            })
        }
        Err(err) => {
            let msg = err.to_string();
            if is_cli_auth_or_timeout_error(provider_id, &msg) {
                let _ = storage.set_cli_auth_paused_reason(provider_id, Some(&msg));
            }
            let _ = storage.set_provider_sync_status(provider_id, None, Some(&msg));
            Err(err)
        }
    }
}

pub async fn refresh_provider_balance_with_fetcher_inner(
    storage: &Storage,
    provider_id: &str,
    credentials: &dyn CredentialManager,
    fetcher: &dyn AuthHttpFetcher,
    endpoints: &BalanceApiEndpoints,
    now_ms: i64,
    bypass_cooldown: bool,
) -> CollectorResult<BalanceRefreshOutcome> {
    if provider_id.trim().eq_ignore_ascii_case(provider_ids::ANTIGRAVITY) {
        return refresh_antigravity_cli_quota_with_runner(storage, now_ms, bypass_cooldown, || {
            crate::providers::antigravity::collect_antigravity_cli_usage(now_ms)
        })
        .await;
    }
    if provider_id.trim().eq_ignore_ascii_case(provider_ids::CLAUDE_CODE) {
        return refresh_cli_quota_with_runner(
            storage,
            provider_ids::CLAUDE_CODE,
            CLAUDE_CLI_ENDPOINT,
            now_ms,
            bypass_cooldown,
            || crate::providers::claude_cli::collect_claude_cli_usage(now_ms),
        )
        .await;
    }

    let default_ep = default_endpoint_label(provider_id, endpoints)?;

    if !bypass_cooldown {
        if let Some(cached) = storage.get_provider_balance(provider_id)? {
            let age_ms = now_ms.saturating_sub(cached.fetched_at_ms);
            if (0..BALANCE_COOLDOWN_MS).contains(&age_ms) {
                let remaining_ms = BALANCE_COOLDOWN_MS - age_ms;
                let remaining_secs = (remaining_ms.max(1) as u64).div_ceil(1000);
                return Ok(BalanceRefreshOutcome {
                    snapshot: cached,
                    rate_limits_updated: 0,
                    endpoint: default_ep,
                    from_cache: true,
                    cooldown_remaining_secs: remaining_secs,
                });
            }
        }
    }

    let api_key = credentials
        .get_api_key(provider_id)?
        .ok_or_else(|| CollectorError::Credential(format!("请先录入 {provider_id} 官方 API Key")))?;

    let candidates = candidate_endpoints_for_provider(provider_id, &api_key, endpoints)?;
    let mut selected_endpoint = default_ep;
    let mut response_body: Option<Value> = None;
    let mut last_error: Option<CollectorError> = None;

    for (idx, candidate_ep) in candidates.iter().enumerate() {
        let fetched = fetcher.fetch_bearer_get(candidate_ep, &api_key).await;
        match fetched {
            Ok((status, raw_body)) => {
                let body: Value = serde_json::from_str(&raw_body).unwrap_or(Value::Null);
                if status < 400 {
                    selected_endpoint = candidate_ep.clone();
                    response_body = Some(body);
                    break;
                }
                let detail = body
                    .get("error")
                    .and_then(|e| e.get("message").or(Some(e)))
                    .or_else(|| body.get("message"))
                    .or_else(|| body.get("msg"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("request rejected");
                let err_msg = format!("{provider_id} 官方接口 ({candidate_ep}) 返回 HTTP {status}: {detail}");
                last_error = Some(CollectorError::Compliance(err_msg));
                let retryable_fallback = idx + 1 < candidates.len()
                    && matches!(status, 401 | 403 | 404);
                if !retryable_fallback {
                    break;
                }
            }
            Err(net_err) => {
                last_error = Some(CollectorError::Compliance(net_err));
                if idx + 1 >= candidates.len() {
                    break;
                }
            }
        }
    }

    let body = match response_body {
        Some(b) => b,
        None => {
            return Err(last_error.unwrap_or_else(|| {
                CollectorError::Compliance(format!("{provider_id} 官方接口请求失败"))
            }))
        }
    };

    let (snapshot, rate_limits) =
        parse_official_balance_payload(provider_id, &selected_endpoint, &body, now_ms)
            .map_err(|e| CollectorError::Compliance(e.to_string()))?;

    storage.upsert_provider_balance(&snapshot)?;
    let rate_limits_updated = rate_limits.len();
    if !rate_limits.is_empty() {
        storage.upsert_observed_rate_limits(&rate_limits)?;
    }

    Ok(BalanceRefreshOutcome {
        snapshot,
        rate_limits_updated,
        endpoint: selected_endpoint,
        from_cache: false,
        cooldown_remaining_secs: 0,
    })
}

fn sum_anthropic_buckets(body: &Value) -> (i64, i64) {
    let mut input = 0i64;
    let mut output = 0i64;
    if let Some(buckets) = body.get("data").and_then(|d| d.as_array()) {
        for b in buckets {
            if let Some(results) = b.get("results").and_then(|r| r.as_array()) {
                for r in results {
                    input += r.get("uncached_input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                    input += r.get("cache_read_input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                    output += r.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                }
            }
        }
    }
    (input, output)
}

fn sum_openai_buckets(body: &Value) -> (i64, i64) {
    let mut input = 0i64;
    let mut output = 0i64;
    if let Some(buckets) = body.get("data").and_then(|d| d.as_array()) {
        for b in buckets {
            if let Some(results) = b.get("results").and_then(|r| r.as_array()) {
                for r in results {
                    input += r.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                    output += r.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                }
            }
        }
    }
    (input, output)
}
