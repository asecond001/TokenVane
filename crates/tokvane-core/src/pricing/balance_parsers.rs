use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::{
    parse_timestamp_ms_json, provider_ids, ObservedRateLimitWindow, ProviderBalanceSnapshot,
};
use crate::error::{CoreError, CoreResult};

/// Summary returned after persisting an official vendor balance/quota REST payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfficialBalanceApplyResult {
    pub balance: ProviderBalanceSnapshot,
    pub observed_windows_updated: usize,
}

fn json_amount_to_micro(val: Option<&Value>) -> Option<i64> {
    let v = val?;
    let parsed = if let Some(n) = v.as_f64() {
        Some(n)
    } else if let Some(i) = v.as_i64() {
        Some(i as f64)
    } else if let Some(s) = v.as_str() {
        s.trim().parse::<f64>().ok()
    } else {
        None
    }?;
    if !parsed.is_finite() {
        return None;
    }
    Some((parsed * 1_000_000.0).round() as i64)
}

fn json_number_as_f64(val: Option<&Value>) -> Option<f64> {
    let v = val?;
    let parsed = if let Some(n) = v.as_f64() {
        Some(n)
    } else if let Some(i) = v.as_i64() {
        Some(i as f64)
    } else if let Some(s) = v.as_str() {
        s.trim().parse::<f64>().ok()
    } else {
        None
    }?;
    parsed.is_finite().then_some(parsed)
}

fn compute_usage_percent_from_obj(obj: &Value) -> Option<f64> {
    if let Some(pct) = json_number_as_f64(
        obj.get("used_percent")
            .or_else(|| obj.get("usedPercent"))
            .or_else(|| obj.get("percent")),
    ) {
        return Some(pct.clamp(0.0, 100.0));
    }
    if let Some(ratio) = json_number_as_f64(
        obj.get("used_ratio")
            .or_else(|| obj.get("usedRatio"))
            .or_else(|| obj.get("ratio")),
    ) {
        let pct = if ratio <= 1.0 { ratio * 100.0 } else { ratio };
        return Some(pct.clamp(0.0, 100.0));
    }
    let limit = json_number_as_f64(obj.get("limit"))?;
    if limit <= 0.0 {
        return None;
    }
    if let Some(used) = json_number_as_f64(obj.get("used")) {
        return Some(((used / limit) * 100.0).clamp(0.0, 100.0));
    }
    if let Some(rem) = json_number_as_f64(obj.get("remaining")) {
        return Some((((limit - rem).max(0.0) / limit) * 100.0).clamp(0.0, 100.0));
    }
    None
}

/// Kimi Code accounts below this `user_level` are free accounts without a paid coding plan
/// (the official Kimi Code CLI applies the same threshold).
const KIMI_FREE_USER_LEVEL_BELOW: i64 = 30;

/// Extract the membership plan from the Kimi Code profile response (`/coding/v1/me`).
///
/// Only `user_level_name` / `user_level` are read; every other profile field (nickname, phone,
/// avatar, identifiers) is ignored and never stored. Returns the plan name as reported (e.g.
/// `"Allegretto"`), `"Free"` for free accounts, or `None` when the payload carries no level.
pub fn parse_kimi_user_level(body: &Value) -> Option<String> {
    let profile = body.get("data").filter(|d| d.is_object()).unwrap_or(body);
    let name = profile
        .get("user_level_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(name) = name {
        return Some(name.to_string());
    }
    let level = profile.get("user_level").and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_str().and_then(|s| s.trim().parse::<i64>().ok()))
    })?;
    (level < KIMI_FREE_USER_LEVEL_BELOW).then(|| "Free".to_string())
}

/// Source tag of the hidden `tier_hint` observation carrying the Kimi Code plan level read from
/// the profile endpoint. A level from this source is authoritative and overrides a manual tier
/// pick in the auto-discovered subscription.
pub const KIMI_PROFILE_HINT_SOURCE: &str = "official_profile";

/// Build the hidden `tier_hint` observation that carries a Kimi Code plan level. It is never
/// shown as a quota window; it only feeds tier and monthly-fee detection.
pub fn kimi_profile_tier_hint(
    provider_id: &str,
    level: &str,
    now_ms: i64,
) -> ObservedRateLimitWindow {
    ObservedRateLimitWindow {
        provider_id: provider_id.to_string(),
        window_name: "tier_hint".to_string(),
        used_percent: 0.0,
        window_minutes: None,
        resets_at_ms: None,
        observed_at_ms: now_ms,
        source: Some(KIMI_PROFILE_HINT_SOURCE.to_string()),
        plan_hint: Some(level.to_string()),
    }
}

/// Parse Kimi Coding Plan (`/coding/v1/usages`) usage response into a balance snapshot and rate-limit windows.
pub fn parse_kimi_coding_usages(
    body: &Value,
    provider_id: &str,
    now_ms: i64,
) -> CoreResult<(ProviderBalanceSnapshot, Vec<ObservedRateLimitWindow>)> {
    let data = body.get("data").unwrap_or(body);
    let plan_hint = body
        .pointer("/user/membership/level")
        .or_else(|| data.pointer("/user/membership/level"))
        .or_else(|| data.pointer("/membership/level"))
        .or_else(|| data.get("plan"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    let mut windows = Vec::new();
    let mut primary_used_count: Option<i64> = None;
    let mut primary_limit_count: Option<i64> = None;
    let mut weekly_used_count: Option<i64> = None;
    let mut weekly_limit_count: Option<i64> = None;

    if let Some(usage_obj) = data.get("usage").or_else(|| body.get("usage")) {
        let limit = json_number_as_f64(usage_obj.get("limit"));
        let used = json_number_as_f64(usage_obj.get("used")).or_else(|| {
            match (limit, json_number_as_f64(usage_obj.get("remaining"))) {
                (Some(l), Some(r)) => Some((l - r).max(0.0)),
                _ => None,
            }
        });
        if let (Some(u), Some(l)) = (used, limit) {
            weekly_used_count = Some(u.round() as i64);
            weekly_limit_count = Some(l.round() as i64);
        }
        if let Some(used_percent) = compute_usage_percent_from_obj(usage_obj) {
            let resets_at_ms = parse_timestamp_ms_json(
                usage_obj
                    .get("resetTime")
                    .or_else(|| usage_obj.get("reset_time"))
                    .or_else(|| usage_obj.get("resetAt"))
                    .or_else(|| usage_obj.get("resets_at")),
            );
            let unit_str = usage_obj
                .pointer("/window/timeUnit")
                .or_else(|| usage_obj.pointer("/window/time_unit"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_uppercase();
            let raw_dur = json_number_as_f64(usage_obj.pointer("/window/duration"))
                .map(|d| d.round() as i64)
                .unwrap_or(0);
            let is_monthly = unit_str.contains("MONTH")
                || (unit_str.contains("DAY") && raw_dur >= 25)
                || resets_at_ms.is_some_and(|r| r.saturating_sub(now_ms) > 8 * 86_400_000);
            let (win_name, win_mins) = if is_monthly {
                ("30d".to_string(), 43_200)
            } else {
                ("7d".to_string(), 10_080)
            };
            windows.push(ObservedRateLimitWindow {
                provider_id: provider_id.to_string(),
                window_name: win_name,
                used_percent,
                window_minutes: Some(win_mins),
                resets_at_ms,
                observed_at_ms: now_ms,
                source: Some("official_api".to_string()),
                plan_hint: plan_hint.clone(),
            });
        }
    }

    if let Some(monthly_obj) = data
        .get("monthly_usage")
        .or_else(|| body.get("monthly_usage"))
        .or_else(|| data.get("monthlyUsage"))
        .or_else(|| body.get("monthlyUsage"))
    {
        if let Some(used_percent) = compute_usage_percent_from_obj(monthly_obj) {
            let resets_at_ms = parse_timestamp_ms_json(
                monthly_obj
                    .get("resetTime")
                    .or_else(|| monthly_obj.get("reset_time"))
                    .or_else(|| monthly_obj.get("resetAt"))
                    .or_else(|| monthly_obj.get("resets_at")),
            );
            windows.push(ObservedRateLimitWindow {
                provider_id: provider_id.to_string(),
                window_name: "30d".to_string(),
                used_percent,
                window_minutes: Some(43_200),
                resets_at_ms,
                observed_at_ms: now_ms,
                source: Some("official_api".to_string()),
                plan_hint: plan_hint.clone(),
            });
        }
    }

    if let Some(usages_obj) = data
        .get("usages")
        .or_else(|| body.get("usages"))
        .and_then(|v| v.as_object())
    {
        let map_entries: &[(&[&str], &str, i64)] = &[
            (&["limit_5h", "5h", "primary"], "5h", 300),
            (&["limit_7d", "limit_week", "7d", "weekly"], "7d", 10_080),
            (
                &[
                    "limit_month_total",
                    "limit_month_code",
                    "limit_month",
                    "limit_30d",
                    "30d",
                    "monthly",
                ],
                "30d",
                43_200,
            ),
        ];
        for (keys, win_name, win_mins) in map_entries {
            let mut best: Option<(f64, Option<i64>)> = None;
            for k in *keys {
                if let Some(sub) = usages_obj.get(*k) {
                    if let Some(pct) = compute_usage_percent_from_obj(sub) {
                        let reset_ms = parse_timestamp_ms_json(
                            sub.get("resetTime")
                                .or_else(|| sub.get("reset_time"))
                                .or_else(|| sub.get("resetAt"))
                                .or_else(|| sub.get("resets_at")),
                        );
                        if best.is_none_or(|(prev_pct, _)| pct > prev_pct) {
                            best = Some((pct, reset_ms));
                        }
                    }
                }
            }
            if let Some((used_percent, resets_at_ms)) = best {
                if !windows.iter().any(|w| w.window_name == *win_name) {
                    windows.push(ObservedRateLimitWindow {
                        provider_id: provider_id.to_string(),
                        window_name: (*win_name).to_string(),
                        used_percent,
                        window_minutes: Some(*win_mins),
                        resets_at_ms,
                        observed_at_ms: now_ms,
                        source: Some("official_api".to_string()),
                        plan_hint: plan_hint.clone(),
                    });
                }
            }
        }
    }

    if let Some(limits) = data
        .get("limits")
        .or_else(|| body.get("limits"))
        .and_then(|v| v.as_array())
    {
        for (idx, item) in limits.iter().enumerate() {
            let detail = item.get("detail").unwrap_or(item);
            let Some(used_percent) = compute_usage_percent_from_obj(detail) else {
                continue;
            };
            let raw_dur = json_number_as_f64(item.pointer("/window/duration"))
                .map(|d| d.round() as i64)
                .unwrap_or(if idx == 0 { 300 } else { 10_080 });
            let unit_str = item
                .pointer("/window/timeUnit")
                .or_else(|| item.pointer("/window/time_unit"))
                .and_then(|v| v.as_str())
                .unwrap_or("TIME_UNIT_MINUTE")
                .to_ascii_uppercase();
            let window_minutes = if unit_str.contains("MONTH") {
                raw_dur.max(1) * 43_200
            } else if unit_str.contains("HOUR") {
                raw_dur.max(1) * 60
            } else if unit_str.contains("DAY") {
                raw_dur.max(1) * 1_440
            } else if unit_str.contains("SECOND") {
                (raw_dur / 60).max(1)
            } else {
                raw_dur.max(1)
            };

            let window_name = match window_minutes {
                300 => "5h".to_string(),
                1_440 => "24h".to_string(),
                10_080 => "7d".to_string(),
                m if m >= 40_000 => "30d".to_string(),
                m if m % 60 == 0 => format!("{}h", m / 60),
                m => format!("{m}m"),
            };

            let limit = json_number_as_f64(detail.get("limit"));
            let used = json_number_as_f64(detail.get("used")).or_else(|| {
                match (limit, json_number_as_f64(detail.get("remaining"))) {
                    (Some(l), Some(r)) => Some((l - r).max(0.0)),
                    _ => None,
                }
            });
            if primary_limit_count.is_none() {
                if let (Some(u), Some(l)) = (used, limit) {
                    primary_used_count = Some(u.round() as i64);
                    primary_limit_count = Some(l.round() as i64);
                }
            }

            let resets_at_ms = parse_timestamp_ms_json(
                detail
                    .get("resetTime")
                    .or_else(|| detail.get("reset_time"))
                    .or_else(|| detail.get("resetAt"))
                    .or_else(|| detail.get("resets_at")),
            );
            windows.push(ObservedRateLimitWindow {
                provider_id: provider_id.to_string(),
                window_name,
                used_percent,
                window_minutes: Some(if window_minutes >= 40_000 {
                    43_200
                } else {
                    window_minutes
                }),
                resets_at_ms,
                observed_at_ms: now_ms,
                source: Some("official_api".to_string()),
                plan_hint: plan_hint.clone(),
            });
        }
    }

    if windows.is_empty() && primary_limit_count.is_none() && weekly_limit_count.is_none() {
        return Err(CoreError::Validation(
            "Kimi Coding usage response missing usage / limits fields".to_string(),
        ));
    }

    let snapshot = ProviderBalanceSnapshot {
        provider_id: provider_id.to_string(),
        currency: "QUOTA".to_string(),
        total_balance_micro: None,
        granted_balance_micro: None,
        topped_up_balance_micro: None,
        daily_quota_used_micro: primary_used_count.or(weekly_used_count),
        daily_quota_limit_micro: primary_limit_count.or(weekly_limit_count),
        fetched_at_ms: now_ms,
    };

    Ok((snapshot, windows))
}

/// Parse Moonshot AI (`/v1/users/me/balance`) balance response.
pub fn parse_moonshot_balance(
    body: &Value,
    provider_id: &str,
    endpoint: &str,
    now_ms: i64,
) -> CoreResult<ProviderBalanceSnapshot> {
    let data = body.get("data").unwrap_or(body);
    let available = json_amount_to_micro(data.get("available_balance"));
    let voucher = json_amount_to_micro(data.get("voucher_balance"));
    let cash = json_amount_to_micro(data.get("cash_balance"));
    if available.is_none() && voucher.is_none() && cash.is_none() {
        return Err(CoreError::Validation(
            "Moonshot balance response missing available_balance / voucher_balance / cash_balance"
                .to_string(),
        ));
    }
    let currency = if endpoint.contains("moonshot.ai") {
        "USD"
    } else {
        "CNY"
    };
    Ok(ProviderBalanceSnapshot {
        provider_id: provider_id.to_string(),
        currency: currency.to_string(),
        total_balance_micro: available,
        granted_balance_micro: voucher,
        topped_up_balance_micro: cash,
        daily_quota_used_micro: None,
        daily_quota_limit_micro: None,
        fetched_at_ms: now_ms,
    })
}

/// Parse DeepSeek (`/user/balance`) balance response.
pub fn parse_deepseek_balance(
    body: &Value,
    provider_id: &str,
    now_ms: i64,
) -> CoreResult<ProviderBalanceSnapshot> {
    let infos = body
        .get("balance_infos")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            CoreError::Validation("DeepSeek balance response missing balance_infos".to_string())
        })?;
    if infos.is_empty() {
        return Err(CoreError::Validation(
            "DeepSeek balance_infos array is empty".to_string(),
        ));
    }
    let selected = infos
        .iter()
        .find(|b| b.get("currency").and_then(|c| c.as_str()) == Some("CNY"))
        .or_else(|| {
            infos
                .iter()
                .find(|b| b.get("currency").and_then(|c| c.as_str()) == Some("USD"))
        })
        .unwrap_or(&infos[0]);

    let currency = selected
        .get("currency")
        .and_then(|c| c.as_str())
        .unwrap_or("CNY")
        .to_string();

    Ok(ProviderBalanceSnapshot {
        provider_id: provider_id.to_string(),
        currency,
        total_balance_micro: json_amount_to_micro(selected.get("total_balance")),
        granted_balance_micro: json_amount_to_micro(selected.get("granted_balance")),
        topped_up_balance_micro: json_amount_to_micro(selected.get("topped_up_balance")),
        daily_quota_used_micro: None,
        daily_quota_limit_micro: None,
        fetched_at_ms: now_ms,
    })
}

/// Parse QCode (`/api/v1/usage`) balance and daily quota window response.
pub fn parse_qcode_usage(
    body: &Value,
    provider_id: &str,
    now_ms: i64,
) -> CoreResult<(ProviderBalanceSnapshot, Vec<ObservedRateLimitWindow>)> {
    let data = body.get("data").unwrap_or(body);
    let currency = data
        .get("currency")
        .and_then(|v| v.as_str())
        .unwrap_or("USD")
        .to_string();

    let mut total_balance = json_amount_to_micro(
        data.get("remaining_balance")
            .or_else(|| data.get("available_balance"))
            .or_else(|| data.get("balance"))
            .or_else(|| data.get("remaining_quota")),
    );
    let mut daily_used = json_amount_to_micro(
        data.get("daily_used")
            .or_else(|| data.get("today_used"))
            .or_else(|| data.get("daily_usage")),
    );
    let mut daily_limit = json_amount_to_micro(
        data.get("daily_limit")
            .or_else(|| data.get("daily_quota")),
    );

    if let Some(keys) = data.get("keys").and_then(|v| v.as_array()) {
        let mut sum_rem = 0i64;
        let mut sum_used = 0i64;
        let mut sum_limit = 0i64;
        let mut has_rem = false;
        let mut has_used = false;
        let mut has_limit = false;
        for k in keys {
            if let Some(v) = json_amount_to_micro(
                k.get("remaining_quota")
                    .or_else(|| k.get("quota_remaining"))
                    .or_else(|| k.get("balance")),
            ) {
                sum_rem += v;
                has_rem = true;
            }
            if let Some(v) =
                json_amount_to_micro(k.get("daily_usage").or_else(|| k.get("daily_used")))
            {
                sum_used += v;
                has_used = true;
            }
            if let Some(v) =
                json_amount_to_micro(k.get("daily_limit").or_else(|| k.get("daily_quota")))
            {
                sum_limit += v;
                has_limit = true;
            }
        }
        if total_balance.is_none() && has_rem {
            total_balance = Some(sum_rem);
        }
        if daily_used.is_none() && has_used {
            daily_used = Some(sum_used);
        }
        if daily_limit.is_none() && has_limit {
            daily_limit = Some(sum_limit);
        }
    }

    if total_balance.is_none() && daily_used.is_none() && daily_limit.is_none() {
        return Err(CoreError::Validation(
            "QCode usage response did not contain balance or daily quota fields".to_string(),
        ));
    }

    let qcode_plan_hint = data
        .get("plan_name")
        .or_else(|| data.get("tier"))
        .or_else(|| data.get("package_name"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| "qcode_daily".to_string());

    let mut windows = Vec::new();
    if let (Some(used), Some(limit)) = (daily_used, daily_limit) {
        if limit > 0 {
            windows.push(ObservedRateLimitWindow {
                provider_id: provider_id.to_string(),
                window_name: "24h".to_string(),
                used_percent: ((used as f64 / limit as f64) * 100.0).clamp(0.0, 100.0),
                window_minutes: Some(1_440),
                resets_at_ms: None,
                observed_at_ms: now_ms,
                source: Some("official_api".to_string()),
                plan_hint: Some(qcode_plan_hint),
            });
        }
    }

    Ok((
        ProviderBalanceSnapshot {
            provider_id: provider_id.to_string(),
            currency,
            total_balance_micro: total_balance,
            granted_balance_micro: None,
            topped_up_balance_micro: None,
            daily_quota_used_micro: daily_used,
            daily_quota_limit_micro: daily_limit,
            fetched_at_ms: now_ms,
        },
        windows,
    ))
}

/// Parse MiniMax Coding Plan (`/v1/token_plan/remains`) remains response.
pub fn parse_minimax_remains(
    body: &Value,
    provider_id: &str,
    now_ms: i64,
) -> CoreResult<(ProviderBalanceSnapshot, Vec<ObservedRateLimitWindow>)> {
    if let Some(code) = body
        .pointer("/base_resp/status_code")
        .and_then(|v| v.as_i64())
    {
        if code != 0 {
            let msg = body
                .pointer("/base_resp/status_msg")
                .and_then(|v| v.as_str())
                .unwrap_or("MiniMax API error");
            return Err(CoreError::Validation(format!(
                "MiniMax API returned status_code {code}: {msg}"
            )));
        }
    }

    let data = body.get("data").unwrap_or(body);
    let total_balance = json_amount_to_micro(
        data.get("available_amount")
            .or_else(|| data.get("available_balance"))
            .or_else(|| data.get("balance")),
    );

    let mut windows = Vec::new();
    let mut sum_used_count = 0i64;
    let mut sum_total_count = 0i64;

    if let Some(remains) = data.get("model_remains").and_then(|v| v.as_array()) {
        for item in remains {
            let model_name = item
                .get("model_name")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("MiniMax Coding Plan");
            let total_count = item
                .get("current_interval_total_count")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .max(0);
            if total_count == 0 {
                continue;
            }
            let used_count = if let Some(u) = item
                .get("current_interval_used_count")
                .and_then(|v| v.as_i64())
            {
                u.clamp(0, total_count)
            } else if let Some(rem) = item
                .get("current_interval_remains_count")
                .and_then(|v| v.as_i64())
            {
                total_count.saturating_sub(rem.max(0))
            } else {
                item.get("current_interval_usage_count")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
                    .clamp(0, total_count)
            };

            let used_percent = ((used_count as f64 / total_count as f64) * 100.0).clamp(0.0, 100.0);
            let resets_at_ms = item
                .get("end_time")
                .and_then(|v| v.as_i64())
                .map(|t| if t.abs() < 10_000_000_000 { t * 1000 } else { t })
                .or_else(|| {
                    item.get("remains_time")
                        .and_then(|v| v.as_i64())
                        .filter(|&r| r > 0)
                        .map(|r| now_ms + if r < 1_000_000 { r * 1000 } else { r })
                });

            sum_used_count += used_count;
            sum_total_count += total_count;

            let minimax_plan_hint = item
                .get("plan_name")
                .or_else(|| item.get("combo_name"))
                .or_else(|| data.get("plan_name"))
                .or_else(|| data.get("combo_name"))
                .or_else(|| data.get("package_name"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| "minimax_coding_plan".to_string());

            windows.push(ObservedRateLimitWindow {
                provider_id: provider_id.to_string(),
                window_name: format!("5h · {model_name}"),
                used_percent,
                window_minutes: Some(300),
                resets_at_ms,
                observed_at_ms: now_ms,
                source: Some("official_api".to_string()),
                plan_hint: Some(minimax_plan_hint),
            });
        }
    }

    if windows.is_empty() && total_balance.is_none() {
        return Err(CoreError::Validation(
            "MiniMax remains response missing model_remains and balance fields".to_string(),
        ));
    }

    let snapshot = ProviderBalanceSnapshot {
        provider_id: provider_id.to_string(),
        currency: data
            .get("currency")
            .and_then(|v| v.as_str())
            .unwrap_or("USD")
            .to_string(),
        total_balance_micro: total_balance,
        granted_balance_micro: None,
        topped_up_balance_micro: None,
        daily_quota_used_micro: (sum_total_count > 0).then_some(sum_used_count),
        daily_quota_limit_micro: (sum_total_count > 0).then_some(sum_total_count),
        fetched_at_ms: now_ms,
    };

    Ok((snapshot, windows))
}

/// Dispatch official REST balance/quota JSON payload parsing by `provider_id` and `endpoint`.
pub fn parse_official_balance_payload(
    provider_id: &str,
    endpoint: &str,
    body: &Value,
    now_ms: i64,
) -> CoreResult<(ProviderBalanceSnapshot, Vec<ObservedRateLimitWindow>)> {
    let trimmed_pid = provider_id.trim();
    match trimmed_pid {
        provider_ids::KIMI_CODE => {
            if endpoint.contains("/usages")
                || body.get("usage").is_some()
                || body.get("limits").is_some()
                || body.get("usages").is_some()
            {
                parse_kimi_coding_usages(body, trimmed_pid, now_ms)
            } else {
                Ok((
                    parse_moonshot_balance(body, trimmed_pid, endpoint, now_ms)?,
                    Vec::new(),
                ))
            }
        }
        provider_ids::DEEPSEEK_HARNESS => Ok((
            parse_deepseek_balance(body, trimmed_pid, now_ms)?,
            Vec::new(),
        )),
        provider_ids::QCODE => parse_qcode_usage(body, trimmed_pid, now_ms),
        provider_ids::MINIMAX_CODE => parse_minimax_remains(body, trimmed_pid, now_ms),
        other => Err(CoreError::Validation(format!(
            "Provider '{other}' does not expose a documented public REST balance/quota API for simple Bearer keys"
        ))),
    }
}
