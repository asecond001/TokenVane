use chrono::{DateTime, NaiveDateTime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Canonical built-in provider identifiers (Phase 1 + Phase 2).
pub mod provider_ids {
    pub const CLAUDE_CODE: &str = "claude_code";
    pub const CODEX: &str = "codex";
    pub const ANTIGRAVITY: &str = "antigravity";
    pub const KIMI_CODE: &str = "kimi_code";
    pub const WORKBUDDY: &str = "workbuddy";
    pub const DEEPSEEK_HARNESS: &str = "deepseek_harness";
    pub const QCODE: &str = "qcode";
    pub const MINIMAX_CODE: &str = "minimax_code";
    pub const CURSOR: &str = "cursor";
    pub const GITHUB_COPILOT: &str = "github_copilot";
    pub const WINDSURF: &str = "windsurf";

    pub const ALL: [&str; 8] = [
        CLAUDE_CODE,
        CODEX,
        ANTIGRAVITY,
        KIMI_CODE,
        WORKBUDDY,
        DEEPSEEK_HARNESS,
        QCODE,
        MINIMAX_CODE,
    ];

    pub fn display_name(provider_id: &str) -> &str {
        match provider_id {
            CLAUDE_CODE => "Claude Code",
            CODEX => "Codex",
            ANTIGRAVITY => "Antigravity",
            KIMI_CODE => "Kimi Code",
            WORKBUDDY => "Workbuddy",
            DEEPSEEK_HARNESS => "DeepSeek Harness",
            QCODE => "QCode",
            MINIMAX_CODE => "MiniMax Code",
            CURSOR => "Cursor",
            GITHUB_COPILOT => "GitHub Copilot",
            WINDSURF => "Windsurf",
            other => other,
        }
    }
}

/// Compliant data acquisition modes permitted by AGENTS.md Section 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectionMode {
    LocalLog,
    Otlp,
    OfficialApi,
}

impl CollectionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalLog => "local_log",
            Self::Otlp => "otlp",
            Self::OfficialApi => "official_api",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "otlp" => Self::Otlp,
            "official_api" => Self::OfficialApi,
            _ => Self::LocalLog,
        }
    }
}

/// Display currency supported in Phase 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum CurrencyCode {
    Usd,
    Cny,
}

impl CurrencyCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usd => "USD",
            Self::Cny => "CNY",
        }
    }

    pub fn parse(value: &str) -> Self {
        if value.eq_ignore_ascii_case("CNY") {
            Self::Cny
        } else {
            Self::Usd
        }
    }
}

/// Local or remote registered device record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub device_id: String,
    pub device_name: String,
    pub platform: String,
    pub created_at_ms: i64,
}

/// Provider collector configuration persisted in SQLite.
/// Note: API keys are NEVER stored here; only `has_credential` boolean flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub provider_id: String,
    pub enabled: bool,
    pub collection_mode: CollectionMode,
    pub custom_log_paths: Vec<String>,
    pub has_credential: bool,
    pub last_sync_at_ms: Option<i64>,
    pub last_error: Option<String>,
    pub updated_at_ms: i64,
}

/// Byte-offset resume cursor for incremental log tailing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestionCursor {
    pub source_path: String,
    pub provider_id: String,
    pub file_identity: String,
    pub byte_offset: u64,
    pub last_line_hash: String,
    pub updated_at_ms: i64,
}

/// Raw parsed event emitted by a compliant collector before/after pricing enrichment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedUsageEvent {
    pub event_id: String,
    pub device_id: String,
    pub provider_id: String,
    pub session_id: String,
    pub project_label: String,
    pub model_id: String,
    pub timestamp_ms: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    /// True when the public log records an observable agent step without public token counters.
    pub observable_only: bool,
    pub duration_ms: Option<i64>,
    pub ttft_ms: Option<i64>,
    pub output_tps: Option<f64>,
    pub estimated_cost_micro_usd: i64,
    pub cache_saved_micro_usd: i64,
}

impl NormalizedUsageEvent {
    pub fn total_tokens(&self) -> i64 {
        self.input_tokens
            + self.output_tokens
            + self.cache_read_tokens
            + self.cache_write_tokens
            + self.reasoning_tokens
    }

    pub fn effective_output_tokens(&self) -> i64 {
        self.output_tokens + self.reasoning_tokens
    }

    pub fn context_tokens(&self) -> i64 {
        self.input_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

/// Deterministic SHA-256 event identifier guaranteeing idempotent re-ingestion.
pub fn compute_event_id(
    provider_id: &str,
    source_session_id: &str,
    step_or_msg_id: &str,
    timestamp_ms: i64,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider_id.as_bytes());
    hasher.update(b"|");
    hasher.update(source_session_id.as_bytes());
    hasher.update(b"|");
    hasher.update(step_or_msg_id.as_bytes());
    hasher.update(b"|");
    hasher.update(timestamp_ms.to_string().as_bytes());
    hex::encode(hasher.finalize())
}

/// Extract a desensitized leaf folder label without retaining sensitive absolute paths.
pub fn sanitize_project_label(raw_path: Option<&str>) -> String {
    let Some(raw) = raw_path else {
        return "workspace".to_string();
    };
    let trimmed = raw.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return "workspace".to_string();
    }
    let leaf = trimmed
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("workspace")
        .trim();
    if leaf.is_empty() {
        "workspace".to_string()
    } else {
        leaf.chars().take(48).collect()
    }
}

/// Pre-aggregated hourly rollup bucket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HourlyRollup {
    pub device_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub bucket_start_ms: i64,
    pub request_count: i64,
    pub observable_step_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    pub estimated_cost_micro_usd: i64,
    pub cache_saved_micro_usd: i64,
    pub total_duration_ms: i64,
    pub tps_sample_count: i64,
    pub tps_sum: f64,
    pub max_output_tps: f64,
    pub updated_at_ms: i64,
}

/// Pre-aggregated daily rollup bucket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DailyRollup {
    pub device_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub bucket_start_ms: i64,
    pub request_count: i64,
    pub observable_step_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    pub estimated_cost_micro_usd: i64,
    pub cache_saved_micro_usd: i64,
    pub total_duration_ms: i64,
    pub tps_sample_count: i64,
    pub tps_sum: f64,
    pub max_output_tps: f64,
    pub updated_at_ms: i64,
}

/// Model pricing catalog entry priced in micro-USD per 1,000,000 tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelPricingEntry {
    pub model_pattern: String,
    pub provider_id: String,
    pub input_per_m_micro_usd: i64,
    pub output_per_m_micro_usd: i64,
    pub cache_read_per_m_micro_usd: i64,
    pub cache_write_per_m_micro_usd: i64,
    pub reasoning_per_m_micro_usd: i64,
    pub is_user_override: bool,
    pub effective_from: String,
}

/// Budget unit used when evaluating a local subscription rolling window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaUnit {
    MicroUsd,
    Tokens,
}

/// Single window rule inside a subscription plan (e.g. 5h rolling or 7d weekly).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionWindowRule {
    pub window_id: String,
    pub label: String,
    pub duration_hours: i64,
    pub rolling: bool,
    pub unit: QuotaUnit,
    pub limit_value: i64,
    pub warning_percent: u8,
    pub critical_percent: u8,
}

/// Evaluated live status for one subscription window (e.g. 5h rolling or 7d weekly).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubscriptionWindowStatus {
    pub window_id: String,
    pub label: String,
    pub duration_hours: i64,
    pub rolling: bool,
    pub unit: QuotaUnit,
    pub used_value: i64,
    pub limit_value: i64,
    pub used_percent: f64,
    pub warning_percent: u8,
    pub critical_percent: u8,
    pub burn_rate_per_hour: f64,
    pub projected_exhaustion_hours: Option<f64>,
    pub resets_at_ms: Option<i64>,
    pub resets_in_seconds: i64,
    /// `"official_api"`, `"official_cli"`, `"official_log_snapshot"`, or `"local_estimation"`.
    pub source: String,
    /// `"normal"` | `"warning"` | `"critical"` | `"exhausted"`.
    pub alert_level: String,
}

/// User-configurable or auto-discovered subscription plan definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionPlan {
    pub plan_id: String,
    pub provider_id: String,
    pub plan_name: String,
    pub enabled: bool,
    pub monthly_fee_micro_usd: i64,
    pub windows: Vec<SubscriptionWindowRule>,
    #[serde(default)]
    pub is_user_configured: bool,
    #[serde(default)]
    pub auto_source: Option<String>,
    pub updated_at_ms: i64,
}

/// Reference template used to prefill a user-configured subscription form and preview live tier utilization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubscriptionPlanTemplate {
    pub template_id: String,
    pub provider_id: String,
    pub plan_name: String,
    pub monthly_fee_micro_usd: i64,
    pub windows: Vec<SubscriptionWindowRule>,
    pub description: String,
    #[serde(default)]
    pub live_windows: Vec<SubscriptionWindowStatus>,
}

/// Official rate limit window snapshot parsed from provider local logs (e.g. Codex) or official quota APIs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservedRateLimitWindow {
    pub provider_id: String,
    pub window_name: String,
    pub used_percent: f64,
    pub window_minutes: Option<i64>,
    pub resets_at_ms: Option<i64>,
    pub observed_at_ms: i64,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub plan_hint: Option<String>,
}

fn default_bg_mode() -> String {
    "none".to_string()
}

fn default_bg_preset_id() -> String {
    "aurora_slate".to_string()
}

fn default_bg_opacity() -> f64 {
    0.18
}

fn default_bg_blur_px() -> i64 {
    16
}

fn default_true() -> bool {
    true
}

/// Persisted desktop/workspace user preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppPreferences {
    pub currency: CurrencyCode,
    pub usd_to_cny_rate: String,
    pub theme: String,
    pub minimize_to_tray: bool,
    pub launch_at_startup: bool,
    pub otlp_enabled: bool,
    pub otlp_port: u16,
    pub device_display_name: String,
    #[serde(default = "default_bg_mode")]
    pub bg_mode: String,
    #[serde(default)]
    pub bg_image_path: Option<String>,
    #[serde(default = "default_bg_preset_id")]
    pub bg_preset_id: String,
    #[serde(default = "default_bg_opacity")]
    pub bg_opacity: f64,
    #[serde(default = "default_bg_blur_px")]
    pub bg_blur_px: i64,
    #[serde(default = "default_true")]
    pub auto_sync_pricing: bool,
    #[serde(default = "default_true")]
    pub exchange_rate_auto: bool,
    #[serde(default)]
    pub last_rate_sync_at_ms: Option<i64>,
    #[serde(default)]
    pub last_pricing_sync_at_ms: Option<i64>,
    #[serde(default)]
    pub last_subscription_sync_at_ms: Option<i64>,
    #[serde(default = "default_true")]
    pub auto_check_updates: bool,
    #[serde(default)]
    pub floating_window_enabled: bool,
    #[serde(default = "default_true")]
    pub background_cli_quota_poll: bool,
    pub updated_at_ms: i64,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            currency: CurrencyCode::Usd,
            usd_to_cny_rate: "7.25".to_string(),
            theme: "dark".to_string(),
            minimize_to_tray: true,
            launch_at_startup: false,
            otlp_enabled: false,
            otlp_port: 4318,
            device_display_name: "win-workstation".to_string(),
            bg_mode: default_bg_mode(),
            bg_image_path: None,
            bg_preset_id: default_bg_preset_id(),
            bg_opacity: default_bg_opacity(),
            bg_blur_px: default_bg_blur_px(),
            auto_sync_pricing: true,
            exchange_rate_auto: true,
            last_rate_sync_at_ms: None,
            last_pricing_sync_at_ms: None,
            last_subscription_sync_at_ms: None,
            auto_check_updates: true,
            floating_window_enabled: false,
            background_cli_quota_poll: true,
            updated_at_ms: 0,
        }
    }
}

/// Measurement caliber behind a provider's per-event `output_tps` samples.
///
/// `Decode` divides output tokens by the streaming window that starts at the first generated
/// token, so queueing / prefill / TTFT are excluded. `EndToEnd` divides by a window anchored at
/// the request (or prompt) start, so it includes TTFT and is systematically lower.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpeedBasis {
    Decode,
    #[default]
    EndToEnd,
}

/// Caliber used by each built-in collector when it records `output_tps`.
///
/// - `claude_code`: first JSONL chunk → last chunk of the same `message.id` (decode window).
/// - `codex`: first `Reasoning`/`AgentMessage` `item_completed.started_at_ms` → `token_usage_record`.
/// - `kimi_code`: `step.end.llmStreamDurationMs` (server decode window).
/// - Everything else (Antigravity, compatible JSONL, OTLP `duration_ms`) measures end-to-end.
pub fn speed_basis_for_provider(provider_id: &str) -> SpeedBasis {
    match provider_id {
        provider_ids::CLAUDE_CODE | provider_ids::CODEX | provider_ids::KIMI_CODE => {
            SpeedBasis::Decode
        }
        _ => SpeedBasis::EndToEnd,
    }
}

/// Minimum generated tokens required before physical throughput (Tokens/s) is reliable.
pub const MIN_VALID_TPS_OUTPUT_TOKENS: i64 = 12;
/// Minimum active generation duration (ms) required to rule out buffered flush bursts.
pub const MIN_VALID_TPS_DURATION_MS: i64 = 500;
/// Upper bound on credible LLM generation throughput (Tokens/s).
pub const MAX_VALID_OUTPUT_TPS: f64 = 450.0;

/// Returns whether a stored or computed TPS sample represents credible token generation speed.
pub fn is_valid_tps_sample(
    output_tokens: i64,
    duration_ms: Option<i64>,
    tps: Option<f64>,
) -> bool {
    let Some(dur) = duration_ms else {
        return false;
    };
    let Some(val) = tps else {
        return false;
    };
    output_tokens >= MIN_VALID_TPS_OUTPUT_TOKENS
        && dur >= MIN_VALID_TPS_DURATION_MS
        && val.is_finite()
        && val > 0.0
        && val <= MAX_VALID_OUTPUT_TPS
}

/// Physical TPS calculation guarded against streaming tail flushes and tiny bursts.
pub fn compute_tps(output_tokens: i64, duration_ms: Option<i64>) -> Option<f64> {
    let dur = duration_ms?;
    if output_tokens < MIN_VALID_TPS_OUTPUT_TOKENS || dur < MIN_VALID_TPS_DURATION_MS {
        return None;
    }
    let tps = output_tokens as f64 / (dur as f64 / 1000.0);
    if is_valid_tps_sample(output_tokens, Some(dur), Some(tps)) {
        Some(tps)
    } else {
        None
    }
}

fn sorted_quantile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let pos = p.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    let frac = pos - lo as f64;
    sorted[lo] + (sorted[hi] - sorted[lo]) * frac
}

/// Compute a non-parametric Tukey IQR upper fence (`max(Q3 + 1.5 * IQR, median * 2.2)`)
/// for a sorted slice of valid TPS samples when at least 4 samples are available.
pub fn compute_iqr_upper_fence(sorted_tps: &[f64]) -> Option<f64> {
    if sorted_tps.len() < 4 {
        return None;
    }
    let q1 = sorted_quantile(sorted_tps, 0.25);
    let median = sorted_quantile(sorted_tps, 0.50);
    let q3 = sorted_quantile(sorted_tps, 0.75);
    let iqr = (q3 - q1).max(0.0);
    let tukey = q3 + 1.5 * iqr;
    let ratio_guard = median * 2.2;
    Some(tukey.max(ratio_guard).min(MAX_VALID_OUTPUT_TPS))
}

/// Smooth a chronological sequence of TPS samples using Tukey IQR outlier rejection
/// followed by an Exponentially Weighted Moving Average (`EWMA`, `alpha` in `0.0..=1.0`).
/// When fewer than 4 samples exist, returns the latest sample directly unless 3 samples
/// show an extreme (>2.4x median) spike, in which case EWMA damps the spike.
pub fn compute_ewma_tps(chronological_samples: &[f64], alpha: f64) -> Option<f64> {
    let valid: Vec<f64> = chronological_samples
        .iter()
        .copied()
        .filter(|v| v.is_finite() && *v > 0.0 && *v <= MAX_VALID_OUTPUT_TPS)
        .collect();
    if valid.is_empty() {
        return None;
    }
    if valid.len() < 3 {
        return valid.last().copied();
    }

    let mut sorted = valid.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let median = sorted_quantile(&sorted, 0.50);
    let fence = compute_iqr_upper_fence(&sorted).unwrap_or(median * 2.4);

    let last = *valid.last().unwrap_or(&median);
    // If all samples are within the normal fence, preserve the crisp latest reading unless smoothing an outlier tail.
    if last <= fence && valid.iter().all(|&v| v <= fence) {
        return Some(last);
    }

    let inliers: Vec<f64> = valid.into_iter().filter(|&v| v <= fence).collect();
    let series = if inliers.is_empty() { &sorted } else { &inliers };
    let clamped_alpha = alpha.clamp(0.1, 0.9);
    let mut ewma = series[0];
    for &sample in &series[1..] {
        ewma = clamped_alpha * sample + (1.0 - clamped_alpha) * ewma;
    }
    Some(ewma)
}

/// Parse an ISO-8601 / RFC-3339 timestamp string or epoch seconds/ms number into Unix epoch ms.
pub fn parse_timestamp_ms_json(val: Option<&serde_json::Value>) -> Option<i64> {
    let v = val?;
    if let Some(num) = v.as_i64() {
        return Some(if num.abs() < 10_000_000_000 {
            num * 1000
        } else {
            num
        });
    }
    if let Some(num_f) = v.as_f64() {
        if num_f.is_finite() {
            let ms = if num_f.abs() < 10_000_000_000.0 {
                (num_f * 1000.0).round() as i64
            } else {
                num_f.round() as i64
            };
            return Some(ms);
        }
    }
    let s = v.as_str()?.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(num) = s.parse::<i64>() {
        return Some(if num.abs() < 10_000_000_000 {
            num * 1000
        } else {
            num
        });
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y/%m/%d %H:%M:%S") {
        return Some(ndt.and_utc().timestamp_millis());
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(ndt.and_utc().timestamp_millis());
    }
    None
}

/// Cached balance and quota response fetched from a provider's official public API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderBalanceSnapshot {
    pub provider_id: String,
    pub currency: String,
    pub total_balance_micro: Option<i64>,
    pub granted_balance_micro: Option<i64>,
    pub topped_up_balance_micro: Option<i64>,
    pub daily_quota_used_micro: Option<i64>,
    pub daily_quota_limit_micro: Option<i64>,
    pub fetched_at_ms: i64,
}

/// Pay-as-you-go account or prepaid credit tracker persisted in SQLite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaygAccount {
    pub account_id: String,
    pub provider_id: String,
    pub account_name: String,
    pub enabled: bool,
    /// Optional user-recorded credit/prepaid balance in micro-USD when no official balance API exists.
    pub manual_balance_micro_usd: Option<i64>,
    /// Epoch ms when `manual_balance_micro_usd` was recorded; local usage after this time deducts from it.
    pub balance_anchor_ms: Option<i64>,
    /// Optional monthly pay-as-you-go spend budget alert threshold in micro-USD.
    pub monthly_budget_micro_usd: Option<i64>,
    #[serde(default)]
    pub is_auto: bool,
    pub updated_at_ms: i64,
}

/// Live evaluated pay-as-you-go balance, credits, and local spend summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaygAccountStatus {
    pub account: PaygAccount,
    pub official_balance: Option<ProviderBalanceSnapshot>,
    pub remaining_manual_balance_micro_usd: Option<i64>,
    pub today_cost_micro_usd: i64,
    pub month_cost_micro_usd: i64,
    pub budget_used_percent: Option<f64>,
}

/// Supported log/payload format presets for user-configured custom providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomProtocolKind {
    OpenaiJsonl,
    AnthropicJsonl,
    DeclarativeJsonl,
}

impl CustomProtocolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenaiJsonl => "openai_jsonl",
            Self::AnthropicJsonl => "anthropic_jsonl",
            Self::DeclarativeJsonl => "declarative_jsonl",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "anthropic_jsonl" => Self::AnthropicJsonl,
            "declarative_jsonl" => Self::DeclarativeJsonl,
            _ => Self::OpenaiJsonl,
        }
    }
}

/// JSON pointer path configuration for declarative JSONL extraction.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclarativeMappingSpec {
    #[serde(default)]
    pub provider_id_override: Option<String>,
    #[serde(default)]
    pub file_extensions: Vec<String>,
    #[serde(default)]
    pub timestamp_path: Option<String>,
    #[serde(default)]
    pub model_path: Option<String>,
    #[serde(default)]
    pub session_id_path: Option<String>,
    #[serde(default)]
    pub request_id_path: Option<String>,
    #[serde(default)]
    pub input_tokens_path: Option<String>,
    #[serde(default)]
    pub output_tokens_path: Option<String>,
    #[serde(default)]
    pub cache_read_tokens_path: Option<String>,
    #[serde(default)]
    pub cache_write_tokens_path: Option<String>,
    #[serde(default)]
    pub reasoning_tokens_path: Option<String>,
    #[serde(default)]
    pub duration_ms_path: Option<String>,
    #[serde(default)]
    pub ttft_ms_path: Option<String>,
}

/// User-defined compatible / declarative local log ingestion rule stored in SQLite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomProviderRule {
    pub rule_id: String,
    pub display_name: String,
    pub enabled: bool,
    pub protocol_kind: CustomProtocolKind,
    pub log_roots: Vec<String>,
    pub mapping_spec: DeclarativeMappingSpec,
    pub updated_at_ms: i64,
}

/// Local dry-run validation result for testing one sanitized JSONL line against a custom rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomRuleTestResult {
    pub matched: bool,
    pub provider_id: String,
    pub model_id: Option<String>,
    pub timestamp_ms: Option<i64>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    pub estimated_cost_micro_usd: i64,
    pub diagnostic: String,
}

/// Result summary returned after synchronizing remote USD->CNY rate, public model pricing,
/// and AI Subscription Catalog templates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogSyncResult {
    pub rate_updated: bool,
    pub usd_to_cny_rate: String,
    pub pricing_updated: bool,
    pub synced_models_count: usize,
    pub recomputed_events_count: usize,
    pub synced_at_ms: i64,
    pub rate_source: Option<String>,
    pub pricing_source: Option<String>,
    #[serde(default)]
    pub subscriptions_updated: bool,
    #[serde(default)]
    pub synced_plans_count: usize,
    #[serde(default)]
    pub subscription_source: Option<String>,
    pub warning: Option<String>,
}

/// Persisted display title for an observed session (`(provider_id, session_id)`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTitleRecord {
    pub session_key: String,
    pub provider_id: String,
    pub title: String,
    pub source: String,
    pub updated_at_ms: i64,
}

const BOILERPLATE_BLOCK_TAGS: &[&str] = &[
    "environment_context",
    "system-reminder",
    "user_instructions",
    "user-instructions",
    "permissions instructions",
    "developer_instructions",
    "developer-instructions",
    "system_instruction",
    "system-instruction",
    "instructions",
    "agents_md",
    "agents-md",
    "local-command-caveat",
    "turn_aborted",
    "ide_opened_file",
    "ide_selection",
    "collaboration_mode",
    "additional_metadata",
    "additional-metadata",
    "user_settings_change",
    "context_summary",
    "compacted_context",
];

fn is_compaction_or_handoff_preamble(lower: &str) -> bool {
    lower.starts_with("the following is the codex agent history")
        || lower.starts_with("the following is a summary of")
        || lower.starts_with("another language model started to solve this problem")
        || lower.starts_with("[plan review")
        || lower.starts_with("context summary:")
        || lower.starts_with("conversation summary:")
        || lower.starts_with("here is a summary of the conversation")
        || lower.starts_with("summary of the conversation so far")
        || lower.starts_with("compact summary")
}

fn strip_boilerplate_blocks(text: &str) -> String {
    let mut owned = text.to_string();
    let lower_init = owned.to_ascii_lowercase();
    if let Some(req_start) = lower_init.find("<user_request>") {
        let after_tag = req_start + "<user_request>".len();
        let rest_lower = &lower_init[after_tag..];
        let extracted = if let Some(req_end_rel) = rest_lower.find("</user_request>") {
            owned[after_tag..after_tag + req_end_rel].to_string()
        } else {
            owned[after_tag..].to_string()
        };
        if !extracted.trim().is_empty() {
            owned = extracted;
        }
    }
    for tag in BOILERPLATE_BLOCK_TAGS {
        let open_prefix = format!("<{tag}");
        let close_tag = format!("</{tag}>");
        loop {
            let lower = owned.to_ascii_lowercase();
            let Some(start) = lower.find(&open_prefix) else {
                break;
            };
            let rest = &lower[start..];
            if let Some(end_rel) = rest.find(&close_tag) {
                let end = start + end_rel + close_tag.len();
                owned.replace_range(start..end, " ");
            } else {
                // Multi-line boilerplate block without a closing tag in the same chunk: drop remainder
                owned.replace_range(start..owned.len(), " ");
                break;
            }
        }
    }
    // Unwrap <command-message>...</command-message>, <command-name>...</command-name>, <user_request>
    owned
        .trim()
        .replace("<user_request>", "")
        .replace("</user_request>", " ")
        .replace("<USER_REQUEST>", "")
        .replace("</USER_REQUEST>", " ")
        .replace("<command-message>", "")
        .replace("</command-message>", " ")
        .replace("<command-name>", "")
        .replace("</command-name>", " ")
        .replace("<command-args>", "")
        .replace("</command-args>", " ")
}

fn looks_like_uuid(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() == 36
        && bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
    {
        return bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 8 || i == 13 || i == 18 || i == 23 || b.is_ascii_hexdigit());
    }
    bytes.len() == 32 && bytes.iter().all(|b| b.is_ascii_hexdigit())
}

/// Sanitize raw explicit session titles or initial user prompts into concise, safe display names.
pub fn sanitize_session_title(raw: &str) -> Option<String> {
    let stripped = strip_boilerplate_blocks(raw);
    let mut candidate_lines: Vec<&str> = Vec::new();
    let mut skipping_agents_block = false;
    for line in stripped.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("# agents.md")
            || lower.starts_with("agents.md instructions")
            || lower.starts_with("<instructions")
            || trimmed.starts_with("# 工作偏好")
            || trimmed.starts_with("这些是跨项目的工作偏好")
            || trimmed.contains("这些是跨项目的工作偏好")
        {
            skipping_agents_block = true;
            continue;
        }
        if lower == "</instructions>" {
            skipping_agents_block = false;
            continue;
        }
        if skipping_agents_block {
            continue;
        }
        if lower.starts_with("<system")
            || lower.starts_with("</system")
            || lower.starts_with("[request interrupted")
            || lower.starts_with("[tool")
        {
            continue;
        }
        candidate_lines.push(trimmed);
    }

    let joined = candidate_lines.join(" ");
    let collapsed = joined
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_start_matches('#')
        .trim()
        .to_string();

    let collapsed_lower = collapsed.to_ascii_lowercase();
    if collapsed.chars().count() < 2
        || looks_like_uuid(&collapsed)
        || is_compaction_or_handoff_preamble(&collapsed_lower)
    {
        return None;
    }

    let max_chars = 64;
    if collapsed.chars().count() <= max_chars {
        Some(collapsed)
    } else {
        let mut out: String = collapsed.chars().take(max_chars - 1).collect();
        out = out.trim_end().to_string();
        out.push('…');
        Some(out)
    }
}

#[cfg(test)]
mod domain_tps_and_title_tests {
    use super::{
        compute_ewma_tps, compute_iqr_upper_fence, compute_tps, is_valid_tps_sample,
        sanitize_session_title, speed_basis_for_provider, SpeedBasis,
    };

    #[test]
    fn speed_basis_marks_decode_collectors_and_serializes_snake_case() {
        assert_eq!(speed_basis_for_provider("claude_code"), SpeedBasis::Decode);
        assert_eq!(speed_basis_for_provider("codex"), SpeedBasis::Decode);
        assert_eq!(speed_basis_for_provider("kimi_code"), SpeedBasis::Decode);
        assert_eq!(speed_basis_for_provider("antigravity"), SpeedBasis::EndToEnd);
        assert_eq!(speed_basis_for_provider("custom_vendor"), SpeedBasis::EndToEnd);
        assert_eq!(
            serde_json::to_string(&SpeedBasis::EndToEnd).expect("json"),
            "\"end_to_end\""
        );
        assert_eq!(
            serde_json::from_str::<SpeedBasis>("\"decode\"").expect("parse"),
            SpeedBasis::Decode
        );
    }

    #[test]
    fn compute_tps_and_statistical_iqr_ewma_denoise_spikes() {
        assert_eq!(compute_tps(11, Some(1000)), None);
        assert_eq!(compute_tps(100, Some(499)), None);
        assert_eq!(compute_tps(250, Some(500)), None); // 500 tok/s > 450
        let valid = compute_tps(120, Some(1000)).expect("120 tok/s should be valid");
        assert!((valid - 120.0).abs() < 1e-6);
        assert!(!is_valid_tps_sample(10, Some(1000), Some(10.0)));
        assert!(!is_valid_tps_sample(200, Some(1000), Some(520.0)));
        assert!(is_valid_tps_sample(24, Some(500), Some(48.0)));

        // Tukey IQR fence rejects a 380 tok/s buffered flush spike when steady-state is ~50-65 tok/s
        let mut sorted: Vec<f64> = vec![48.0, 52.0, 55.0, 60.0, 64.0, 380.0];
        sorted.sort_by(|a, b| a.total_cmp(b));
        let fence = compute_iqr_upper_fence(&sorted).expect("fence");
        assert!(fence < 180.0);
        let smoothed = compute_ewma_tps(&[48.0, 52.0, 55.0, 60.0, 64.0, 380.0], 0.45)
            .expect("smoothed ewma");
        assert!(smoothed > 50.0 && smoothed < 65.0);
    }

    #[test]
    fn sanitize_session_title_cleans_boilerplate_and_uuids() {
        assert_eq!(
            sanitize_session_title("<environment_context><cwd>/tmp</cwd></environment_context>\nFix speed monitor spikes"),
            Some("Fix speed monitor spikes".to_string())
        );
        assert_eq!(
            sanitize_session_title("<system-reminder>Internal note</system-reminder>"),
            None
        );
        assert_eq!(
            sanitize_session_title(
                "# AGENTS.md instructions for c:\\Projects\\TokenVane\n\n<INSTRUCTIONS>\n# 工作偏好\n\n这些是跨项目的工作偏好，默认长期有效。\n</INSTRUCTIONS>"
            ),
            None
        );
        assert_eq!(
            sanitize_session_title("# 工作偏好\n\n这些是跨项目的工作偏好，默认长期有效。"),
            None
        );
        assert_eq!(
            sanitize_session_title("550e8400-e29b-41d4-a716-446655440000"),
            None
        );
        assert_eq!(
            sanitize_session_title("<command-name>/plan</command-name> Upgrade Tauri to 0.2.2"),
            Some("/plan Upgrade Tauri to 0.2.2".to_string())
        );
        assert_eq!(
            sanitize_session_title(
                "The following is the Codex agent history. Some context was compacted..."
            ),
            None
        );
        assert_eq!(
            sanitize_session_title("Another language model started to solve this problem..."),
            None
        );
        assert_eq!(
            sanitize_session_title("[plan review] please inspect current status"),
            None
        );
        assert_eq!(
            sanitize_session_title(
                "<USER_REQUEST>Refactor pay-as-you-go UI</USER_REQUEST>\n<USER_SETTINGS_CHANGE>Model changed</USER_SETTINGS_CHANGE>"
            ),
            Some("Refactor pay-as-you-go UI".to_string())
        );
    }
}
