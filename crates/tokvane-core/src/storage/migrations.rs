use chrono::Utc;
use rusqlite::{params, Connection};

use crate::domain::{
    provider_ids, AppPreferences, CollectionMode, QuotaUnit, SubscriptionPlan,
    SubscriptionWindowRule,
};
use crate::error::CoreResult;
use crate::pricing::default_pricing_catalog;
use crate::storage::repository::{rebuild_rollup_from_events, DAY_MS, HOUR_MS};

pub const CURRENT_SCHEMA_VERSION: i64 = 15;

const SCHEMA_V1_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS devices (
    device_id TEXT PRIMARY KEY,
    device_name TEXT NOT NULL,
    platform TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS provider_configs (
    provider_id TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL,
    collection_mode TEXT NOT NULL,
    custom_log_paths_json TEXT NOT NULL,
    has_credential INTEGER NOT NULL,
    last_sync_at_ms INTEGER,
    last_error TEXT,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS ingestion_cursors (
    source_path TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    file_identity TEXT NOT NULL,
    byte_offset INTEGER NOT NULL,
    last_line_hash TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS usage_events (
    event_id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    project_label TEXT NOT NULL,
    model_id TEXT NOT NULL,
    timestamp_ms INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_write_tokens INTEGER NOT NULL,
    reasoning_tokens INTEGER NOT NULL,
    observable_only INTEGER NOT NULL DEFAULT 0,
    duration_ms INTEGER,
    ttft_ms INTEGER,
    output_tps REAL,
    estimated_cost_micro_usd INTEGER NOT NULL,
    cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (device_id) REFERENCES devices(device_id)
);

CREATE INDEX IF NOT EXISTS idx_usage_events_ts ON usage_events(timestamp_ms);
CREATE INDEX IF NOT EXISTS idx_usage_events_provider_ts ON usage_events(provider_id, timestamp_ms);
CREATE INDEX IF NOT EXISTS idx_usage_events_model_ts ON usage_events(model_id, timestamp_ms);
CREATE INDEX IF NOT EXISTS idx_usage_events_device_ts ON usage_events(device_id, timestamp_ms);

CREATE TABLE IF NOT EXISTS usage_hourly_rollups (
    device_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    bucket_start_ms INTEGER NOT NULL,
    request_count INTEGER NOT NULL,
    observable_step_count INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_write_tokens INTEGER NOT NULL,
    reasoning_tokens INTEGER NOT NULL,
    estimated_cost_micro_usd INTEGER NOT NULL,
    cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0,
    total_duration_ms INTEGER NOT NULL,
    tps_sample_count INTEGER NOT NULL,
    tps_sum REAL NOT NULL,
    max_output_tps REAL NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (device_id, provider_id, model_id, bucket_start_ms)
);

CREATE INDEX IF NOT EXISTS idx_hourly_rollups_bucket ON usage_hourly_rollups(bucket_start_ms);
CREATE INDEX IF NOT EXISTS idx_hourly_rollups_updated ON usage_hourly_rollups(updated_at_ms);

CREATE TABLE IF NOT EXISTS usage_daily_rollups (
    device_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    bucket_start_ms INTEGER NOT NULL,
    request_count INTEGER NOT NULL,
    observable_step_count INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_write_tokens INTEGER NOT NULL,
    reasoning_tokens INTEGER NOT NULL,
    estimated_cost_micro_usd INTEGER NOT NULL,
    cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0,
    total_duration_ms INTEGER NOT NULL,
    tps_sample_count INTEGER NOT NULL,
    tps_sum REAL NOT NULL,
    max_output_tps REAL NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (device_id, provider_id, model_id, bucket_start_ms)
);

CREATE INDEX IF NOT EXISTS idx_daily_rollups_bucket ON usage_daily_rollups(bucket_start_ms);

CREATE TABLE IF NOT EXISTS model_pricing (
    model_pattern TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    input_per_m_micro_usd INTEGER NOT NULL,
    output_per_m_micro_usd INTEGER NOT NULL,
    cache_read_per_m_micro_usd INTEGER NOT NULL,
    cache_write_per_m_micro_usd INTEGER NOT NULL,
    reasoning_per_m_micro_usd INTEGER NOT NULL,
    is_user_override INTEGER NOT NULL,
    effective_from TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS subscription_plans (
    plan_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    plan_name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    monthly_fee_micro_usd INTEGER NOT NULL,
    windows_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS observed_rate_limits (
    provider_id TEXT NOT NULL,
    window_name TEXT NOT NULL,
    used_percent REAL NOT NULL,
    window_minutes INTEGER,
    resets_at_ms INTEGER,
    observed_at_ms INTEGER NOT NULL,
    PRIMARY KEY (provider_id, window_name)
);

CREATE TABLE IF NOT EXISTS app_preferences (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    currency TEXT NOT NULL,
    usd_to_cny_rate TEXT NOT NULL,
    theme TEXT NOT NULL,
    minimize_to_tray INTEGER NOT NULL,
    launch_at_startup INTEGER NOT NULL,
    otlp_enabled INTEGER NOT NULL,
    otlp_port INTEGER NOT NULL,
    device_display_name TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
"#;

/// Run database schema migrations and seed required baseline defaults idempotently.
pub fn run_migrations(conn: &mut Connection) -> CoreResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, applied_at_ms INTEGER NOT NULL);",
    )?;

    let current_version: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;

    let now_ms = Utc::now().timestamp_millis();

    if current_version < 1 {
        let tx = conn.transaction()?;
        tx.execute_batch(SCHEMA_V1_SQL)?;
        tx.execute(
            "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
            params![1i64, now_ms],
        )?;
        tx.commit()?;
    }

    if current_version < 2 {
        migrate_v1_to_v2(conn, now_ms)?;
    }

    if current_version < 3 {
        migrate_v2_to_v3(conn, now_ms)?;
    }

    if current_version < 4 {
        migrate_v3_to_v4(conn, now_ms)?;
    }

    if current_version < 5 {
        migrate_v4_to_v5(conn, now_ms)?;
    }

    if current_version < 6 {
        migrate_v5_to_v6(conn, now_ms)?;
    }

    if current_version < 7 {
        migrate_v6_to_v7(conn, now_ms)?;
    }

    if current_version < 8 {
        migrate_v7_to_v8(conn, now_ms)?;
    }

    if current_version < 9 {
        migrate_v8_to_v9(conn, now_ms)?;
    }

    if current_version < 10 {
        migrate_v9_to_v10(conn, now_ms)?;
    }

    if current_version < 11 {
        migrate_v10_to_v11(conn, now_ms)?;
    }

    if current_version < 12 {
        migrate_v11_to_v12(conn, now_ms)?;
    }

    if current_version < 13 {
        migrate_v12_to_v13(conn, now_ms)?;
    }

    if current_version < 14 {
        migrate_v13_to_v14(conn, now_ms)?;
    }

    if current_version < 15 {
        migrate_v14_to_v15(conn, now_ms)?;
    }

    seed_defaults(conn, now_ms)?;
    Ok(())
}

fn legacy_default_windows(limit_5h: i64, limit_7d: i64) -> Vec<SubscriptionWindowRule> {
    vec![
        SubscriptionWindowRule {
            window_id: "5h".to_string(),
            label: "5h 滚动窗口".to_string(),
            duration_hours: 5,
            rolling: true,
            unit: QuotaUnit::MicroUsd,
            limit_value: limit_5h,
            warning_percent: 80,
            critical_percent: 95,
        },
        SubscriptionWindowRule {
            window_id: "7d".to_string(),
            label: "7d 周额度".to_string(),
            duration_hours: 168,
            rolling: true,
            unit: QuotaUnit::MicroUsd,
            limit_value: limit_7d,
            warning_percent: 80,
            critical_percent: 95,
        },
    ]
}

fn legacy_v1_default_subscription_plans() -> Vec<SubscriptionPlan> {
    vec![
        SubscriptionPlan {
            plan_id: "claude_pro".to_string(),
            provider_id: provider_ids::CLAUDE_CODE.to_string(),
            plan_name: "Claude Pro ($20/mo)".to_string(),
            enabled: true,
            monthly_fee_micro_usd: 20_000_000,
            windows: legacy_default_windows(15_000_000, 75_000_000),
            is_user_configured: false,
            auto_source: None,
            updated_at_ms: 0,
        },
        SubscriptionPlan {
            plan_id: "claude_max_5x".to_string(),
            provider_id: provider_ids::CLAUDE_CODE.to_string(),
            plan_name: "Claude Max 5x ($100/mo)".to_string(),
            enabled: false,
            monthly_fee_micro_usd: 100_000_000,
            windows: legacy_default_windows(75_000_000, 375_000_000),
            is_user_configured: false,
            auto_source: None,
            updated_at_ms: 0,
        },
        SubscriptionPlan {
            plan_id: "claude_max_20x".to_string(),
            provider_id: provider_ids::CLAUDE_CODE.to_string(),
            plan_name: "Claude Max 20x ($200/mo)".to_string(),
            enabled: false,
            monthly_fee_micro_usd: 200_000_000,
            windows: legacy_default_windows(200_000_000, 1_200_000_000),
            is_user_configured: false,
            auto_source: None,
            updated_at_ms: 0,
        },
        SubscriptionPlan {
            plan_id: "codex_plus".to_string(),
            provider_id: provider_ids::CODEX.to_string(),
            plan_name: "ChatGPT Plus / Codex ($20/mo)".to_string(),
            enabled: true,
            monthly_fee_micro_usd: 20_000_000,
            windows: legacy_default_windows(18_000_000, 90_000_000),
            is_user_configured: false,
            auto_source: None,
            updated_at_ms: 0,
        },
        SubscriptionPlan {
            plan_id: "codex_pro".to_string(),
            provider_id: provider_ids::CODEX.to_string(),
            plan_name: "ChatGPT Pro / Codex ($200/mo)".to_string(),
            enabled: false,
            monthly_fee_micro_usd: 200_000_000,
            windows: legacy_default_windows(150_000_000, 900_000_000),
            is_user_configured: false,
            auto_source: None,
            updated_at_ms: 0,
        },
        SubscriptionPlan {
            plan_id: "antigravity_ai_pro".to_string(),
            provider_id: provider_ids::ANTIGRAVITY.to_string(),
            plan_name: "Google AI Pro / Antigravity ($19.99/mo)".to_string(),
            enabled: true,
            monthly_fee_micro_usd: 19_990_000,
            windows: legacy_default_windows(20_000_000, 100_000_000),
            is_user_configured: false,
            auto_source: None,
            updated_at_ms: 0,
        },
    ]
}

fn migrate_v1_to_v2(conn: &mut Connection, now_ms: i64) -> CoreResult<()> {
    let tx = conn.transaction()?;

    // 1. Upgrade subscription_plans: remove untouched v0.1.0 auto-seeded presets and mark any
    //    customized or user-created rows as `is_user_configured = 1`.
    tx.execute_batch(
        "ALTER TABLE subscription_plans ADD COLUMN is_user_configured INTEGER NOT NULL DEFAULT 0;",
    )?;

    let min_device_created_ms: i64 = tx.query_row(
        "SELECT COALESCE(MIN(created_at_ms), 0) FROM devices",
        [],
        |row| row.get(0),
    )?;

    let legacy_defaults = legacy_v1_default_subscription_plans();
    let mut untouched_plan_ids: Vec<String> = Vec::new();
    {
        let mut stmt = tx.prepare(
            r#"
            SELECT plan_id, provider_id, plan_name, enabled, monthly_fee_micro_usd, windows_json, updated_at_ms
            FROM subscription_plans
            "#,
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)? != 0,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?;

        for row in rows {
            let (plan_id, provider_id, plan_name, enabled, fee, windows_json, updated_at_ms) =
                row?;
            let within_seed_window = (updated_at_ms - min_device_created_ms).abs() <= 5_000;
            if !within_seed_window {
                continue;
            }
            let Ok(parsed_windows) = serde_json::from_str::<Vec<SubscriptionWindowRule>>(&windows_json)
            else {
                continue;
            };
            let matches_untouched_default = legacy_defaults.iter().any(|def| {
                def.plan_id == plan_id
                    && def.provider_id == provider_id
                    && def.plan_name == plan_name
                    && def.enabled == enabled
                    && def.monthly_fee_micro_usd == fee
                    && def.windows == parsed_windows
            });
            if matches_untouched_default {
                untouched_plan_ids.push(plan_id);
            }
        }
    }

    for plan_id in &untouched_plan_ids {
        tx.execute(
            "DELETE FROM subscription_plans WHERE plan_id = ?1",
            params![plan_id],
        )?;
    }
    tx.execute("UPDATE subscription_plans SET is_user_configured = 1", [])?;

    // 2. Extend app_preferences with custom background configuration columns.
    tx.execute_batch(
        r#"
        ALTER TABLE app_preferences ADD COLUMN bg_mode TEXT NOT NULL DEFAULT 'none';
        ALTER TABLE app_preferences ADD COLUMN bg_image_path TEXT;
        ALTER TABLE app_preferences ADD COLUMN bg_preset_id TEXT NOT NULL DEFAULT 'aurora_slate';
        ALTER TABLE app_preferences ADD COLUMN bg_opacity REAL NOT NULL DEFAULT 0.18;
        ALTER TABLE app_preferences ADD COLUMN bg_blur_px INTEGER NOT NULL DEFAULT 16;
        "#,
    )?;

    // 3. Clean historical distorted TPS outliers and atomically rebuild hourly/daily rollups.
    tx.execute(
        r#"
        UPDATE usage_events
        SET output_tps = NULL
        WHERE output_tps IS NOT NULL
          AND (
            (duration_ms IS NOT NULL AND duration_ms < 300)
            OR (output_tokens + reasoning_tokens) < 8
            OR output_tps > 800.0
            OR output_tps <= 0.0
          )
        "#,
        [],
    )?;

    tx.execute("DELETE FROM usage_hourly_rollups", [])?;
    tx.execute("DELETE FROM usage_daily_rollups", [])?;
    rebuild_rollup_from_events(&tx, "usage_hourly_rollups", HOUR_MS, now_ms)?;
    rebuild_rollup_from_events(&tx, "usage_daily_rollups", DAY_MS, now_ms)?;

    tx.execute(
        "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
        params![2i64, now_ms],
    )?;

    tx.commit()?;
    Ok(())
}

fn migrate_v2_to_v3(conn: &mut Connection, now_ms: i64) -> CoreResult<()> {
    let tx = conn.transaction()?;
    tx.execute_batch(
        r#"
        ALTER TABLE app_preferences ADD COLUMN auto_sync_pricing INTEGER NOT NULL DEFAULT 1;
        ALTER TABLE app_preferences ADD COLUMN exchange_rate_auto INTEGER NOT NULL DEFAULT 1;
        ALTER TABLE app_preferences ADD COLUMN last_rate_sync_at_ms INTEGER;
        ALTER TABLE app_preferences ADD COLUMN last_pricing_sync_at_ms INTEGER;
        ALTER TABLE app_preferences ADD COLUMN pricing_catalog_hash TEXT;
        "#,
    )?;
    tx.execute(
        "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
        params![3i64, now_ms],
    )?;
    tx.commit()?;
    Ok(())
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    column_def: &str,
) -> CoreResult<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let existing: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !existing.iter().any(|c| c.eq_ignore_ascii_case(column)) {
        conn.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {column_def};"
        ))?;
    }
    Ok(())
}

fn migrate_v3_to_v4(conn: &mut Connection, now_ms: i64) -> CoreResult<()> {
    let tx = conn.transaction()?;
    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS sync_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            enabled INTEGER NOT NULL DEFAULT 0,
            worker_url TEXT,
            e2ee_enabled INTEGER NOT NULL DEFAULT 0,
            e2ee_kdf_salt_hex TEXT,
            last_push_watermark_ms INTEGER NOT NULL DEFAULT 0,
            last_pull_watermark_ms INTEGER NOT NULL DEFAULT 0,
            last_sync_at_ms INTEGER,
            last_sync_error TEXT,
            updated_at_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS remote_subscription_snapshots (
            device_id TEXT NOT NULL,
            device_display_name TEXT NOT NULL,
            plan_id TEXT NOT NULL,
            window_id TEXT NOT NULL,
            provider_id TEXT NOT NULL,
            plan_name TEXT NOT NULL,
            window_label TEXT NOT NULL,
            used_percent REAL NOT NULL,
            resets_at_ms INTEGER,
            source TEXT NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            PRIMARY KEY (device_id, plan_id, window_id)
        );

        CREATE TABLE IF NOT EXISTS provider_balances (
            provider_id TEXT PRIMARY KEY,
            currency TEXT NOT NULL,
            total_balance_micro INTEGER,
            granted_balance_micro INTEGER,
            topped_up_balance_micro INTEGER,
            daily_quota_used_micro INTEGER,
            daily_quota_limit_micro INTEGER,
            fetched_at_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS custom_provider_rules (
            rule_id TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            protocol_kind TEXT NOT NULL,
            log_roots_json TEXT NOT NULL,
            mapping_spec_json TEXT NOT NULL,
            updated_at_ms INTEGER NOT NULL
        );

        UPDATE model_pricing
        SET provider_id = 'deepseek_harness'
        WHERE provider_id = 'deepseek' AND is_user_override = 0;

        DELETE FROM model_pricing
        WHERE model_pattern = '_default_deepseek' AND is_user_override = 0;
        "#,
    )?;

    add_column_if_missing(
        &tx,
        "app_preferences",
        "auto_check_updates",
        "INTEGER NOT NULL DEFAULT 1",
    )?;

    tx.execute(
        "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
        params![4i64, now_ms],
    )?;
    tx.commit()?;
    Ok(())
}

fn migrate_v4_to_v5(conn: &mut Connection, now_ms: i64) -> CoreResult<()> {
    let tx = conn.transaction()?;

    add_column_if_missing(&tx, "subscription_plans", "auto_source", "TEXT")?;
    add_column_if_missing(
        &tx,
        "subscription_plans",
        "user_customized",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        &tx,
        "subscription_plans",
        "user_dismissed",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(&tx, "observed_rate_limits", "source", "TEXT")?;
    add_column_if_missing(&tx, "observed_rate_limits", "plan_hint", "TEXT")?;

    tx.execute(
        "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
        params![5i64, now_ms],
    )?;
    tx.commit()?;

    // Backfill auto-discovered subscriptions for any existing official observed rate-limit snapshots.
    let existing_observed = crate::storage::repository::load_observed_rate_limits(conn)?;
    let mut providers: Vec<String> = existing_observed
        .iter()
        .map(|o| o.provider_id.clone())
        .collect();
    providers.sort();
    providers.dedup();
    for pid in providers {
        crate::storage::repository::sync_auto_subscription_for_provider_conn(conn, &pid, now_ms)?;
    }
    Ok(())
}

fn migrate_v5_to_v6(conn: &mut Connection, now_ms: i64) -> CoreResult<()> {
    let tx = conn.transaction()?;

    add_column_if_missing(
        &tx,
        "app_preferences",
        "floating_window_enabled",
        "INTEGER NOT NULL DEFAULT 0",
    )?;

    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS session_titles (
            session_key TEXT PRIMARY KEY,
            provider_id TEXT NOT NULL,
            title TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'derived',
            updated_at_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_session_titles_updated
            ON session_titles (updated_at_ms DESC);
        "#,
    )?;

    // Clean historical TPS outliers (<12 tokens, <500ms, missing duration, or >450 tok/s)
    // and rebuild local-device hourly/daily rollups without disturbing synced remote devices.
    tx.execute(
        r#"
        UPDATE usage_events
        SET output_tps = NULL
        WHERE output_tps IS NOT NULL
          AND (
            (output_tokens + reasoning_tokens) < ?1
            OR duration_ms IS NULL
            OR duration_ms < ?2
            OR output_tps <= 0.0
            OR output_tps > ?3
          )
        "#,
        params![
            crate::domain::MIN_VALID_TPS_OUTPUT_TOKENS,
            crate::domain::MIN_VALID_TPS_DURATION_MS,
            crate::domain::MAX_VALID_OUTPUT_TPS,
        ],
    )?;

    crate::storage::repository::rebuild_local_rollups_conn(&tx, now_ms)?;

    tx.execute(
        "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
        params![6i64, now_ms],
    )?;
    tx.commit()?;
    Ok(())
}

fn migrate_v6_to_v7(conn: &mut Connection, now_ms: i64) -> CoreResult<()> {
    let tx = conn.transaction()?;

    // Clean historical Antigravity TPS entries where 1s-quantized USER_INPUT -> PLANNER_RESPONSE
    // start deltas (<= 2000ms for >= 80 tokens) distorted speed before GENERIC step-end tracking.
    tx.execute(
        r#"
        UPDATE usage_events
        SET output_tps = NULL,
            duration_ms = NULL
        WHERE provider_id = 'antigravity'
           AND output_tps IS NOT NULL
           AND duration_ms IS NOT NULL
           AND duration_ms <= 2000
           AND (output_tokens + reasoning_tokens) >= 80
        "#,
        [],
    )?;

    // Purge stale/injected AGENTS.md session titles and reset codex/kimi_code cursors so their
    // session titles are cleanly backfilled (with strict event_id deduplication) on next scan.
    tx.execute_batch(
        r#"
        DELETE FROM session_titles
        WHERE provider_id IN ('codex', 'kimi_code')
           OR title LIKE '%工作偏好%'
           OR title LIKE '%AGENTS.md%'
           OR title LIKE '%<INSTRUCTIONS%';
        DELETE FROM ingestion_cursors
        WHERE provider_id IN ('codex', 'kimi_code');
        "#,
    )?;

    crate::storage::repository::rebuild_local_rollups_conn(&tx, now_ms)?;
    crate::storage::repository::sync_auto_subscription_for_provider_conn(
        &tx,
        provider_ids::ANTIGRAVITY,
        now_ms,
    )?;

    tx.execute(
        "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
        params![7i64, now_ms],
    )?;
    tx.commit()?;
    Ok(())
}

fn migrate_v7_to_v8(conn: &mut Connection, now_ms: i64) -> CoreResult<()> {
    let tx = conn.transaction()?;

    // 1. Upgrade untouched Google AI Ultra ($249.99) auto plans to Google AI Ultra 20x ($199.99)
    tx.execute(
        r#"
        UPDATE subscription_plans
        SET plan_name = 'Google AI Ultra 20x',
            monthly_fee_micro_usd = 199990000,
            updated_at_ms = ?1
        WHERE provider_id = 'antigravity'
          AND plan_name = 'Google AI Ultra'
          AND monthly_fee_micro_usd = 249990000
          AND user_customized = 0
        "#,
        params![now_ms],
    )?;

    // 2. Ensure all existing Kimi Code subscription plans include the 30d monthly limit window
    let kimi_rows: Vec<(String, String, String)> = {
        let mut stmt = tx.prepare(
            "SELECT plan_id, plan_name, windows_json FROM subscription_plans WHERE provider_id = 'kimi_code'",
        )?;
        let mapped = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        mapped.collect::<Result<Vec<_>, _>>()?
    };

    for (plan_id, plan_name, windows_json) in kimi_rows {
        let Ok(mut windows) = serde_json::from_str::<Vec<SubscriptionWindowRule>>(&windows_json)
        else {
            continue;
        };
        if !windows.iter().any(|w| w.window_id == "30d") {
            let lower_name = plan_name.to_ascii_lowercase();
            let monthly_limit = if lower_name.contains("vivace") || lower_name.contains("ultra") {
                2_800_000_000
            } else if lower_name.contains("allegro") || lower_name.contains("max") {
                1_200_000_000
            } else if lower_name.contains("allegretto") || lower_name.contains("pro") {
                400_000_000
            } else {
                120_000_000
            };
            windows.push(SubscriptionWindowRule {
                window_id: "30d".to_string(),
                label: "30d 月度限额".to_string(),
                duration_hours: 720,
                rolling: false,
                unit: QuotaUnit::MicroUsd,
                limit_value: monthly_limit,
                warning_percent: 80,
                critical_percent: 95,
            });
            windows.sort_by_key(|w| w.duration_hours);
            let next_json = serde_json::to_string(&windows)?;
            tx.execute(
                "UPDATE subscription_plans SET windows_json = ?2, updated_at_ms = ?3 WHERE plan_id = ?1",
                params![plan_id, next_json, now_ms],
            )?;
        }
    }

    for pid in [provider_ids::ANTIGRAVITY, provider_ids::KIMI_CODE] {
        crate::storage::repository::sync_auto_subscription_for_provider_conn(&tx, pid, now_ms)?;
    }

    tx.execute(
        "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
        params![8i64, now_ms],
    )?;
    tx.commit()?;
    Ok(())
}

fn migrate_v8_to_v9(conn: &mut Connection, now_ms: i64) -> CoreResult<()> {
    let tx = conn.transaction()?;

    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS payg_accounts (
            account_id TEXT PRIMARY KEY,
            provider_id TEXT NOT NULL,
            account_name TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            manual_balance_micro_usd INTEGER,
            balance_anchor_ms INTEGER,
            monthly_budget_micro_usd INTEGER,
            is_auto INTEGER NOT NULL DEFAULT 0,
            user_dismissed INTEGER NOT NULL DEFAULT 0,
            updated_at_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_payg_accounts_provider
            ON payg_accounts (provider_id, user_dismissed);

        DELETE FROM observed_rate_limits
        WHERE provider_id = 'antigravity'
          AND window_name = 'tier_hint'
          AND LOWER(COALESCE(plan_hint, '')) = 'pro';

        DELETE FROM subscription_plans
        WHERE plan_id = 'auto_antigravity'
          AND user_customized = 0
          AND user_dismissed = 0
          AND plan_name = 'Google AI Pro';

        DELETE FROM session_titles
        WHERE (provider_id = 'antigravity' AND source != 'explicit')
           OR (provider_id = 'codex' AND (
               source != 'explicit'
               OR LOWER(title) LIKE 'the following is the codex agent history%'
               OR LOWER(title) LIKE 'the following is a summary of%'
               OR LOWER(title) LIKE 'another language model started%'
               OR LOWER(title) LIKE '[plan review%'
           ));

        DELETE FROM ingestion_cursors
        WHERE provider_id IN ('codex', 'antigravity');
        "#,
    )?;

    tx.execute(
        r#"
        UPDATE subscription_plans
        SET plan_name = 'Google AI Ultra',
            updated_at_ms = ?1
        WHERE plan_id = 'auto_antigravity'
          AND user_customized = 0
          AND plan_name = 'Google AI Ultra 20x'
        "#,
        params![now_ms],
    )?;

    tx.execute(
        "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
        params![9i64, now_ms],
    )?;
    tx.commit()?;
    Ok(())
}

fn migrate_v9_to_v10(conn: &mut Connection, now_ms: i64) -> CoreResult<()> {
    let tx = conn.transaction()?;

    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS subscription_catalog (
            template_id TEXT PRIMARY KEY,
            provider_id TEXT NOT NULL,
            plan_name TEXT NOT NULL,
            monthly_fee_micro_usd INTEGER NOT NULL,
            windows_json TEXT NOT NULL,
            description TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            updated_at_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_subscription_catalog_order
            ON subscription_catalog (sort_order ASC, template_id ASC);
        "#,
    )?;

    add_column_if_missing(
        &tx,
        "app_preferences",
        "last_subscription_sync_at_ms",
        "INTEGER",
    )?;
    add_column_if_missing(
        &tx,
        "app_preferences",
        "subscription_catalog_hash",
        "TEXT",
    )?;

    // Remove bogus `gpt-5-codex` rows fabricated by previous hardcoded fallback and reset Codex
    // ingestion cursors so startup scan cleanly re-ingests real turn_context models.
    tx.execute_batch(
        r#"
        DELETE FROM usage_events
        WHERE provider_id = 'codex' AND model_id = 'gpt-5-codex';

        DELETE FROM ingestion_cursors
        WHERE provider_id = 'codex';
        "#,
    )?;

    crate::storage::repository::rebuild_local_rollups_conn(&tx, now_ms)?;

    // Mark historical Codex Guardian / auto-review titles as background sessions so they do not
    // appear under Active Sessions.
    tx.execute(
        r#"
        UPDATE session_titles
        SET source = 'background',
            updated_at_ms = ?1
        WHERE provider_id = 'codex'
          AND LOWER(TRIM(title)) IN ('guardian review', 'codex-auto-review', 'auto-review', 'auto review')
        "#,
        params![now_ms],
    )?;

    // Abolish unverified local estimated windows on auto-discovered Antigravity plans.
    tx.execute(
        r#"
        UPDATE subscription_plans
        SET windows_json = '[]',
            updated_at_ms = ?1
        WHERE plan_id = 'auto_antigravity'
          AND user_customized = 0
        "#,
        params![now_ms],
    )?;

    tx.execute(
        "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
        params![10i64, now_ms],
    )?;
    tx.commit()?;
    Ok(())
}

fn migrate_v10_to_v11(conn: &mut Connection, now_ms: i64) -> CoreResult<()> {
    let tx = conn.transaction()?;

    // Manual subscription entries and windowless local-estimation placeholders are removed once
    // unified account-only quota/balance views supersede manual plan configuration.
    tx.execute_batch(
        r#"
        DELETE FROM subscription_plans
        WHERE (auto_source IS NULL AND plan_id NOT LIKE 'auto_%')
           OR (auto_source = 'local_log' AND TRIM(windows_json) = '[]');
        "#,
    )?;

    tx.execute(
        "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
        params![11i64, now_ms],
    )?;
    tx.commit()?;
    Ok(())
}

fn migrate_v11_to_v12(conn: &mut Connection, now_ms: i64) -> CoreResult<()> {
    let tx = conn.transaction()?;

    // Clean historical Antigravity events poisoned by `GENERIC`/`<CONTEXT_SUMMARY>`
    // `<USER_SETTINGS_CHANGE>` fixture strings (`Gemini 3.1 Pro`), remove the obsolete built-in
    // pricing row, and reset Antigravity cursors so the hardened parser re-attributes real models.
    tx.execute_batch(
        r#"
        UPDATE usage_events
        SET model_id = 'antigravity-default'
        WHERE provider_id = 'antigravity'
          AND (LOWER(model_id) LIKE '%gemini 3.1%' OR LOWER(model_id) LIKE '%gemini-3.1%');

        DELETE FROM model_pricing
        WHERE provider_id = 'antigravity'
          AND is_user_override = 0
          AND LOWER(model_pattern) LIKE '%gemini-3.1%';

        DELETE FROM ingestion_cursors
        WHERE provider_id = 'antigravity';
        "#,
    )?;

    crate::storage::repository::rebuild_local_rollups_conn(&tx, now_ms)?;

    tx.execute(
        "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
        params![12i64, now_ms],
    )?;
    tx.commit()?;
    Ok(())
}

fn migrate_v12_to_v13(conn: &mut Connection, now_ms: i64) -> CoreResult<()> {
    let tx = conn.transaction()?;

    // Codex `output_tps` used to divide by prompt → `token_count` (end-to-end, incl. queueing,
    // prefill, TTFT and tool execution) and only sampled the first step of each turn. The collector
    // now measures the decode window (first streamed item → `token_usage_record`) for every step.
    // Clear the old local timing fields and reset Codex cursors so the rescan re-fills them; Codex
    // event ids are derived from cumulative token totals, so the rescan updates rows in place.
    tx.execute_batch(
        r#"
        UPDATE usage_events
        SET duration_ms = NULL, ttft_ms = NULL, output_tps = NULL
        WHERE provider_id = 'codex'
          AND device_id = (SELECT device_id FROM devices ORDER BY created_at_ms ASC LIMIT 1);

        DELETE FROM ingestion_cursors
        WHERE provider_id = 'codex';
        "#,
    )?;

    crate::storage::repository::rebuild_local_rollups_conn(&tx, now_ms)?;

    tx.execute(
        "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
        params![13i64, now_ms],
    )?;
    tx.commit()?;
    Ok(())
}

fn migrate_v13_to_v14(conn: &mut Connection, now_ms: i64) -> CoreResult<()> {
    let tx = conn.transaction()?;

    add_column_if_missing(
        &tx,
        "remote_subscription_snapshots",
        "monthly_fee_micro_usd",
        "INTEGER NOT NULL DEFAULT 0",
    )?;

    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS remote_payg_snapshots (
            device_id TEXT NOT NULL,
            device_display_name TEXT NOT NULL,
            account_id TEXT NOT NULL,
            provider_id TEXT NOT NULL,
            account_name TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            currency TEXT,
            total_balance_micro INTEGER,
            granted_balance_micro INTEGER,
            topped_up_balance_micro INTEGER,
            remaining_manual_balance_micro_usd INTEGER,
            monthly_budget_micro_usd INTEGER,
            today_cost_micro_usd INTEGER NOT NULL DEFAULT 0,
            month_cost_micro_usd INTEGER NOT NULL DEFAULT 0,
            fetched_at_ms INTEGER,
            updated_at_ms INTEGER NOT NULL,
            PRIMARY KEY (device_id, account_id)
        );
        CREATE INDEX IF NOT EXISTS idx_remote_payg_provider
            ON remote_payg_snapshots (provider_id, updated_at_ms DESC);
        "#,
    )?;

    tx.execute(
        "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
        params![14i64, now_ms],
    )?;
    tx.commit()?;
    Ok(())
}

fn migrate_v14_to_v15(conn: &mut Connection, now_ms: i64) -> CoreResult<()> {
    let tx = conn.transaction()?;

    add_column_if_missing(
        &tx,
        "app_preferences",
        "background_cli_quota_poll",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    add_column_if_missing(&tx, "provider_configs", "cli_auth_paused_reason", "TEXT")?;

    tx.execute(
        "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
        params![15i64, now_ms],
    )?;
    tx.commit()?;
    Ok(())
}

fn seed_defaults(conn: &mut Connection, now_ms: i64) -> CoreResult<()> {
    let tx = conn.transaction()?;

    // 1. Ensure local device exists
    let device_count: i64 = tx.query_row("SELECT COUNT(*) FROM devices", [], |r| r.get(0))?;
    if device_count == 0 {
        let device_id = uuid::Uuid::new_v4().to_string();
        let platform = std::env::consts::OS.to_string();
        let default_name = format!("{platform}-workstation");
        tx.execute(
            "INSERT INTO devices (device_id, device_name, platform, created_at_ms) VALUES (?1, ?2, ?3, ?4)",
            params![device_id, default_name, platform, now_ms],
        )?;
    }

    // 2. Ensure default provider configs exist
    for provider_id in provider_ids::ALL {
        tx.execute(
            r#"
            INSERT OR IGNORE INTO provider_configs
                (provider_id, enabled, collection_mode, custom_log_paths_json, has_credential, last_sync_at_ms, last_error, updated_at_ms)
            VALUES (?1, 1, ?2, '[]', 0, NULL, NULL, ?3)
            "#,
            params![provider_id, CollectionMode::LocalLog.as_str(), now_ms],
        )?;
    }

    // 3. Seed built-in pricing catalog (preserves user overrides)
    for item in default_pricing_catalog() {
        tx.execute(
            r#"
            INSERT OR IGNORE INTO model_pricing
                (model_pattern, provider_id, input_per_m_micro_usd, output_per_m_micro_usd, cache_read_per_m_micro_usd, cache_write_per_m_micro_usd, reasoning_per_m_micro_usd, is_user_override, effective_from)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)
            "#,
            params![
                item.model_pattern,
                item.provider_id,
                item.input_per_m_micro_usd,
                item.output_per_m_micro_usd,
                item.cache_read_per_m_micro_usd,
                item.cache_write_per_m_micro_usd,
                item.reasoning_per_m_micro_usd,
                item.effective_from,
            ],
        )?;
    }

    // 4. Seed default preferences row (id = 1); fresh installs deliberately start with ZERO
    //    auto-seeded subscription plans.
    let prefs = AppPreferences::default();
    tx.execute(
        r#"
        INSERT OR IGNORE INTO app_preferences
            (id, currency, usd_to_cny_rate, theme, minimize_to_tray, launch_at_startup,
             otlp_enabled, otlp_port, device_display_name,
             bg_mode, bg_image_path, bg_preset_id, bg_opacity, bg_blur_px,
             auto_sync_pricing, exchange_rate_auto, last_rate_sync_at_ms, last_pricing_sync_at_ms,
             auto_check_updates, floating_window_enabled, background_cli_quota_poll, updated_at_ms)
        VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)
        "#,
        params![
            prefs.currency.as_str(),
            prefs.usd_to_cny_rate,
            prefs.theme,
            i64::from(prefs.minimize_to_tray),
            i64::from(prefs.launch_at_startup),
            i64::from(prefs.otlp_enabled),
            i64::from(prefs.otlp_port),
            prefs.device_display_name,
            prefs.bg_mode,
            prefs.bg_image_path,
            prefs.bg_preset_id,
            prefs.bg_opacity,
            prefs.bg_blur_px,
            i64::from(prefs.auto_sync_pricing),
            i64::from(prefs.exchange_rate_auto),
            prefs.last_rate_sync_at_ms,
            prefs.last_pricing_sync_at_ms,
            i64::from(prefs.auto_check_updates),
            i64::from(prefs.floating_window_enabled),
            i64::from(prefs.background_cli_quota_poll),
            now_ms,
        ],
    )?;

    // 5. Seed cloud sync state row (id = 1)
    tx.execute(
        r#"
        INSERT OR IGNORE INTO sync_state
            (id, enabled, worker_url, e2ee_enabled, e2ee_kdf_salt_hex,
             last_push_watermark_ms, last_pull_watermark_ms, last_sync_at_ms, last_sync_error, updated_at_ms)
        VALUES (1, 0, NULL, 0, NULL, 0, 0, NULL, NULL, ?1)
        "#,
        params![now_ms],
    )?;

    tx.commit()?;
    Ok(())
}
