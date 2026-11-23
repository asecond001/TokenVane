use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

use chrono::DateTime;
use serde_json::Value;
use tokvane_core::domain::{
    compute_event_id, provider_ids, sanitize_session_title, NormalizedUsageEvent,
    ObservedRateLimitWindow, ProviderBalanceSnapshot,
};

use crate::error::{CollectorError, CollectorResult};
use crate::providers::{
    collapse_duplicate_events_keep_last, compute_tps, get_i64, now_ms, parse_timestamp_ms,
    user_home_dir, FileParseContext, ParsedLineResult, ProviderLogParser,
};
use crate::watcher::open_shared_readonly;

const DEFAULT_ANTIGRAVITY_MODEL: &str = "antigravity-planner";
const SETTINGS_CHANGE_OPEN: &str = "<USER_SETTINGS_CHANGE>";
const SETTINGS_CHANGE_CLOSE: &str = "</USER_SETTINGS_CHANGE>";

/// Remove wrapper tags whose body may quote `<USER_SETTINGS_CHANGE>` text (for example,
/// `<USER_REQUEST>` or `<CONTEXT_SUMMARY>`), so only real settings blocks are parsed.
fn strip_tag_blocks(content: &str, open_tag: &str, close_tag: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut cursor = 0usize;
    while let Some(rel_open) = content[cursor..].find(open_tag) {
        let open_idx = cursor + rel_open;
        out.push_str(&content[cursor..open_idx]);
        let after_open = open_idx + open_tag.len();
        if let Some(rel_close) = content[after_open..].find(close_tag) {
            cursor = after_open + rel_close + close_tag.len();
        } else {
            return out;
        }
    }
    out.push_str(&content[cursor..]);
    out
}

/// Extract the selected model name from Antigravity's `<USER_SETTINGS_CHANGE>` block
/// inside non-`SYSTEM` `USER_INPUT` content, e.g.:
/// `The user changed setting \`Model Selection\` from None to Claude Sonnet 5.5 (Medium). ...`
fn extract_model_selection_from_content(content: &str) -> Option<String> {
    if !content.contains(SETTINGS_CHANGE_OPEN) {
        return None;
    }
    let scrubbed = strip_tag_blocks(content, "<USER_REQUEST>", "</USER_REQUEST>");
    let scrubbed = strip_tag_blocks(&scrubbed, "<CONTEXT_SUMMARY>", "</CONTEXT_SUMMARY>");

    let mut latest_model: Option<String> = None;
    let mut scan = scrubbed.as_str();
    while let Some(rel_start) = scan.find(SETTINGS_CHANGE_OPEN) {
        let rest = &scan[rel_start + SETTINGS_CHANGE_OPEN.len()..];
        let (block, next_scan) = match rest.find(SETTINGS_CHANGE_CLOSE) {
            Some(end) => (&rest[..end], &rest[end + SETTINGS_CHANGE_CLOSE.len()..]),
            None => (rest, ""),
        };
        scan = next_scan;

        // Require Antigravity's exact `changed setting` phrasing before `Model Selection`
        // so copied prose or test assertions inside USER_INPUT cannot hijack model attribution.
        let Some(changed_pos) = block.find("changed setting") else {
            continue;
        };
        let after_changed = &block[changed_pos + "changed setting".len()..];
        let Some(setting_pos) = after_changed.find("Model Selection") else {
            continue;
        };
        let after_setting = &after_changed[setting_pos + "Model Selection".len()..];
        let Some(to_pos) = after_setting.find(" to ") else {
            continue;
        };
        let after_to = after_setting[to_pos + " to ".len()..].trim_start();

        // Stop at newline or sentence boundary (". " or trailing "."), preserving dots in version numbers like "5.5".
        let line_part = after_to.lines().next().unwrap_or(after_to).trim();
        let before_sentence_end = line_part
            .split_once(". ")
            .map(|(head, _)| head)
            .unwrap_or(line_part)
            .trim_end_matches('.')
            .trim();

        if !before_sentence_end.is_empty() && !before_sentence_end.eq_ignore_ascii_case("none") {
            latest_model = Some(before_sentence_end.to_string());
        }
    }

    latest_model
}

/// Extract an explicit subscription / tier hint from settings or JSON tier metadata (NEVER from model names like Gemini Pro).
fn normalize_explicit_antigravity_tier(raw: &str) -> Option<String> {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.is_empty() || lower == "none" || lower == "unknown" {
        return None;
    }
    if lower.contains("ultra") && (lower.contains("5x") || lower.contains("99")) {
        Some("ultra_5x".to_string())
    } else if lower.contains("ultra") {
        Some("ultra".to_string())
    } else if lower.contains("enterprise") || lower.contains("business") {
        Some("enterprise".to_string())
    } else if lower.contains("plus") {
        Some("plus".to_string())
    } else if lower.contains("free") || lower.contains("standard-tier") {
        Some("free".to_string())
    } else if lower == "pro"
        || lower.contains("g1-pro")
        || lower.contains("google ai pro")
        || lower.contains("google_ai_pro")
    {
        Some("pro".to_string())
    } else {
        None
    }
}

/// Infer Ultra/Enterprise tier ONLY from Ultra-exclusive model identifiers (e.g. Carbon backend);
/// never infer Pro from "Gemini Pro" models because Pro models are shared by Ultra & Pro plans.
fn infer_antigravity_tier_from_model_name(model_name: &str) -> Option<String> {
    let lower = model_name.trim().to_ascii_lowercase();
    if lower.is_empty() || lower == "none" {
        return None;
    }
    if lower.contains("ultra") && (lower.contains("5x") || lower.contains("99")) {
        Some("ultra_5x".to_string())
    } else if lower.contains("ultra") || lower.contains("carbon") || lower.contains("g1-ultra") {
        Some("ultra".to_string())
    } else if lower.contains("enterprise") {
        Some("enterprise".to_string())
    } else {
        None
    }
}

fn parse_settings_json_for_plan_hint(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let mut file = open_shared_readonly(path).ok()?;
    let mut raw = String::new();
    file.read_to_string(&mut raw).ok()?;
    let val = serde_json::from_str::<Value>(&raw).ok()?;

    for key in [
        "subscriptionTier",
        "subscription_tier",
        "planTier",
        "plan_tier",
        "quotaTier",
        "quota_tier",
        "userTier",
        "user_tier",
        "tier",
        "plan",
        "membership",
    ] {
        if let Some(s) = val.get(key).and_then(|v| v.as_str()) {
            if let Some(hint) = normalize_explicit_antigravity_tier(s) {
                return Some(hint);
            }
        }
    }

    for nested_val in [
        val.pointer("/agent/preferredModelTier"),
        val.pointer("/agent/preferred_model_tier"),
        val.pointer("/models/orchestratorBackend"),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(s) = nested_val.as_str() {
            if let Some(hint) = normalize_explicit_antigravity_tier(s) {
                return Some(hint);
            }
        }
    }

    None
}

/// Read-only inspection of documented public Antigravity `settings.json` files in `~/.gemini/`.
pub fn detect_explicit_tier_from_local_settings() -> Option<String> {
    let home = user_home_dir()?;
    let gemini_dir = home.join(".gemini");
    for candidate in [
        gemini_dir.join("antigravity-cli").join("settings.json"),
        gemini_dir.join("antigravity").join("settings.json"),
        gemini_dir.join("settings.json"),
    ] {
        if let Some(hint) = parse_settings_json_for_plan_hint(&candidate) {
            return Some(hint);
        }
    }
    None
}

/// Inspect documented public Antigravity `settings.json` locations read-only for subscription tier hints.
fn resolve_antigravity_plan_hint(transcript_path: &Path) -> Option<String> {
    // Check ancestor directories (e.g., <root>/settings.json or ~/.gemini/antigravity-cli/settings.json)
    let mut ancestor = transcript_path.parent();
    for _ in 0..5 {
        let Some(dir) = ancestor else {
            break;
        };
        if let Some(hint) = parse_settings_json_for_plan_hint(&dir.join("settings.json")) {
            return Some(hint);
        }
        ancestor = dir.parent();
    }

    detect_explicit_tier_from_local_settings()
}

/// Locate the official Antigravity CLI executable (`agy` / `agy.exe`) without triggering GUI wrappers.
pub fn resolve_agy_cli_binary() -> Option<PathBuf> {
    if let Some(override_bin) = std::env::var_os("TOKENVANE_AGY_BIN")
        .or_else(|| std::env::var_os("TOKENTRACKER_AGY_BIN"))
    {
        let path = PathBuf::from(override_bin);
        if path.is_file() {
            return Some(path);
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            let base = PathBuf::from(local_app_data);
            candidates.push(base.join("agy").join("bin").join("agy.exe"));
            candidates.push(base.join("agy").join("bin").join("agy.cmd"));
            candidates.push(base.join("Programs").join("Antigravity").join("bin").join("agy.exe"));
            candidates.push(
                base.join("Programs")
                    .join("Antigravity")
                    .join("resources")
                    .join("app")
                    .join("bin")
                    .join("agy.cmd"),
            );
        }
        if let Some(path_env) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_env) {
                candidates.push(dir.join("agy.exe"));
                candidates.push(dir.join("agy.cmd"));
            }
        }
    }

    #[cfg(not(windows))]
    {
        if let Some(home) = user_home_dir() {
            candidates.push(home.join(".local").join("bin").join("agy"));
        }
        candidates.push(PathBuf::from("/opt/homebrew/bin/agy"));
        candidates.push(PathBuf::from("/usr/local/bin/agy"));
        candidates.push(PathBuf::from(
            "/Applications/Antigravity.app/Contents/Resources/app/bin/agy",
        ));
        if let Some(path_env) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_env) {
                candidates.push(dir.join("agy"));
            }
        }
    }

    candidates.into_iter().find(|p| p.is_file())
}

fn json_float(val: Option<&Value>) -> Option<f64> {
    let v = val?;
    let parsed = if let Some(n) = v.as_f64() {
        Some(n)
    } else if let Some(i) = v.as_i64() {
        Some(i as f64)
    } else if let Some(s) = v.as_str() {
        s.trim().trim_end_matches('%').parse::<f64>().ok()
    } else {
        None
    }?;
    parsed.is_finite().then_some(parsed)
}

fn parse_cli_bucket_window_minutes(bucket: &Value, bucket_id: &str) -> Option<i64> {
    let raw_window = bucket
        .get("window")
        .or_else(|| bucket.get("windowType"))
        .or_else(|| bucket.get("name"))
        .and_then(|v| v.as_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if raw_window == "5h" || raw_window.contains("five hour") || raw_window.contains("5-hour") {
        return Some(300);
    }
    if raw_window == "weekly" || raw_window == "7d" || raw_window.contains("week") {
        return Some(10_080);
    }
    if raw_window == "daily" || raw_window == "24h" || raw_window.contains("day") {
        return Some(1_440);
    }
    if raw_window == "monthly" || raw_window == "30d" || raw_window.contains("month") {
        return Some(43_200);
    }
    let lower_id = bucket_id.to_ascii_lowercase();
    if lower_id.contains("5h") {
        Some(300)
    } else if lower_id.contains("weekly") || lower_id.contains("7d") {
        Some(10_080)
    } else if lower_id.contains("daily") || lower_id.contains("24h") {
        Some(1_440)
    } else if lower_id.contains("monthly") || lower_id.contains("30d") {
        Some(43_200)
    } else {
        None
    }
}

fn parse_cli_reset_time_ms(bucket: &Value) -> Option<i64> {
    let raw = bucket
        .get("reset_time")
        .or_else(|| bucket.get("resetTime"))
        .or_else(|| bucket.get("resets_at"))
        .or_else(|| bucket.get("resetsAt"))?;
    if let Some(s) = raw.as_str() {
        if let Ok(dt) = DateTime::parse_from_rfc3339(s.trim()) {
            return Some(dt.timestamp_millis());
        }
    }
    parse_timestamp_ms(Some(raw))
}

fn push_cli_bucket_window(
    out: &mut Vec<ObservedRateLimitWindow>,
    bucket: &Value,
    group_prefix: Option<&str>,
    now_ms: i64,
    plan_hint: &Option<String>,
) {
    let raw_id = bucket
        .get("id")
        .or_else(|| bucket.get("bucketId"))
        .or_else(|| bucket.get("bucket_id"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let window_name = match (raw_id, group_prefix) {
        (Some(id), _) => id.to_string(),
        (None, Some(prefix)) => {
            let suffix = bucket
                .get("window")
                .and_then(|v| v.as_str())
                .unwrap_or("quota");
            format!("{prefix}-{suffix}")
        }
        (None, None) => return,
    };

    let used_percent = if let Some(rem_frac) = json_float(
        bucket
            .get("remaining_fraction")
            .or_else(|| bucket.get("remainingFraction")),
    ) {
        ((1.0 - rem_frac.clamp(0.0, 1.0)) * 100.0).clamp(0.0, 100.0)
    } else if let Some(used_frac) = json_float(
        bucket
            .get("used_fraction")
            .or_else(|| bucket.get("usedFraction")),
    ) {
        (used_frac.clamp(0.0, 1.0) * 100.0).clamp(0.0, 100.0)
    } else if let Some(used_pct) = json_float(
        bucket
            .get("used_percent")
            .or_else(|| bucket.get("usedPercent")),
    ) {
        used_pct.clamp(0.0, 100.0)
    } else if let Some(rem_pct) = json_float(
        bucket
            .get("remaining_percent")
            .or_else(|| bucket.get("remainingPercent")),
    ) {
        (100.0 - rem_pct.clamp(0.0, 100.0)).clamp(0.0, 100.0)
    } else {
        return;
    };

    let window_minutes = parse_cli_bucket_window_minutes(bucket, &window_name);
    let resets_at_ms = parse_cli_reset_time_ms(bucket);

    out.push(ObservedRateLimitWindow {
        provider_id: provider_ids::ANTIGRAVITY.to_string(),
        window_name,
        used_percent,
        window_minutes,
        resets_at_ms,
        observed_at_ms: now_ms,
        source: Some("official_cli".to_string()),
        plan_hint: plan_hint.clone(),
    });
}

fn infer_cli_group_prefix(group_name: Option<&str>) -> Option<&'static str> {
    let lower = group_name?.to_ascii_lowercase();
    if lower.contains("gemini") {
        Some("gemini")
    } else if lower.contains("claude") || lower.contains("gpt") {
        Some("3p")
    } else {
        None
    }
}

/// Parse the JSON output emitted by `agy -p "/usage" --output-format json`.
/// Never infer Ultra tier from `groups[].description` because `Gemini 4 Carbon` appears in every user's static group listing.
pub fn parse_antigravity_cli_usage_output(
    raw: &str,
    now_ms: i64,
    fallback_plan_hint: Option<String>,
) -> CollectorResult<(ProviderBalanceSnapshot, Vec<ObservedRateLimitWindow>)> {
    let start = raw.find('{').ok_or_else(|| {
        CollectorError::Compliance(
            "agy CLI /usage 待返回有效 JSON（请确认已在本机登录 Antigravity CLI）".to_string(),
        )
    })?;
    let end = raw.rfind('}').ok_or_else(|| {
        CollectorError::Compliance("agy CLI /usage JSON 输出不完整".to_string())
    })?;
    if end < start {
        return Err(CollectorError::Compliance(
            "agy CLI /usage JSON 边界无效".to_string(),
        ));
    }

    let root: Value = serde_json::from_str(&raw[start..=end]).map_err(|e| {
        CollectorError::Compliance(format!("解析 agy CLI /usage JSON 失败: {e}"))
    })?;

    if let Some(status) = root.get("status").and_then(|v| v.as_str()) {
        if !status.eq_ignore_ascii_case("SUCCESS") && !status.eq_ignore_ascii_case("OK") {
            let detail = root
                .pointer("/error/message")
                .or_else(|| root.get("error"))
                .or_else(|| root.get("message"))
                .or_else(|| root.get("response"))
                .and_then(|v| v.as_str())
                .unwrap_or("CLI 请求未成功");
            return Err(CollectorError::Compliance(format!(
                "agy CLI /usage 返回状态 {status}: {detail}"
            )));
        }
    }

    let data = root
        .pointer("/command/data")
        .or_else(|| root.get("data"))
        .unwrap_or(&root);

    let plan_hint = [
        root.get("tier"),
        root.get("plan_tier"),
        root.get("planTier"),
        root.get("subscription_tier"),
        root.get("subscriptionTier"),
        root.get("user_tier"),
        root.get("userTier"),
        data.get("tier"),
        data.get("plan_tier"),
        data.get("planTier"),
        data.get("subscription_tier"),
        data.get("subscriptionTier"),
        data.get("user_tier"),
        data.get("userTier"),
    ]
    .into_iter()
    .flatten()
    .filter_map(|v| v.as_str())
    .find_map(normalize_explicit_antigravity_tier)
    .or(fallback_plan_hint);

    let mut windows = Vec::new();
    if let Some(groups) = data.get("groups").and_then(|v| v.as_array()) {
        for group in groups {
            let group_prefix =
                infer_cli_group_prefix(group.get("name").and_then(|v| v.as_str()));
            if let Some(buckets) = group.get("buckets").and_then(|v| v.as_array()) {
                for bucket in buckets {
                    push_cli_bucket_window(
                        &mut windows,
                        bucket,
                        group_prefix,
                        now_ms,
                        &plan_hint,
                    );
                }
            }
        }
    }
    if windows.is_empty() {
        if let Some(buckets) = data.get("buckets").and_then(|v| v.as_array()) {
            for bucket in buckets {
                push_cli_bucket_window(&mut windows, bucket, None, now_ms, &plan_hint);
            }
        }
    }

    if windows.is_empty() {
        return Err(CollectorError::Compliance(
            "请先在终端运行 agy 完成账号登录以获取配额读数".to_string(),
        ));
    }

    let snapshot = ProviderBalanceSnapshot {
        provider_id: provider_ids::ANTIGRAVITY.to_string(),
        currency: "QUOTA".to_string(),
        total_balance_micro: None,
        granted_balance_micro: None,
        topped_up_balance_micro: None,
        daily_quota_used_micro: None,
        daily_quota_limit_micro: None,
        fetched_at_ms: now_ms,
    };

    Ok((snapshot, windows))
}

// ---------------------------------------------------------------------------------------------
// `agy` probe guard
//
// When `agy -p /usage` cannot refresh its Google OAuth token silently (offline, DNS failure,
// broken proxy, unauthenticated, or `invalid_grant`), the CLI falls back to an *interactive*
// OAuth flow and opens the system browser on the Google sign-in page. Probing repeatedly in
// that state pops a browser window (stealing focus from full-screen apps) and spawns `conhost.exe`
// + ephemeral loopback ports on every poll. Multiple guards prevent that:
//   1. `NO_BROWSER=true`, `CI=true`, `TERM=dumb`, `BROWSER` removed + isolated probe cwd;
//   2. an immediate auth-pause state when any auth/interactive/timeout failure occurs, reset
//      only by an explicit user sync or CLI login;
//   3. a cheap reachability check of Google's OAuth host before every spawn; and
//   4. exponential backoff (30 min doubling, 6 h cap) after other transient failures.
// ---------------------------------------------------------------------------------------------

/// Public OAuth host the `agy` CLI must reach to refresh its token silently.
const AGY_AUTH_REACHABILITY_URL: &str = "https://oauth2.googleapis.com/";
const AGY_BACKOFF_BASE: Duration = Duration::from_secs(30 * 60);
const AGY_BACKOFF_MAX: Duration = Duration::from_secs(6 * 3_600);

#[derive(Debug, Default)]
struct AgyProbeBackoff {
    auth_paused: bool,
    consecutive_failures: u32,
    retry_after: Option<std::time::Instant>,
}

static AGY_BACKOFF: Mutex<AgyProbeBackoff> = Mutex::new(AgyProbeBackoff {
    auth_paused: false,
    consecutive_failures: 0,
    retry_after: None,
});

/// Dedicated private working directory for `agy` probes so project-level files cannot affect it.
fn agy_probe_dir() -> CollectorResult<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join("TokenVane"))
        .or_else(|| user_home_dir().map(|h| h.join(".tokenvane")))
        .ok_or_else(|| CollectorError::Compliance("无法定位 TokenVane 本地数据目录".to_string()))?;
    let dir = base.join("agy-probe");
    std::fs::create_dir_all(&dir)
        .map_err(|e| CollectorError::Compliance(format!("创建 agy CLI 探测目录失败: {e}")))?;
    Ok(dir)
}

/// Detect auth, interactive login, OAuth, or timeout errors that require manual sign-in rather
/// than automated background retries.
pub(crate) fn is_agy_auth_or_interactive_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("not signed in")
        || lower.contains("not logged in")
        || lower.contains("unauthenticated")
        || lower.contains("unauthorized")
        || lower.contains("invalid_grant")
        || lower.contains("oauth")
        || lower.contains("accounts.google.com")
        || msg.contains("完成账号登录")
        || msg.contains("请先在终端运行 agy")
        || msg.contains("登录")
        || msg.contains("超时")
}

/// Backoff applied after `failures` consecutive failed `agy` probes (30 min doubling, 6 h cap).
pub(crate) fn agy_backoff_delay(failures: u32) -> Duration {
    if failures == 0 {
        return Duration::ZERO;
    }
    let factor = 1u32
        .checked_shl(failures.saturating_sub(1).min(16))
        .unwrap_or(u32::MAX);
    AGY_BACKOFF_BASE
        .checked_mul(factor)
        .map_or(AGY_BACKOFF_MAX, |d| d.min(AGY_BACKOFF_MAX))
}

/// Clear the failure backoff and auth-pause state so the next probe runs immediately (explicit user sync/login).
pub fn reset_agy_probe_backoff() {
    if let Ok(mut guard) = AGY_BACKOFF.lock() {
        *guard = AgyProbeBackoff::default();
    }
}

fn agy_auth_paused() -> bool {
    AGY_BACKOFF.lock().map(|g| g.auth_paused).unwrap_or(false)
}

fn agy_backoff_remaining() -> Option<Duration> {
    let guard = AGY_BACKOFF.lock().ok()?;
    let until = guard.retry_after?;
    until.checked_duration_since(std::time::Instant::now())
}

fn record_agy_probe_result(err_msg: Option<&str>) {
    if let Ok(mut guard) = AGY_BACKOFF.lock() {
        match err_msg {
            None => {
                *guard = AgyProbeBackoff::default();
            }
            Some(msg) if is_agy_auth_or_interactive_error(msg) => {
                guard.auth_paused = true;
                guard.consecutive_failures = guard.consecutive_failures.saturating_add(1);
                guard.retry_after = None;
            }
            Some(_) => {
                guard.consecutive_failures = guard.consecutive_failures.saturating_add(1);
                guard.retry_after =
                    Some(std::time::Instant::now() + agy_backoff_delay(guard.consecutive_failures));
            }
        }
    }
}

/// Whether Google's OAuth host answers at all (any HTTP status counts). No credentials are sent.
async fn agy_auth_host_reachable() -> bool {
    let Ok(client) = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(6))
        .user_agent(concat!("TokenVane/", env!("CARGO_PKG_VERSION")))
        .build()
    else {
        return false;
    };
    client.get(AGY_AUTH_REACHABILITY_URL).send().await.is_ok()
}

/// Non-interactively invoke the documented official Antigravity CLI (`agy -p "/usage" --output-format json`).
///
/// Guarded so a degraded network or unauthenticated CLI never makes `agy` open its interactive
/// browser sign-in: the probe is skipped while auth-paused, while a failure backoff is active, or
/// while Google's OAuth host is unreachable.
pub async fn collect_antigravity_cli_usage(
    now_ms: i64,
) -> CollectorResult<(ProviderBalanceSnapshot, Vec<ObservedRateLimitWindow>)> {
    let bin_path = resolve_agy_cli_binary().ok_or_else(|| {
        CollectorError::Compliance(
            "请先安装官方 agy CLI（默认路径 %LOCALAPPDATA%\\agy\\bin\\agy.exe）".to_string(),
        )
    })?;

    if agy_auth_paused() {
        return Err(CollectorError::Compliance(
            "agy CLI 未登录或授权待恢复，已暂停自动探测；请在「用量」手动登录并点击同步"
                .to_string(),
        ));
    }
    if let Some(remaining) = agy_backoff_remaining() {
        let minutes = remaining.as_secs().div_ceil(60).max(1);
        return Err(CollectorError::Compliance(format!(
            "agy 配额同步退避中，约 {minutes} 分钟后自动重试，可点击同步立即重试"
        )));
    }
    if !agy_auth_host_reachable().await {
        return Err(CollectorError::Compliance(
            "网络待恢复，agy 配额将在 Google 认证服务连通后自动同步".to_string(),
        ));
    }

    let result = collect_antigravity_cli_usage_from(bin_path, now_ms).await;
    match &result {
        Ok(_) => record_agy_probe_result(None),
        Err(err) => record_agy_probe_result(Some(&err.to_string())),
    }
    result
}

async fn collect_antigravity_cli_usage_from(
    bin_path: PathBuf,
    now_ms: i64,
) -> CollectorResult<(ProviderBalanceSnapshot, Vec<ObservedRateLimitWindow>)> {
    let raw_output = tokio::task::spawn_blocking(move || run_agy_cli_blocking(&bin_path))
        .await
        .map_err(|e| CollectorError::Compliance(format!("执行 agy CLI 后台任务异常: {e}")))??;

    let fallback_hint = detect_explicit_tier_from_local_settings();

    match parse_antigravity_cli_usage_output(&raw_output.stdout, now_ms, fallback_hint) {
        Ok(parsed) => Ok(parsed),
        Err(err) if !raw_output.success => {
            let stderr_snippet: String = raw_output
                .stderr
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .take(2)
                .collect::<Vec<_>>()
                .join(" | ")
                .chars()
                .take(180)
                .collect();
            if stderr_snippet.is_empty() {
                Err(err)
            } else {
                Err(CollectorError::Compliance(format!(
                    "agy CLI /usage 退出失败 ({}): {stderr_snippet}",
                    raw_output.status_desc
                )))
            }
        }
        Err(err) => Err(err),
    }
}

struct CliProcessOutput {
    success: bool,
    status_desc: String,
    stdout: String,
    stderr: String,
}

fn run_agy_cli_blocking(bin_path: &Path) -> CollectorResult<CliProcessOutput> {
    let cwd = agy_probe_dir()?;
    // Rust std >= 1.77 executes `.cmd` / `.bat` wrappers via `cmd.exe` with safe quoting.
    let mut cmd = std::process::Command::new(bin_path);
    cmd.args(["-p", "/usage", "--output-format", "json"])
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    crate::cli_process::apply_headless_no_browser_env(&mut cmd);
    crate::cli_process::configure_hidden_cli_command(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|e| CollectorError::Compliance(format!("启动 agy CLI 失败: {e}")))?;
    let job_guard = crate::cli_process::ChildProcessGuard::attach(&child);

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let (status, stdout_bytes, stderr_bytes) = std::thread::scope(|scope| {
        let stdout_handle = scope.spawn(move || -> Vec<u8> {
            let mut buf = Vec::new();
            if let Some(mut reader) = stdout_pipe {
                let _ = reader.read_to_end(&mut buf);
            }
            buf
        });
        let stderr_handle = scope.spawn(move || -> Vec<u8> {
            let mut buf = Vec::new();
            if let Some(mut reader) = stderr_pipe {
                let _ = reader.read_to_end(&mut buf);
            }
            buf
        });

        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        let status = loop {
            match child.try_wait() {
                Ok(Some(st)) => break Ok(st),
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        job_guard.terminate_tree();
                        let _ = child.kill();
                        let _ = child.wait();
                        break Err(CollectorError::Compliance(
                            "调用官方 agy CLI /usage 超时（>25s），请确认网络连通及 CLI 登录状态"
                                .to_string(),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(40));
                }
                Err(e) => {
                    job_guard.terminate_tree();
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(CollectorError::Compliance(format!(
                        "等待 agy CLI 退出失败: {e}"
                    )));
                }
            }
        };

        let out_b = stdout_handle.join().unwrap_or_default();
        let err_b = stderr_handle.join().unwrap_or_default();
        (status, out_b, err_b)
    });

    // Ensure any lingering sidecars (e.g. loopback listeners) are terminated once pipes are drained.
    drop(job_guard);

    let status = status?;
    Ok(CliProcessOutput {
        success: status.success(),
        status_desc: status.to_string(),
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
    })
}

fn extract_antigravity_rate_limits(
    val: &Value,
    observed_at_ms: i64,
    fallback_hint: Option<&str>,
) -> Vec<ObservedRateLimitWindow> {
    let explicit_hint = val
        .get("plan_tier")
        .or_else(|| val.get("subscription_tier"))
        .or_else(|| val.get("tier"))
        .or_else(|| val.get("plan"))
        .and_then(|v| v.as_str())
        .and_then(normalize_explicit_antigravity_tier)
        .or_else(|| fallback_hint.map(ToString::to_string));

    let mut out = Vec::new();
    let Some(rl_obj) = val
        .get("rate_limits")
        .or_else(|| val.get("rate_limit"))
        .or_else(|| val.get("quota"))
        .and_then(|v| v.as_object())
    else {
        return out;
    };

    let obj_hint = rl_obj
        .get("plan_type")
        .or_else(|| rl_obj.get("tier"))
        .or_else(|| rl_obj.get("plan"))
        .and_then(|v| v.as_str())
        .and_then(normalize_explicit_antigravity_tier)
        .or(explicit_hint);

    for (window_name, win_val) in rl_obj {
        let Some(win_obj) = win_val.as_object() else {
            continue;
        };
        let used_percent = win_obj
            .get("used_percent")
            .or_else(|| win_obj.get("usedPercent"))
            .and_then(|v| v.as_f64())
            .or_else(|| {
                let limit = win_obj.get("limit")?.as_f64()?;
                if limit <= 0.0 {
                    return None;
                }
                if let Some(used) = win_obj.get("used").and_then(|v| v.as_f64()) {
                    Some(((used / limit) * 100.0).clamp(0.0, 100.0))
                } else {
                    let rem = win_obj.get("remaining")?.as_f64()?;
                    Some((((limit - rem).max(0.0) / limit) * 100.0).clamp(0.0, 100.0))
                }
            });
        let Some(pct) = used_percent.filter(|p| p.is_finite()) else {
            continue;
        };
        let window_minutes = win_obj.get("window_minutes").and_then(|v| v.as_i64());
        let resets_at_ms = parse_timestamp_ms(
            win_obj
                .get("resets_at")
                .or_else(|| win_obj.get("reset_at"))
                .or_else(|| win_obj.get("resetTime")),
        )
        .or_else(|| {
            win_obj
                .get("resets_in_seconds")
                .and_then(|v| v.as_i64())
                .map(|secs| observed_at_ms + secs.max(0) * 1000)
        });
        out.push(ObservedRateLimitWindow {
            provider_id: provider_ids::ANTIGRAVITY.to_string(),
            window_name: window_name.clone(),
            used_percent: pct.clamp(0.0, 100.0),
            window_minutes,
            resets_at_ms,
            observed_at_ms,
            source: Some("local_log".to_string()),
            plan_hint: obj_hint.clone(),
        });
    }

    out
}

fn extract_antigravity_user_request(content: &str) -> Option<&str> {
    let extracted = if let Some(s) = content.find("<USER_REQUEST>") {
        let rest = &content[s + "<USER_REQUEST>".len()..];
        rest.split_once("</USER_REQUEST>")
            .map(|(head, _)| head)
            .unwrap_or(rest)
    } else if let Some(start_sc) = content.find(SETTINGS_CHANGE_OPEN) {
        if let Some(end_sc) = content[start_sc..].find(SETTINGS_CHANGE_CLOSE) {
            &content[start_sc + end_sc + SETTINGS_CHANGE_CLOSE.len()..]
        } else {
            content
        }
    } else {
        content
    };
    let trimmed = extracted.trim();
    if trimmed.is_empty() || trimmed.to_ascii_lowercase().starts_with("[plan review") {
        None
    } else {
        Some(trimmed)
    }
}

#[derive(Default)]
struct InitialAntigravityMetadata {
    active_model: Option<String>,
    initial_title: Option<(String, String, i64)>,
    model_tier_hint: Option<String>,
}

/// Scan an existing `transcript.jsonl` read-only from byte 0 so incremental tailing from any
/// offset knows the active model, initial first-message title, and Ultra/Carbon tier signal.
fn scan_initial_antigravity_metadata(path: &Path) -> InitialAntigravityMetadata {
    let Ok(file) = open_shared_readonly(path) else {
        return InitialAntigravityMetadata::default();
    };
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut meta = InitialAntigravityMetadata::default();

    while reader.read_line(&mut line).ok().is_some_and(|n| n > 0) {
        if let Ok(val) = serde_json::from_str::<Value>(line.trim()) {
            let ts_ms = parse_timestamp_ms(val.get("created_at").or_else(|| val.get("timestamp")))
                .unwrap_or(0);
            let source = val.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let step_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");

            if let Some(title) = val
                .get("title")
                .or_else(|| val.get("conversation_title"))
                .and_then(|v| v.as_str())
                .and_then(sanitize_session_title)
            {
                meta.initial_title = Some((title, "explicit".to_string(), ts_ms));
            } else if meta.initial_title.is_none()
                && !source.eq_ignore_ascii_case("SYSTEM")
                && (step_type.eq_ignore_ascii_case("USER_INPUT")
                    || source.eq_ignore_ascii_case("USER_EXPLICIT"))
            {
                if let Some(user_req) = val
                    .get("content")
                    .and_then(|v| v.as_str())
                    .and_then(extract_antigravity_user_request)
                    .and_then(sanitize_session_title)
                {
                    meta.initial_title = Some((user_req, "user_prompt".to_string(), ts_ms));
                }
            }

            if !source.eq_ignore_ascii_case("SYSTEM")
                && step_type.eq_ignore_ascii_case("USER_INPUT")
            {
                if let Some(content) = val.get("content").and_then(|v| v.as_str()) {
                    if content.contains(SETTINGS_CHANGE_OPEN) && content.contains("Model Selection")
                    {
                        if let Some(model) = extract_model_selection_from_content(content) {
                            if let Some(h) = infer_antigravity_tier_from_model_name(&model) {
                                meta.model_tier_hint = Some(h);
                            }
                            meta.active_model = Some(model);
                        }
                    }
                }
            }

            if meta.model_tier_hint.is_none() {
                if let Some(explicit_m) = val
                    .get("model")
                    .or_else(|| val.get("model_id"))
                    .and_then(|v| v.as_str())
                {
                    meta.model_tier_hint = infer_antigravity_tier_from_model_name(explicit_m);
                }
            }
        }
        line.clear();
    }

    meta
}

#[derive(Debug, Default, Clone)]
pub struct AntigravityParser;

impl AntigravityParser {
    pub fn new() -> Self {
        Self
    }
}

impl ProviderLogParser for AntigravityParser {
    fn provider_id(&self) -> &str {
        provider_ids::ANTIGRAVITY
    }

    fn default_roots(&self) -> Vec<PathBuf> {
        let Some(home) = user_home_dir() else {
            return Vec::new();
        };
        vec![home.join(".gemini").join("antigravity").join("brain")]
    }

    fn matches_file(&self, path: &Path) -> bool {
        // Strictly parse compact `transcript.jsonl` inside `.system_generated/logs/`,
        // skipping `transcript_full.jsonl` to prevent duplicate counting of the same conversation.
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if !file_name.eq_ignore_ascii_case("transcript.jsonl") {
            return false;
        }
        let parent_is_logs = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("logs"));
        parent_is_logs
    }

    fn init_context(&self, path: &Path, device_id: &str) -> FileParseContext {
        // Path shape: .../<conversation-id>/.system_generated/logs/transcript.jsonl
        let session_id = path
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("antigravity-conv")
            .to_string();

        let initial_meta = scan_initial_antigravity_metadata(path);
        let initial_model = initial_meta
            .active_model
            .unwrap_or_else(|| DEFAULT_ANTIGRAVITY_MODEL.to_string());
        let detected_plan_hint =
            resolve_antigravity_plan_hint(path).or(initial_meta.model_tier_hint);
        let has_initial_title = initial_meta.initial_title.is_some();

        FileParseContext {
            file_path: path.to_string_lossy().to_string(),
            device_id: device_id.to_string(),
            session_id,
            project_label: "antigravity".to_string(),
            current_model: Some(initial_model),
            detected_plan_hint,
            session_title: initial_meta.initial_title,
            session_title_dirty: has_initial_title,
            ..Default::default()
        }
    }

    fn parse_line(&self, line: &str, ctx: &mut FileParseContext) -> Option<ParsedLineResult> {
        ctx.line_number += 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }
        let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
            return None;
        };

        let ts_ms = parse_timestamp_ms(val.get("created_at").or_else(|| val.get("timestamp")))
            .unwrap_or_else(now_ms);

        ctx.last_step_ts_ms = Some(ts_ms);

        let source = val.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let step_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");

        if let Some(title) = val
            .get("title")
            .or_else(|| val.get("conversation_title"))
            .and_then(|v| v.as_str())
        {
            ctx.record_session_title(title, "explicit", ts_ms);
        }

        if let Some(explicit_tier) = val
            .get("plan_tier")
            .or_else(|| val.get("subscription_tier"))
            .or_else(|| val.get("tier"))
            .and_then(|v| v.as_str())
            .and_then(normalize_explicit_antigravity_tier)
        {
            if ctx.detected_plan_hint.as_deref() != Some(explicit_tier.as_str()) {
                ctx.detected_plan_hint = Some(explicit_tier);
                ctx.plan_hint_emitted = false;
            }
        }

        let content_opt = val.get("content").and_then(|v| v.as_str());
        if !source.eq_ignore_ascii_case("SYSTEM") && step_type.eq_ignore_ascii_case("USER_INPUT") {
            if let Some(content) = content_opt {
                if let Some(selected_model) = extract_model_selection_from_content(content) {
                    if let Some(model_hint) =
                        infer_antigravity_tier_from_model_name(&selected_model)
                    {
                        if ctx.detected_plan_hint.is_none() {
                            ctx.detected_plan_hint = Some(model_hint);
                            ctx.plan_hint_emitted = false;
                        }
                    }
                    ctx.current_model = Some(selected_model);
                }
            }
        }

        let mut rate_limits =
            extract_antigravity_rate_limits(&val, ts_ms, ctx.detected_plan_hint.as_deref());
        if !rate_limits.is_empty() {
            ctx.plan_hint_emitted = true;
        }

        // When a `GENERIC` tool execution step immediately follows a `PLANNER_RESPONSE` that had
        // `tool_calls`, its `created_at` marks the exact completion of the model's generation step.
        if step_type.eq_ignore_ascii_case("GENERIC") {
            if let Some(mut pending) = ctx.pending_antigravity_event.take() {
                let decode_dur = ts_ms - pending.timestamp_ms;
                if decode_dur > 0 && decode_dur <= 600_000 {
                    pending.duration_ms = Some(decode_dur);
                    let eff_out = pending.output_tokens + pending.reasoning_tokens;
                    pending.output_tps = compute_tps(eff_out, Some(decode_dur));
                    return Some(ParsedLineResult {
                        event: Some(pending),
                        rate_limits,
                        balance: None,
                    });
                }
            }
        }

        let is_user_step = !source.eq_ignore_ascii_case("SYSTEM")
            && (step_type.eq_ignore_ascii_case("USER_INPUT")
                || source.eq_ignore_ascii_case("USER_EXPLICIT"));
        if is_user_step {
            ctx.pending_antigravity_event = None;
            ctx.last_prompt_ts_ms = Some(ts_ms);
            if let Some(user_req) = content_opt.and_then(extract_antigravity_user_request) {
                ctx.record_session_title(user_req, "user_prompt", ts_ms);
            }
        }

        // Extract explicit token counters when publicly recorded in transcript.jsonl
        let input_tokens = get_i64(&val, &["input_tokens", "prompt_tokens"]);
        let output_tokens = get_i64(&val, &["output_tokens", "completion_tokens"]);
        let cache_read_tokens = get_i64(&val, &["cache_read_tokens", "cached_tokens"]);
        let cache_write_tokens = get_i64(&val, &["cache_write_tokens"]);
        let reasoning_tokens = get_i64(&val, &["reasoning_tokens", "thinking_tokens"]);
        let total_tokens = input_tokens
            + output_tokens
            + cache_read_tokens
            + cache_write_tokens
            + reasoning_tokens;

        let upper_type = step_type.to_ascii_uppercase();
        let is_model_step = step_type.eq_ignore_ascii_case("PLANNER_RESPONSE")
            || upper_type.ends_with("_RESPONSE")
            || total_tokens > 0;
        if !is_model_step {
            return if rate_limits.is_empty() {
                None
            } else {
                Some(ParsedLineResult {
                    event: None,
                    rate_limits,
                    balance: None,
                })
            };
        }

        ctx.pending_antigravity_event = None;

        // `CHECKPOINT` / `SYSTEM` compaction steps record cumulative context compaction tokens,
        // not live streaming generation; never compute generation duration or TPS on them.
        let is_compaction_or_system = step_type.eq_ignore_ascii_case("CHECKPOINT")
            || source.eq_ignore_ascii_case("SYSTEM");
        let has_tool_calls = val
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty());

        let explicit_dur = val
            .get("duration_ms")
            .and_then(|v| v.as_i64())
            .filter(|&d| d > 0);
        let explicit_ttft = val.get("ttft_ms").and_then(|v| v.as_i64());

        let effective_output = output_tokens + reasoning_tokens;
        let prompt_delta = if is_compaction_or_system {
            None
        } else {
            ctx.last_prompt_ts_ms.take().and_then(|prompt_ts| {
                let delta = ts_ms - prompt_ts;
                if delta > 0 && delta <= 600_000 {
                    Some((prompt_ts, delta))
                } else {
                    None
                }
            })
        };

        let (duration_ms, ttft_ms) = if is_compaction_or_system {
            (None, None)
        } else if explicit_dur.is_some() {
            (explicit_dur, explicit_ttft)
        } else if has_tool_calls {
            // `created_at` on `PLANNER_RESPONSE` with `tool_calls` is the step start timestamp;
            // the following `GENERIC` step's `created_at` will supply the true generation duration.
            let ttft = explicit_ttft
                .or_else(|| prompt_delta.and_then(|(_, d)| (d <= 60_000).then_some(d)));
            (None, ttft)
        } else {
            let dur = prompt_delta.and_then(|(prompt_ts, delta)| {
                let both_second_quantized = ts_ms % 1_000 == 0 && prompt_ts % 1_000 == 0;
                if both_second_quantized && delta <= 2_000 && effective_output >= 80 {
                    None
                } else {
                    Some(delta)
                }
            });
            (dur, explicit_ttft)
        };

        let observable_only = total_tokens == 0;
        let output_tps = compute_tps(effective_output, duration_ms);

        let model_id = val
            .get("model")
            .or_else(|| val.get("model_id"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .or_else(|| ctx.current_model.clone())
            .unwrap_or_else(|| DEFAULT_ANTIGRAVITY_MODEL.to_string());

        if !ctx.plan_hint_emitted {
            if let Some(hint) = ctx
                .detected_plan_hint
                .clone()
                .or_else(|| infer_antigravity_tier_from_model_name(&model_id))
            {
                ctx.plan_hint_emitted = true;
                rate_limits.push(ObservedRateLimitWindow {
                    provider_id: provider_ids::ANTIGRAVITY.to_string(),
                    window_name: "tier_hint".to_string(),
                    used_percent: 0.0,
                    window_minutes: None,
                    resets_at_ms: None,
                    observed_at_ms: ts_ms,
                    source: Some("local_log_hint".to_string()),
                    plan_hint: Some(hint),
                });
            }
        }

        let step_key = val
            .get("step_index")
            .and_then(|v| v.as_i64())
            .map(|idx| format!("step-{idx}"))
            .unwrap_or_else(|| format!("line-{}", ctx.line_number));

        let event_id = compute_event_id(
            provider_ids::ANTIGRAVITY,
            &ctx.session_id,
            &step_key,
            ts_ms,
        );

        let event = NormalizedUsageEvent {
            event_id,
            device_id: ctx.device_id.clone(),
            provider_id: provider_ids::ANTIGRAVITY.to_string(),
            session_id: ctx.session_id.clone(),
            project_label: ctx.project_label.clone(),
            model_id,
            timestamp_ms: ts_ms,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
            reasoning_tokens,
            observable_only,
            duration_ms,
            ttft_ms,
            output_tps,
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        };

        if !is_compaction_or_system && has_tool_calls && duration_ms.is_none() && !observable_only {
            ctx.pending_antigravity_event = Some(event.clone());
        }

        Some(ParsedLineResult {
            event: Some(event),
            rate_limits,
            balance: None,
        })
    }

    fn finalize_batch(&self, events: Vec<NormalizedUsageEvent>) -> Vec<NormalizedUsageEvent> {
        collapse_duplicate_events_keep_last(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agy_probe_backoff_doubles_from_thirty_minutes_and_caps_at_six_hours() {
        assert_eq!(agy_backoff_delay(0), Duration::ZERO);
        assert_eq!(agy_backoff_delay(1), Duration::from_secs(30 * 60));
        assert_eq!(agy_backoff_delay(2), Duration::from_secs(60 * 60));
        assert_eq!(agy_backoff_delay(3), Duration::from_secs(120 * 60));
        assert_eq!(agy_backoff_delay(4), Duration::from_secs(240 * 60));
        assert_eq!(agy_backoff_delay(5), Duration::from_secs(6 * 3_600));
        assert_eq!(agy_backoff_delay(u32::MAX), Duration::from_secs(6 * 3_600));
    }

    #[test]
    fn extracts_model_selection_preserving_version_dots_and_ignoring_plain_text() {
        let claude = "<USER_REQUEST>hi</USER_REQUEST>\n<USER_SETTINGS_CHANGE>\nThe user changed setting `Model Selection` from None to Claude Sonnet 5.5 (Medium). No need to comment on this change.\n</USER_SETTINGS_CHANGE>";
        assert_eq!(
            extract_model_selection_from_content(claude).as_deref(),
            Some("Claude Sonnet 5.5 (Medium)")
        );

        let hydrogen = "<USER_SETTINGS_CHANGE>The user changed setting `Model Selection` from None to Gemini 4 Hydrogen (Medium).</USER_SETTINGS_CHANGE>";
        assert_eq!(
            extract_model_selection_from_content(hydrogen).as_deref(),
            Some("Gemini 4 Hydrogen (Medium)")
        );

        let orchestrator = "<USER_SETTINGS_CHANGE>\nThe user changed setting `Model Selection` from Gemini 4 Argon (Medium) to Orchestrator (Argon). No need to comment.\n</USER_SETTINGS_CHANGE>";
        assert_eq!(
            extract_model_selection_from_content(orchestrator).as_deref(),
            Some("Orchestrator (Argon)")
        );

        let plain_prose = "<USER_REQUEST>The user changed setting `Model Selection` from None to FakeModel.</USER_REQUEST>";
        assert_eq!(extract_model_selection_from_content(plain_prose), None);
    }

    #[test]
    fn generic_tool_output_and_context_summary_never_hijack_model_selection() {
        let parser = AntigravityParser::new();
        let mut ctx = parser.init_context(
            Path::new("/tmp/conv-hijack/.system_generated/logs/transcript.jsonl"),
            "dev-1",
        );

        let dir = tempfile::tempdir().expect("tempdir");
        let transcript = dir.path().join("transcript.jsonl");
        let jsonl = [
            // Genuine USER_INPUT selects Gemini 4 Argon (High)
            r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-16T01:00:00Z","content":"<USER_REQUEST>Fix antigravity bug</USER_REQUEST>\n<USER_SETTINGS_CHANGE>\nThe user changed setting `Model Selection` from None to Gemini 4 Argon (High). No need to comment.\n</USER_SETTINGS_CHANGE>"}"#,
            // GENERIC tool output (e.g. read_file/Select-String on source code or fixture) must be ignored
            r#"{"step_index":1,"source":"SYSTEM","type":"GENERIC","status":"DONE","created_at":"2026-11-16T01:00:05Z","content":"<USER_SETTINGS_CHANGE>The user changed setting `Model Selection` from None to Gemini 3.1 Pro (High).</USER_SETTINGS_CHANGE>"}"#,
            // USER_INPUT quoting <CONTEXT_SUMMARY> with a prior session's model text must also be ignored
            r#"{"step_index":2,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-16T01:00:10Z","content":"<CONTEXT_SUMMARY><USER_SETTINGS_CHANGE>The user changed setting `Model Selection` from None to Gemini 3.1 Pro (High).</USER_SETTINGS_CHANGE></CONTEXT_SUMMARY>\n<USER_REQUEST><USER_SETTINGS_CHANGE>The user changed setting `Model Selection` from None to Gemini 3.1 Pro (High).</USER_SETTINGS_CHANGE></USER_REQUEST>"}"#,
            // USER_INPUT containing malformed tag without "changed setting" phrase must be ignored
            r#"{"step_index":3,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-16T01:00:15Z","content":"<USER_SETTINGS_CHANGE>Model Selection from None to Gemini 3.1 Pro (High).</USER_SETTINGS_CHANGE>"}"#,
            // Actual response should still be attributed to Gemini 4 Argon (High)
            r#"{"step_index":4,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-11-16T01:00:25Z","content":"Patch applied.","usage":{"input_tokens":1000,"output_tokens":200}}"#,
        ]
        .join("\n");
        std::fs::write(&transcript, jsonl).expect("write transcript");

        let meta = scan_initial_antigravity_metadata(&transcript);
        assert_eq!(meta.active_model.as_deref(), Some("Gemini 4 Argon (High)"));

        let mut events = Vec::new();
        for line in std::fs::read_to_string(&transcript).expect("read").lines() {
            if let Some(parsed) = parser.parse_line(line, &mut ctx) {
                events.extend(parsed.event);
            }
        }
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].model_id, "Gemini 4 Argon (High)");
    }

    #[test]
    fn tracks_model_changes_across_session_and_prefers_explicit_model_field() {
        let parser = AntigravityParser::new();
        let mut ctx = parser.init_context(
            Path::new("/tmp/conv-abc/.system_generated/logs/transcript.jsonl"),
            "dev-1",
        );

        let line1 = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-15T16:34:58Z","content":"<USER_REQUEST>hello</USER_REQUEST>\n<USER_SETTINGS_CHANGE>\nThe user changed setting `Model Selection` from None to Claude Sonnet 5.5 (Medium). No need to comment.\n</USER_SETTINGS_CHANGE>"}"#;
        assert!(parser.parse_line(line1, &mut ctx).is_none());

        let line2 = r#"{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-11-15T16:35:00Z","input_tokens":1000,"output_tokens":50}"#;
        let ev1 = parser
            .parse_line(line2, &mut ctx)
            .and_then(|r| r.event)
            .expect("planner response");
        assert_eq!(ev1.model_id, "Claude Sonnet 5.5 (Medium)");

        let line3 = r#"{"step_index":2,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-15T16:36:00Z","content":"<USER_SETTINGS_CHANGE>The user changed setting `Model Selection` from Claude Sonnet 5.5 (Medium) to Gemini 4 Hydrogen (Medium).</USER_SETTINGS_CHANGE>"}"#;
        assert!(parser.parse_line(line3, &mut ctx).is_none());

        let line4 = r#"{"step_index":3,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-11-15T16:36:05Z","input_tokens":500,"output_tokens":20}"#;
        let ev2 = parser
            .parse_line(line4, &mut ctx)
            .and_then(|r| r.event)
            .expect("second response");
        assert_eq!(ev2.model_id, "Gemini 4 Hydrogen (Medium)");

        let line5 = r#"{"step_index":4,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-11-15T16:36:10Z","model":"gemini-3-pro","input_tokens":200,"output_tokens":10}"#;
        let ev3 = parser
            .parse_line(line5, &mut ctx)
            .and_then(|r| r.event)
            .expect("explicit model response");
        assert_eq!(ev3.model_id, "gemini-3-pro");

        // Session title extracted cleanly from <USER_REQUEST>, ignoring preceding non-user step timing
        assert_eq!(
            ctx.session_title.as_ref().map(|t| t.0.as_str()),
            Some("hello")
        );
        // Step 4 followed step 3 without a new USER_INPUT prompt: must NOT divide 10 tokens by 5s prev_step
        assert_eq!(ev3.duration_ms, None);
    }

    #[test]
    fn planner_response_with_tool_calls_uses_generic_step_completion_and_ignores_checkpoint() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings_path = dir.path().join("settings.json");
        std::fs::write(
            &settings_path,
            r#"{"useG1Credits":true,"subscriptionTier":"Google AI Ultra 5x"}"#,
        )
        .expect("write settings");

        let transcript = dir
            .path()
            .join("brain")
            .join("conv-99")
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl");
        std::fs::create_dir_all(transcript.parent().expect("parent")).expect("mkdir");
        std::fs::write(&transcript, "").expect("touch transcript");

        let parser = AntigravityParser::new();
        let mut ctx = parser.init_context(&transcript, "dev-1");
        assert_eq!(ctx.detected_plan_hint.as_deref(), Some("ultra_5x"));

        // 1. User prompt at 01:13:07Z, PLANNER_RESPONSE (with tool_calls, 400 tokens) also starts at 01:13:07Z,
        //    and GENERIC tool step starts at 01:13:23Z (16s true decode duration -> 25.0 tok/s).
        let user_step = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-16T01:13:07Z","content":"<USER_REQUEST>Audit speed</USER_REQUEST>"}"#;
        let planner_step = r#"{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-11-16T01:13:07Z","input_tokens":19000,"output_tokens":400,"tool_calls":[{"name":"read_file"}]}"#;
        let generic_step = r#"{"step_index":2,"source":"MODEL","type":"GENERIC","status":"DONE","created_at":"2026-11-16T01:13:23Z","content":"Created At: 2026-11-16T09:13:23+08:00"}"#;
        let checkpoint_step = r#"{"step_index":3,"source":"SYSTEM","type":"CHECKPOINT","status":"DONE","created_at":"2026-11-16T01:13:24Z","input_tokens":490000,"output_tokens":2800,"content":"Compaction summary"}"#;

        assert!(parser.parse_line(user_step, &mut ctx).is_none());
        let p_res = parser.parse_line(planner_step, &mut ctx).expect("planner");
        assert_eq!(p_res.rate_limits.len(), 1);
        assert_eq!(p_res.rate_limits[0].plan_hint.as_deref(), Some("ultra_5x"));
        let p_ev = p_res.event.expect("planner event");
        assert_eq!(p_ev.duration_ms, None);
        assert_eq!(p_ev.output_tps, None);

        let g_ev = parser
            .parse_line(generic_step, &mut ctx)
            .and_then(|r| r.event)
            .expect("upgraded planner event on generic step");
        assert_eq!(g_ev.event_id, p_ev.event_id);
        assert_eq!(g_ev.duration_ms, Some(16_000));
        assert!((g_ev.output_tps.expect("valid tps") - 25.0).abs() < 1e-6);

        let cp_ev = parser
            .parse_line(checkpoint_step, &mut ctx)
            .and_then(|r| r.event)
            .expect("checkpoint event");
        assert_eq!(cp_ev.duration_ms, None);
        assert_eq!(cp_ev.output_tps, None);

        let finalized = parser.finalize_batch(vec![p_ev, g_ev, cp_ev]);
        assert_eq!(finalized.len(), 2);
        assert_eq!(finalized[0].duration_ms, Some(16_000));
        assert!((finalized[0].output_tps.expect("final tps") - 25.0).abs() < 1e-6);
        assert_eq!(finalized[1].output_tps, None);
    }

    #[test]
    fn locks_first_user_prompt_title_and_avoids_false_pro_from_g1_or_gemini_pro() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings_path = dir.path().join("settings.json");
        std::fs::write(&settings_path, r#"{"useG1Credits":true}"#).expect("write settings");
        assert_eq!(parse_settings_json_for_plan_hint(&settings_path), None);

        std::fs::write(
            &settings_path,
            r#"{"agent":{"preferredModelTier":"g1-ultra-tier"}}"#,
        )
        .expect("write ultra settings");
        assert_eq!(
            parse_settings_json_for_plan_hint(&settings_path).as_deref(),
            Some("ultra")
        );

        assert_eq!(infer_antigravity_tier_from_model_name("Gemini 3.1 Pro (High)"), None);
        assert_eq!(
            infer_antigravity_tier_from_model_name("Orchestrator (Carbon)").as_deref(),
            Some("ultra")
        );

        let transcript = dir
            .path()
            .join("brain")
            .join("conv-first-msg")
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl");
        std::fs::create_dir_all(transcript.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            &transcript,
            concat!(
                r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-16T01:00:00Z","content":"<USER_REQUEST>Build initial dark glass layout</USER_REQUEST>\n<USER_SETTINGS_CHANGE>The user changed setting `Model Selection` from None to Gemini 4 Argon (High).</USER_SETTINGS_CHANGE>"}"#,
                "\n",
                r#"{"step_index":1,"source":"SYSTEM","type":"USER_INPUT","status":"DONE","created_at":"2026-11-16T01:01:00Z","content":"[plan review] internal handoff"}"#,
                "\n",
                r#"{"step_index":2,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-16T01:05:00Z","content":"<USER_REQUEST>Continue step 3</USER_REQUEST>"}"#,
                "\n",
            ),
        )
        .expect("write transcript");

        let parser = AntigravityParser::new();
        let mut ctx = parser.init_context(&transcript, "dev-1");
        assert_eq!(
            ctx.session_title.as_ref().map(|t| t.0.as_str()),
            Some("Build initial dark glass layout")
        );

        // Even feeding later turn or mid-file warmup keeps first sentence locked
        ctx.lock_non_explicit_title = true;
        assert!(parser
            .parse_line(
                r#"{"step_index":3,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-16T02:00:00Z","content":"<USER_REQUEST>Latest message must not win</USER_REQUEST>"}"#,
                &mut ctx,
            )
            .is_none());
        assert_eq!(
            ctx.session_title.as_ref().map(|t| t.0.as_str()),
            Some("Build initial dark glass layout")
        );
    }

    #[test]
    fn parses_live_agy_cli_usage_json_without_false_ultra_from_group_description() {
        let sample = r#"{
          "conversation_id": "",
          "status": "SUCCESS",
          "response": "Gemini Models\tWeekly Limit Remaining\t36%\t2026-11-22T14:36:11Z\n",
          "duration_seconds": 0,
          "num_turns": 0,
          "usage": {"input_tokens":0,"output_tokens":0,"thinking_tokens":0,"cache_read_tokens":0,"total_tokens":0},
          "command": {
            "name": "usage",
            "data": {
              "description": "Within each group, models share a weekly limit and a 5-hour limit.",
              "groups": [
                {
                  "name": "Gemini Models",
                  "description": "Models within this group: Gemini 4 Carbon, Gemini 4 Argon, Gemini 4 Hydrogen",
                  "buckets": [
                    {"id": "gemini-weekly", "name": "Weekly Limit Remaining", "window": "weekly", "remaining_fraction": 0.3614097535610199, "reset_time": "2026-11-22T14:36:11Z"},
                    {"id": "gemini-5h", "name": "Five Hour Limit Remaining", "window": "5h", "remaining_fraction": 0.634879469871521, "reset_time": "2026-11-16T17:13:50Z"}
                  ]
                },
                {
                  "name": "Claude and GPT models",
                  "description": "Models within this group: Claude Opus 5.5, Claude Sonnet 5.5, GPT-OSS 120B",
                  "buckets": [
                    {"id": "3p-weekly", "name": "Weekly Limit Remaining", "window": "weekly", "remaining_fraction": 0.9954540729522705, "reset_time": "2026-11-22T16:34:57Z"},
                    {"id": "3p-5h", "name": "Five Hour Limit Remaining", "window": "5h", "remaining_fraction": 1, "reset_time": "2026-11-16T20:57:37Z"}
                  ]
                }
              ]
            }
          }
        }"#;
        let now_ms = 1_794_840_000_000i64;
        let (snap, windows) =
            parse_antigravity_cli_usage_output(sample, now_ms, None).expect("parse agy json");
        assert_eq!(snap.provider_id, provider_ids::ANTIGRAVITY);
        assert_eq!(snap.currency, "QUOTA");
        assert_eq!(snap.fetched_at_ms, now_ms);
        assert_eq!(windows.len(), 4);

        // Critical: static "Gemini 4 Carbon" inside groups[0].description must NOT fabricate ultra plan_hint
        assert!(windows.iter().all(|w| w.plan_hint.is_none()));
        assert!(windows
            .iter()
            .all(|w| w.source.as_deref() == Some("official_cli")));

        assert_eq!(windows[0].window_name, "gemini-weekly");
        assert_eq!(windows[0].window_minutes, Some(10_080));
        assert!((windows[0].used_percent - 63.8590246).abs() < 0.01);
        assert!(windows[0].resets_at_ms.is_some());

        assert_eq!(windows[1].window_name, "gemini-5h");
        assert_eq!(windows[1].window_minutes, Some(300));
        assert!((windows[1].used_percent - 36.512053).abs() < 0.01);

        assert_eq!(windows[2].window_name, "3p-weekly");
        assert!((windows[2].used_percent - 0.4545927).abs() < 0.01);

        assert_eq!(windows[3].window_name, "3p-5h");
        assert!((windows[3].used_percent - 0.0).abs() < 0.01);

        let err = parse_antigravity_cli_usage_output(
            r#"{"status":"ERROR","error":"Not signed in"}"#,
            now_ms,
            None,
        )
        .expect_err("error status must fail");
        assert!(err.to_string().contains("Not signed in"));
    }

    #[test]
    fn agy_auth_pause_detects_auth_or_timeout_errors_and_resets_on_explicit_sync() {
        assert!(is_agy_auth_or_interactive_error("agy CLI 返回状态 ERROR: Not signed in"));
        assert!(is_agy_auth_or_interactive_error("oauth invalid_grant from accounts.google.com"));
        assert!(is_agy_auth_or_interactive_error("调用官方 agy CLI /usage 超时（>25s）"));
        assert!(is_agy_auth_or_interactive_error("请先在终端运行 agy 完成账号登录以获取配额读数"));
        assert!(!is_agy_auth_or_interactive_error("temporary dns lookup failure"));

        reset_agy_probe_backoff();
        assert!(!agy_auth_paused());
        assert!(agy_backoff_remaining().is_none());

        // Transient non-auth error sets exponential backoff without permanent auth pause
        record_agy_probe_result(Some("temporary 503 service unavailable"));
        assert!(!agy_auth_paused());
        assert!(agy_backoff_remaining().is_some());

        // Auth / timeout error immediately sets auth_paused
        record_agy_probe_result(Some("agy CLI 返回状态 ERROR: Not signed in"));
        assert!(agy_auth_paused());
        assert!(agy_backoff_remaining().is_none());

        // Explicit user sync / login clears both auth_paused and backoff
        reset_agy_probe_backoff();
        assert!(!agy_auth_paused());
        assert!(agy_backoff_remaining().is_none());
    }
}
