//! Claude Code official CLI subscription + quota probe.
//!
//! Compliance boundary (AGENTS.md §3.1.4): only the documented, non-interactive public CLI
//! commands `claude auth status --json` and `claude -p -- /usage` are spawned. `/usage` is a local
//! slash command (`num_turns = 0`, zero tokens), so probing never consumes subscription quota.
//! TokenVane never reads `~/.claude/.credentials.json`, never touches OAuth tokens and never calls
//! private Anthropic endpoints.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, NaiveDate, NaiveTime, TimeZone, Utc};
use serde_json::Value;
use tokvane_core::domain::{provider_ids, ObservedRateLimitWindow, ProviderBalanceSnapshot};

use crate::error::{CollectorError, CollectorResult};
use crate::providers::user_home_dir;

/// Human-readable endpoint label reported in refresh outcomes.
pub const CLAUDE_CLI_ENDPOINT: &str = "claude auth status --json + claude -p -- /usage";

const AUTH_STATUS_TIMEOUT: Duration = Duration::from_secs(20);
const USAGE_TIMEOUT: Duration = Duration::from_secs(40);
/// Subscription tier rarely changes; reuse a successful `auth status` probe for this long.
const AUTH_CACHE_TTL: Duration = Duration::from_secs(6 * 3_600);

static AUTH_CACHE: Mutex<Option<(ClaudeAuthStatus, Instant)>> = Mutex::new(None);
static CLAUDE_AUTH_PAUSED: Mutex<Option<String>> = Mutex::new(None);

/// Drop the cached `claude auth status` result and resume probing (e.g. after manual sync or login).
pub fn invalidate_claude_auth_cache() {
    if let Ok(mut guard) = AUTH_CACHE.lock() {
        *guard = None;
    }
    if let Ok(mut guard) = CLAUDE_AUTH_PAUSED.lock() {
        *guard = None;
    }
}

fn paused_claude_auth_error() -> Option<String> {
    CLAUDE_AUTH_PAUSED.lock().ok()?.clone()
}

fn pause_claude_probes(reason: &str) {
    if let Ok(mut guard) = CLAUDE_AUTH_PAUSED.lock() {
        *guard = Some(reason.to_string());
    }
}

fn cached_auth_status() -> Option<ClaudeAuthStatus> {
    let guard = AUTH_CACHE.lock().ok()?;
    let (status, at) = guard.as_ref()?;
    (at.elapsed() < AUTH_CACHE_TTL).then(|| status.clone())
}

fn store_auth_status(status: &ClaudeAuthStatus) {
    if let Ok(mut guard) = AUTH_CACHE.lock() {
        *guard = Some((status.clone(), Instant::now()));
    }
}

/// Hardening flags: user-level settings only, no MCP servers, no tools, no session persistence.
const USAGE_ARGS: &[&str] = &[
    "--setting-sources",
    "user",
    "--strict-mcp-config",
    "--tools",
    "",
    "--no-session-persistence",
    "-p",
    "--output-format",
    "json",
    "--",
    "/usage",
];

/// Parsed subset of `claude auth status --json`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClaudeAuthStatus {
    pub logged_in: bool,
    pub auth_method: Option<String>,
    pub subscription_type: Option<String>,
    pub plan_tier: Option<String>,
}

impl ClaudeAuthStatus {
    /// Combined plan hint consumed by auto-subscription discovery (e.g. `"pro"`, `"max max_20x"`).
    pub fn plan_hint(&self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        for raw in [&self.subscription_type, &self.plan_tier].into_iter().flatten() {
            let trimmed = raw.trim().to_ascii_lowercase();
            if !trimmed.is_empty() && !parts.contains(&trimmed) {
                parts.push(trimmed);
            }
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }

    fn uses_subscription(&self) -> bool {
        self.auth_method
            .as_deref()
            .map(|m| m.eq_ignore_ascii_case("claude.ai"))
            .unwrap_or(false)
            || self.subscription_type.is_some()
    }
}

#[cfg(any(windows, test))]
fn npm_native_claude_exe(dir: &Path) -> PathBuf {
    dir.join("node_modules")
        .join("@anthropic-ai")
        .join("claude-code")
        .join("bin")
        .join("claude.exe")
}

#[cfg(any(windows, test))]
fn resolve_windows_claude_candidates(
    home: Option<&Path>,
    path_dirs: &[PathBuf],
    app_data: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(home) = home {
        candidates.push(home.join(".local").join("bin").join("claude.exe"));
    }
    for dir in path_dirs {
        candidates.push(dir.join("claude.exe"));
        candidates.push(npm_native_claude_exe(dir));
        candidates.push(dir.join("claude.cmd"));
    }
    if let Some(app_data) = app_data {
        let npm_dir = app_data.join("npm");
        candidates.push(npm_native_claude_exe(&npm_dir));
        candidates.push(npm_dir.join("claude.cmd"));
    }
    candidates
}

/// Locate the official Claude Code CLI executable (native installer or npm shim).
pub fn resolve_claude_cli_binary() -> Option<PathBuf> {
    if let Some(override_bin) = std::env::var_os("TOKENVANE_CLAUDE_BIN") {
        let path = PathBuf::from(override_bin);
        if path.is_file() {
            return Some(path);
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        let home = user_home_dir();
        let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        let app_data = std::env::var_os("APPDATA").map(PathBuf::from);
        candidates.extend(resolve_windows_claude_candidates(
            home.as_deref(),
            &path_dirs,
            app_data.as_deref(),
        ));
    }

    #[cfg(not(windows))]
    {
        if let Some(home) = user_home_dir() {
            candidates.push(home.join(".local").join("bin").join("claude"));
            candidates.push(home.join(".claude").join("local").join("claude"));
        }
        candidates.push(PathBuf::from("/opt/homebrew/bin/claude"));
        candidates.push(PathBuf::from("/usr/local/bin/claude"));
        if let Some(path_env) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_env) {
                candidates.push(dir.join("claude"));
            }
        }
    }

    candidates.into_iter().find(|p| p.is_file())
}

/// Dedicated private working directory for CLI probes so no project-level settings are loaded.
fn claude_probe_dir() -> CollectorResult<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join("TokenVane"))
        .or_else(|| user_home_dir().map(|h| h.join(".tokenvane")))
        .ok_or_else(|| CollectorError::Compliance("无法定位 TokenVane 本地数据目录".to_string()))?;
    let dir = base.join("claude-probe");
    std::fs::create_dir_all(&dir)
        .map_err(|e| CollectorError::Compliance(format!("创建 Claude CLI 探测目录失败: {e}")))?;
    Ok(dir)
}

struct CliOutput {
    success: bool,
    status_desc: String,
    stdout: String,
    stderr: String,
}

fn run_claude_cli_blocking(
    bin_path: &Path,
    args: &[&str],
    cwd: &Path,
    timeout: Duration,
) -> CollectorResult<CliOutput> {
    // Rust std escapes arguments for `.cmd` / `.bat` shims safely (including empty strings).
    let mut cmd = std::process::Command::new(bin_path);
    cmd.args(args)
        .current_dir(cwd)
        .env("CLAUDE_CODE_SKIP_PROMPT_HISTORY", "1")
        // Documented Claude Code switches: a read-only probe must never self-update the user's CLI
        // or start telemetry/error-reporting helpers, which also keeps background helper
        // processes (and any console windows they might create) to a minimum.
        .env("DISABLE_AUTOUPDATER", "1")
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    crate::cli_process::apply_headless_no_browser_env(&mut cmd);
    crate::cli_process::configure_hidden_cli_command(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|e| CollectorError::Compliance(format!("启动 Claude Code CLI 失败: {e}")))?;
    let job_guard = crate::cli_process::ChildProcessGuard::attach(&child);
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let (status, stdout_bytes, stderr_bytes) = std::thread::scope(|scope| {
        let out_handle = scope.spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut r) = stdout_pipe {
                let _ = r.read_to_end(&mut buf);
            }
            buf
        });
        let err_handle = scope.spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut r) = stderr_pipe {
                let _ = r.read_to_end(&mut buf);
            }
            buf
        });

        let deadline = std::time::Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(st)) => break Ok(st),
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        job_guard.terminate_tree();
                        let _ = child.kill();
                        let _ = child.wait();
                        break Err(CollectorError::Compliance(format!(
                            "调用 Claude Code CLI 超时（>{}s）",
                            timeout.as_secs()
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(40));
                }
                Err(e) => {
                    job_guard.terminate_tree();
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(CollectorError::Compliance(format!(
                        "等待 Claude Code CLI 退出失败: {e}"
                    )));
                }
            }
        };
        let out_b = out_handle.join().unwrap_or_default();
        let err_b = err_handle.join().unwrap_or_default();
        (status, out_b, err_b)
    });

    drop(job_guard);

    let status = status?;
    Ok(CliOutput {
        success: status.success(),
        status_desc: status.to_string(),
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
    })
}

fn stderr_snippet(raw: &str) -> String {
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("Warning: no stdin"))
        .take(2)
        .collect::<Vec<_>>()
        .join(" | ")
        .chars()
        .take(180)
        .collect()
}

fn extract_json_object(raw: &str) -> Option<Value> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end < start {
        return None;
    }
    serde_json::from_str(&raw[start..=end]).ok()
}

fn non_empty_str(val: Option<&Value>) -> Option<String> {
    val.and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

/// Parse `claude auth status --json`. Personal identity fields (email, org) are ignored.
pub fn parse_claude_auth_status(raw: &str) -> CollectorResult<ClaudeAuthStatus> {
    let root = extract_json_object(raw).ok_or_else(|| {
        CollectorError::Compliance("claude auth status 待返回有效 JSON".to_string())
    })?;
    Ok(ClaudeAuthStatus {
        logged_in: root.get("loggedIn").and_then(|v| v.as_bool()).unwrap_or(false),
        auth_method: non_empty_str(root.get("authMethod")),
        subscription_type: non_empty_str(root.get("subscriptionType")),
        plan_tier: non_empty_str(root.get("planTier")),
    })
}

/// Classify a `/usage` quota line into `(window_name, window_minutes)`.
fn classify_usage_line(label: &str) -> Option<(String, i64)> {
    let lower = label.trim().to_ascii_lowercase();
    if lower.starts_with("current session") {
        return Some(("5h".to_string(), 300));
    }
    if lower.starts_with("current week") {
        let scope = label
            .find('(')
            .and_then(|s| label[s + 1..].find(')').map(|e| label[s + 1..s + 1 + e].trim()))
            .unwrap_or("");
        let scope_lower = scope.to_ascii_lowercase();
        if scope.is_empty() || scope_lower.contains("all models") {
            return Some(("7d".to_string(), 10_080));
        }
        let pretty = scope
            .trim_end_matches(" only")
            .trim_end_matches(" Only")
            .trim();
        return Some((format!("{pretty} · 7d 周限额"), 10_080));
    }
    None
}

fn parse_used_percent(rest: &str) -> Option<f64> {
    let pct_idx = rest.find('%')?;
    let num_str: String = rest[..pct_idx]
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let value: f64 = num_str.parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    let after = rest[pct_idx + 1..].trim_start().to_ascii_lowercase();
    let used = if after.starts_with("left") || after.starts_with("remaining") {
        100.0 - value
    } else {
        value
    };
    Some(used.clamp(0.0, 100.0))
}

fn parse_month(token: &str) -> Option<u32> {
    let lower = token.trim_end_matches('.').to_ascii_lowercase();
    if lower.len() < 3 {
        return None;
    }
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    MONTHS
        .iter()
        .position(|m| lower.starts_with(m))
        .map(|i| i as u32 + 1)
}

fn parse_clock(token: &str, meridiem: Option<&str>) -> Option<NaiveTime> {
    let lower = token.to_ascii_lowercase();
    let (body, suffix) = if let Some(b) = lower.strip_suffix("am") {
        (b.to_string(), Some("am"))
    } else if let Some(b) = lower.strip_suffix("pm") {
        (b.to_string(), Some("pm"))
    } else {
        (lower.clone(), meridiem)
    };
    let mut parts = body.split(':');
    let hour: u32 = parts.next()?.trim().parse().ok()?;
    let minute: u32 = match parts.next() {
        Some(m) => m.trim().parse().ok()?,
        None => 0,
    };
    let hour24 = match suffix {
        Some("am") => {
            if hour == 12 {
                0
            } else {
                hour
            }
        }
        Some("pm") => {
            if hour == 12 {
                12
            } else {
                hour + 12
            }
        }
        _ => {
            // Bare numbers without `:` are not clock values (avoid matching day numbers).
            if !body.contains(':') {
                return None;
            }
            hour
        }
    };
    NaiveTime::from_hms_opt(hour24, minute, 0)
}

/// Parse a reset phrase such as `Nov 18, 6:20am (Asia/Shanghai)` or `6:20am (UTC)`.
/// The CLI renders times in the machine's local zone; `(UTC)` is honored explicitly.
fn parse_reset_ms(phrase: &str, now_ms: i64) -> Option<i64> {
    let (body, zone) = match phrase.find('(') {
        Some(idx) => (
            &phrase[..idx],
            phrase[idx + 1..].trim_end_matches(')').trim().to_string(),
        ),
        None => (phrase, String::new()),
    };
    let is_utc = zone.eq_ignore_ascii_case("utc") || zone.eq_ignore_ascii_case("etc/utc");

    let cleaned = body.replace(',', " ");
    let tokens: Vec<&str> = cleaned
        .split_whitespace()
        .filter(|t| !t.eq_ignore_ascii_case("at") && !t.eq_ignore_ascii_case("on"))
        .collect();

    let mut month: Option<u32> = None;
    let mut day: Option<u32> = None;
    let mut time: Option<NaiveTime> = None;
    for (i, tok) in tokens.iter().enumerate() {
        if month.is_none() {
            if let Some(m) = parse_month(tok) {
                month = Some(m);
                continue;
            }
        }
        if time.is_none() {
            let next = tokens.get(i + 1).map(|t| t.to_ascii_lowercase());
            let meridiem = match next.as_deref() {
                Some("am") => Some("am"),
                Some("pm") => Some("pm"),
                _ => None,
            };
            if let Some(t) = parse_clock(tok, meridiem) {
                time = Some(t);
                continue;
            }
        }
        if month.is_some() && day.is_none() {
            if let Ok(d) = tok.parse::<u32>() {
                if (1..=31).contains(&d) {
                    day = Some(d);
                }
            }
        }
    }
    let time = time?;

    let to_ms = |date: NaiveDate| -> Option<i64> {
        let ndt = date.and_time(time);
        if is_utc {
            Some(Utc.from_utc_datetime(&ndt).timestamp_millis())
        } else {
            Local
                .from_local_datetime(&ndt)
                .earliest()
                .map(|dt| dt.timestamp_millis())
        }
    };

    let now_date = if is_utc {
        Utc.timestamp_millis_opt(now_ms).single()?.date_naive()
    } else {
        Local.timestamp_millis_opt(now_ms).single()?.date_naive()
    };

    match (month, day) {
        (Some(m), Some(d)) => {
            let mut candidate = to_ms(NaiveDate::from_ymd_opt(now_date.year(), m, d)?)?;
            // Year rollover (e.g. printed `Jan 2` while it is late December).
            if candidate < now_ms - 86_400_000 {
                candidate = to_ms(NaiveDate::from_ymd_opt(now_date.year() + 1, m, d)?)?;
            }
            Some(candidate)
        }
        _ => {
            let today = to_ms(now_date)?;
            if today >= now_ms {
                Some(today)
            } else {
                to_ms(now_date.succ_opt()?)
            }
        }
    }
}

/// Parse the `/usage` output (plain text or `--output-format json` envelope) into quota windows.
pub fn parse_claude_usage_output(
    raw: &str,
    now_ms: i64,
    plan_hint: Option<String>,
) -> CollectorResult<Vec<ObservedRateLimitWindow>> {
    let text = match extract_json_object(raw) {
        Some(root) => {
            if root.get("is_error").and_then(|v| v.as_bool()) == Some(true) {
                let detail = non_empty_str(root.get("result"))
                    .unwrap_or_else(|| "CLI 返回错误".to_string());
                return Err(CollectorError::Compliance(format!(
                    "Claude CLI /usage 返回错误: {}",
                    detail.chars().take(160).collect::<String>()
                )));
            }
            non_empty_str(root.get("result")).unwrap_or_default()
        }
        None => raw.to_string(),
    };

    let mut windows: Vec<ObservedRateLimitWindow> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(colon) = trimmed.find(':') else {
            continue;
        };
        let (label, rest) = trimmed.split_at(colon);
        let Some((window_name, window_minutes)) = classify_usage_line(label) else {
            continue;
        };
        let rest = &rest[1..];
        let Some(used_percent) = parse_used_percent(rest) else {
            continue;
        };
        let resets_at_ms = rest
            .to_ascii_lowercase()
            .find("resets")
            .and_then(|idx| parse_reset_ms(rest[idx + "resets".len()..].trim(), now_ms));
        if windows.iter().any(|w| w.window_name == window_name) {
            continue;
        }
        windows.push(ObservedRateLimitWindow {
            provider_id: provider_ids::CLAUDE_CODE.to_string(),
            window_name,
            used_percent,
            window_minutes: Some(window_minutes),
            resets_at_ms,
            observed_at_ms: now_ms,
            source: Some("official_cli".to_string()),
            plan_hint: plan_hint.clone(),
        });
    }

    if windows.is_empty() {
        return Err(CollectorError::Compliance(
            "Claude CLI /usage 输出待适配（未识别到会话或周配额）".to_string(),
        ));
    }
    Ok(windows)
}

/// Run both official CLI probes and return a quota snapshot plus authoritative windows.
pub async fn collect_claude_cli_usage(
    now_ms: i64,
) -> CollectorResult<(ProviderBalanceSnapshot, Vec<ObservedRateLimitWindow>)> {
    let bin_path = resolve_claude_cli_binary().ok_or_else(|| {
        CollectorError::Compliance("请先安装官方 Claude Code CLI（claude）".to_string())
    })?;

    tokio::task::spawn_blocking(move || collect_claude_cli_usage_blocking(&bin_path, now_ms))
        .await
        .map_err(|e| CollectorError::Compliance(format!("执行 Claude CLI 后台任务异常: {e}")))?
}

fn probe_auth_status(bin_path: &Path, cwd: &Path) -> CollectorResult<ClaudeAuthStatus> {
    let auth_out = run_claude_cli_blocking(
        bin_path,
        &["auth", "status", "--json"],
        cwd,
        AUTH_STATUS_TIMEOUT,
    )?;
    parse_claude_auth_status(&auth_out.stdout).map_err(|err| {
        let snippet = stderr_snippet(&auth_out.stderr);
        if snippet.is_empty() {
            err
        } else {
            CollectorError::Compliance(format!(
                "claude auth status 退出失败 ({}): {snippet}",
                auth_out.status_desc
            ))
        }
    })
}

pub(crate) fn is_claude_auth_or_timeout_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("not logged in")
        || lower.contains("not signed in")
        || lower.contains("unauthenticated")
        || lower.contains("unauthorized")
        || lower.contains("authentication")
        || msg.contains("待登录")
        || msg.contains("超时")
}

fn collect_claude_cli_usage_blocking(
    bin_path: &Path,
    now_ms: i64,
) -> CollectorResult<(ProviderBalanceSnapshot, Vec<ObservedRateLimitWindow>)> {
    if let Some(reason) = paused_claude_auth_error() {
        return Err(CollectorError::Compliance(reason));
    }

    let cwd = claude_probe_dir()?;

    let auth = match cached_auth_status() {
        Some(cached) => cached,
        None => {
            let fresh = match probe_auth_status(bin_path, &cwd) {
                Ok(st) => st,
                Err(err) => {
                    let msg = err.to_string();
                    if is_claude_auth_or_timeout_error(&msg) {
                        pause_claude_probes(&msg);
                    }
                    return Err(err);
                }
            };
            if fresh.logged_in && fresh.uses_subscription() {
                store_auth_status(&fresh);
            }
            fresh
        }
    };
    if !auth.logged_in {
        let msg = "Claude Code 待登录，已暂停自动探测；请在「用量」完成登录后点击同步".to_string();
        pause_claude_probes(&msg);
        return Err(CollectorError::Compliance(msg));
    }
    if !auth.uses_subscription() {
        let msg =
            "Claude Code 当前为 API 计费登录，订阅配额需使用 Claude 账户登录（已暂停自动探测）"
                .to_string();
        pause_claude_probes(&msg);
        return Err(CollectorError::Compliance(msg));
    }
    let plan_hint = auth.plan_hint();

    let usage_out = match run_claude_cli_blocking(bin_path, USAGE_ARGS, &cwd, USAGE_TIMEOUT) {
        Ok(out) => out,
        Err(err) => {
            let msg = err.to_string();
            if is_claude_auth_or_timeout_error(&msg) {
                pause_claude_probes(&msg);
            }
            return Err(err);
        }
    };
    let windows = match parse_claude_usage_output(&usage_out.stdout, now_ms, plan_hint) {
        Ok(w) => w,
        Err(err) => {
            // Login state may have changed; re-probe `auth status` on the next attempt.
            invalidate_claude_auth_cache();
            let err_to_return = if usage_out.success {
                err
            } else {
                let snippet = stderr_snippet(&usage_out.stderr);
                if snippet.is_empty() {
                    err
                } else {
                    CollectorError::Compliance(format!(
                        "Claude CLI /usage 退出失败 ({}): {snippet}",
                        usage_out.status_desc
                    ))
                }
            };
            let msg = err_to_return.to_string();
            if is_claude_auth_or_timeout_error(&msg) {
                pause_claude_probes(&msg);
            }
            return Err(err_to_return);
        }
    };

    let snapshot = ProviderBalanceSnapshot {
        provider_id: provider_ids::CLAUDE_CODE.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn local_ms(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        Local
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .earliest()
            .expect("valid local time")
            .timestamp_millis()
    }

    #[test]
    fn auth_status_extracts_plan_and_ignores_identity() {
        let raw = r#"{
          "loggedIn": true, "authMethod": "claude.ai", "apiProvider": "firstParty",
          "email": "someone@example.com", "orgId": "x", "subscriptionType": "pro", "planTier": "pro"
        }"#;
        let status = parse_claude_auth_status(raw).expect("parse");
        assert!(status.logged_in);
        assert!(status.uses_subscription());
        assert_eq!(status.plan_hint().as_deref(), Some("pro"));

        let max = parse_claude_auth_status(
            r#"{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"max","planTier":"max_20x"}"#,
        )
        .expect("parse");
        assert_eq!(max.plan_hint().as_deref(), Some("max max_20x"));

        let api = parse_claude_auth_status(r#"{"loggedIn":true,"authMethod":"console"}"#)
            .expect("parse");
        assert!(!api.uses_subscription());
    }

    #[test]
    fn usage_json_envelope_parses_session_and_weekly_windows() {
        let now = local_ms(2026, 11, 18, 5, 22);
        let raw = r#"{"is_error":false,"num_turns":0,"total_cost_usd":0,"local_command":"usage","result":"You are currently using your subscription to power your Claude Code usage\n\nCurrent session: 49% used · resets Nov 18, 6:20am (Asia/Shanghai)\nCurrent week (all models): 7% used · resets Nov 24, 2am (Asia/Shanghai)\nCurrent week (Sonnet only): 3% used · resets Nov 24, 2am (Asia/Shanghai)\n\nLast 24h · 293 requests · 3 sessions\n  65%  Start fresh when you switch tasks · run /clear"}"#;
        let windows =
            parse_claude_usage_output(raw, now, Some("pro".to_string())).expect("windows");
        assert_eq!(windows.len(), 3);

        let session = &windows[0];
        assert_eq!(session.window_name, "5h");
        assert_eq!(session.window_minutes, Some(300));
        assert!((session.used_percent - 49.0).abs() < f64::EPSILON);
        assert_eq!(session.resets_at_ms, Some(local_ms(2026, 11, 18, 6, 20)));
        assert_eq!(session.source.as_deref(), Some("official_cli"));
        assert_eq!(session.plan_hint.as_deref(), Some("pro"));

        let week = &windows[1];
        assert_eq!(week.window_name, "7d");
        assert_eq!(week.window_minutes, Some(10_080));
        assert!((week.used_percent - 7.0).abs() < f64::EPSILON);
        assert_eq!(week.resets_at_ms, Some(local_ms(2026, 11, 24, 2, 0)));

        assert_eq!(windows[2].window_name, "Sonnet · 7d 周限额");
    }

    #[test]
    fn usage_plain_text_handles_time_only_and_year_rollover() {
        let now = local_ms(2026, 12, 31, 22, 0);
        let raw = "Current session: 100% used · resets 1am (Asia/Shanghai)\nCurrent week (all models): 12.5% used · resets Jan 3, 9:30pm (Asia/Shanghai)";
        let windows = parse_claude_usage_output(raw, now, None).expect("windows");
        assert_eq!(windows[0].resets_at_ms, Some(local_ms(2027, 1, 1, 1, 0)));
        assert!((windows[0].used_percent - 100.0).abs() < f64::EPSILON);
        assert_eq!(windows[1].resets_at_ms, Some(local_ms(2027, 1, 3, 21, 30)));
        assert!((windows[1].used_percent - 12.5).abs() < f64::EPSILON);
    }

    #[test]
    fn usage_errors_are_reported_without_fabricating_windows() {
        let now = local_ms(2026, 11, 18, 5, 0);
        assert!(parse_claude_usage_output(r#"{"is_error":true,"result":"Not logged in"}"#, now, None).is_err());
        assert!(parse_claude_usage_output("Hello there", now, None).is_err());
    }

    #[test]
    fn utc_zone_is_respected() {
        let now = Utc
            .with_ymd_and_hms(2026, 11, 18, 0, 0, 0)
            .single()
            .expect("utc")
            .timestamp_millis();
        let reset = parse_reset_ms("Nov 18, 6:20am (UTC)", now).expect("reset");
        let expected = Utc
            .with_ymd_and_hms(2026, 11, 18, 6, 20, 0)
            .single()
            .expect("utc")
            .timestamp_millis();
        assert_eq!(reset, expected);
    }

    #[test]
    fn claude_auth_pause_stops_probes_until_cache_invalidated() {
        assert!(is_claude_auth_or_timeout_error("Not logged in"));
        assert!(is_claude_auth_or_timeout_error("调用 Claude Code CLI 超时（>20s）"));
        assert!(!is_claude_auth_or_timeout_error("temporary parse glitch"));

        invalidate_claude_auth_cache();
        assert!(paused_claude_auth_error().is_none());

        pause_claude_probes("Claude Code 待登录");
        assert_eq!(
            paused_claude_auth_error().as_deref(),
            Some("Claude Code 待登录")
        );

        let err = collect_claude_cli_usage_blocking(Path::new("nonexistent-claude"), 0)
            .expect_err("paused probe must short-circuit without spawning");
        assert!(err.to_string().contains("Claude Code 待登录"));

        invalidate_claude_auth_cache();
        assert!(paused_claude_auth_error().is_none());
    }

    #[test]
    fn windows_claude_resolution_prefers_native_npm_binary_over_cmd_shim() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let app_data = temp.path().join("appdata");
        let npm_dir = app_data.join("npm");
        let native_exe = npm_native_claude_exe(&npm_dir);
        let cmd_shim = npm_dir.join("claude.cmd");
        let home_exe = home.join(".local").join("bin").join("claude.exe");

        std::fs::create_dir_all(&npm_dir).expect("mkdir npm");
        std::fs::write(&cmd_shim, "@echo off\r\n").expect("write shim");

        let pick = |path_dirs: &[PathBuf]| {
            resolve_windows_claude_candidates(Some(&home), path_dirs, Some(&app_data))
                .into_iter()
                .find(|p| p.is_file())
        };

        assert_eq!(pick(std::slice::from_ref(&npm_dir)).as_ref(), Some(&cmd_shim));

        std::fs::create_dir_all(native_exe.parent().expect("parent")).expect("mkdir native bin");
        std::fs::write(&native_exe, b"MZ").expect("write native exe");
        assert_eq!(pick(std::slice::from_ref(&npm_dir)).as_ref(), Some(&native_exe));
        assert_eq!(pick(&[]).as_ref(), Some(&native_exe));

        std::fs::create_dir_all(home_exe.parent().expect("parent")).expect("mkdir local bin");
        std::fs::write(&home_exe, b"MZ").expect("write home exe");
        assert_eq!(pick(std::slice::from_ref(&npm_dir)).as_ref(), Some(&home_exe));
    }
}
