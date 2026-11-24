use super::*;
use crate::domain::compute_event_id;
use tempfile::tempdir;

#[test]
fn wal_mode_and_device_bootstrap_work_on_file_db() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("tokenvane.db");
    let storage = Storage::open_file(&db_path).expect("open db");

    let device = storage.local_device().expect("device");
    assert!(!device.device_id.is_empty());

    storage
        .with_conn(|conn| {
            let mode: String =
                conn.query_row("PRAGMA journal_mode;", [], |row| row.get(0))?;
            assert_eq!(mode.to_ascii_lowercase(), "wal");
            Ok(())
        })
        .expect("wal check");
}

#[test]
fn ingestion_is_strictly_idempotent_across_duplicate_replays() {
    let storage = Storage::open_in_memory().expect("in-memory db");
    let device = storage.local_device().expect("device");
    let ts = 1_760_000_000_000i64;

    let ev = NormalizedUsageEvent {
        event_id: compute_event_id("claude_code", "sess-1", "msg-1", ts),
        device_id: device.device_id.clone(),
        provider_id: "claude_code".to_string(),
        session_id: "sess-1".to_string(),
        project_label: "TokenVane".to_string(),
        model_id: "claude-sonnet-4-5".to_string(),
        timestamp_ms: ts,
        input_tokens: 1_000_000,
        output_tokens: 500_000,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        reasoning_tokens: 0,
        observable_only: false,
        duration_ms: Some(2_000),
        ttft_ms: Some(350),
        output_tps: Some(250.0),
        estimated_cost_micro_usd: 0,
        cache_saved_micro_usd: 0,
    };

    let first = storage
        .ingest_events_with_cursor(std::slice::from_ref(&ev), None, &[])
        .expect("first ingest");
    assert_eq!(first.inserted, 1);
    assert_eq!(first.deduplicated, 0);

    let second = storage
        .ingest_events_with_cursor(&[ev], None, &[])
        .expect("second ingest");
    assert_eq!(second.inserted, 0);
    assert_eq!(second.deduplicated, 1);

    storage
        .with_conn(|conn| {
            let (req_count, cost): (i64, i64) = conn.query_row(
                "SELECT SUM(request_count), SUM(estimated_cost_micro_usd) FROM usage_hourly_rollups",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            assert_eq!(req_count, 1);
            // 1M input ($3) + 0.5M output ($7.5) = $10.50 = 10_500_000 micro-USD
            assert_eq!(cost, 10_500_000);
            Ok(())
        })
        .expect("rollup counts verified");
}

#[test]
fn sync_envelope_v1_contains_only_desensitized_aggregates() {
    let storage = Storage::open_in_memory().expect("in-memory db");
    let device = storage.local_device().expect("device");
    let ts = 1_760_000_000_000i64;
    let ev = NormalizedUsageEvent {
        event_id: compute_event_id("codex", "secret-session", "step-1", ts),
        device_id: device.device_id,
        provider_id: "codex".to_string(),
        session_id: "secret-session".to_string(),
        project_label: "secret-project".to_string(),
        model_id: "gpt-5-codex".to_string(),
        timestamp_ms: ts,
        input_tokens: 2000,
        output_tokens: 500,
        cache_read_tokens: 1000,
        cache_write_tokens: 0,
        reasoning_tokens: 100,
        observable_only: false,
        duration_ms: Some(1500),
        ttft_ms: None,
        output_tps: Some(40.0),
        estimated_cost_micro_usd: 0,
        cache_saved_micro_usd: 0,
    };
    storage
        .ingest_events_with_cursor(&[ev], None, &[])
        .expect("ingest");

    let env = storage
        .build_sync_envelope_v1(0, ts + 60_000)
        .expect("build envelope");
    assert_eq!(env.schema_version, 1);
    assert_eq!(env.hourly_deltas.len(), 1);
    let serialized = serde_json::to_string(&env).expect("serialize");
    assert!(!serialized.contains("secret-session"));
    assert!(!serialized.contains("secret-project"));
}

#[test]
fn fresh_install_has_zero_subscriptions_and_default_bg_preferences() {
    let storage = Storage::open_in_memory().expect("in-memory db");
    let plans = storage.list_subscription_plans().expect("plans");
    assert!(plans.is_empty());

    let prefs = storage.get_preferences().expect("prefs");
    assert_eq!(prefs.bg_mode, "none");
    assert_eq!(prefs.bg_image_path, None);
    assert_eq!(prefs.bg_preset_id, "aurora_slate");
    assert!((prefs.bg_opacity - 0.18).abs() < 1e-6);
    assert_eq!(prefs.bg_blur_px, 16);
}

#[test]
fn preferences_bg_roundtrip_and_subscription_delete_work() {
    let storage = Storage::open_in_memory().expect("in-memory db");
    let mut prefs = storage.get_preferences().expect("prefs");
    prefs.bg_mode = "gradient".to_string();
    prefs.bg_image_path = Some("wallpaper_123.png".to_string());
    prefs.bg_preset_id = "midnight_indigo".to_string();
    prefs.bg_opacity = 0.42;
    prefs.bg_blur_px = 24;
    storage.update_preferences(&prefs).expect("update prefs");

    let reloaded = storage.get_preferences().expect("reloaded prefs");
    assert_eq!(reloaded.bg_mode, "gradient");
    assert_eq!(
        reloaded.bg_image_path.as_deref(),
        Some("wallpaper_123.png")
    );
    assert_eq!(reloaded.bg_preset_id, "midnight_indigo");
    assert!((reloaded.bg_opacity - 0.42).abs() < 1e-6);
    assert_eq!(reloaded.bg_blur_px, 24);

    let plan = SubscriptionPlan {
        plan_id: "custom_claude".to_string(),
        provider_id: "claude_code".to_string(),
        plan_name: "My Claude Plan".to_string(),
        enabled: true,
        monthly_fee_micro_usd: 20_000_000,
        windows: vec![],
        is_user_configured: false,
        auto_source: None,
        updated_at_ms: 1_760_000_000_000,
    };
    storage.upsert_subscription_plan(&plan).expect("upsert plan");
    let list = storage.list_subscription_plans().expect("list plans");
    assert_eq!(list.len(), 1);
    assert!(list[0].is_user_configured);

    storage
        .delete_subscription_plan("custom_claude")
        .expect("delete plan");
    assert!(storage
        .list_subscription_plans()
        .expect("empty")
        .is_empty());
}

#[test]
fn v1_to_v2_migration_cleans_presets_tps_and_rebuilds_rollups() {
    use crate::domain::QuotaUnit;

    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("legacy_v1.db");
    let seed_ts = 1_760_000_000_000i64;

    {
        let conn = Connection::open(&db_path).expect("open v1");
        conn.execute_batch(
            r#"
            CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at_ms INTEGER NOT NULL);
            CREATE TABLE devices (device_id TEXT PRIMARY KEY, device_name TEXT NOT NULL, platform TEXT NOT NULL, created_at_ms INTEGER NOT NULL);
            CREATE TABLE provider_configs (provider_id TEXT PRIMARY KEY, enabled INTEGER NOT NULL, collection_mode TEXT NOT NULL, custom_log_paths_json TEXT NOT NULL, has_credential INTEGER NOT NULL, last_sync_at_ms INTEGER, last_error TEXT, updated_at_ms INTEGER NOT NULL);
            CREATE TABLE ingestion_cursors (source_path TEXT PRIMARY KEY, provider_id TEXT NOT NULL, file_identity TEXT NOT NULL, byte_offset INTEGER NOT NULL, last_line_hash TEXT NOT NULL, updated_at_ms INTEGER NOT NULL);
            CREATE TABLE usage_events (event_id TEXT PRIMARY KEY, device_id TEXT NOT NULL, provider_id TEXT NOT NULL, session_id TEXT NOT NULL, project_label TEXT NOT NULL, model_id TEXT NOT NULL, timestamp_ms INTEGER NOT NULL, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, observable_only INTEGER NOT NULL DEFAULT 0, duration_ms INTEGER, ttft_ms INTEGER, output_tps REAL, estimated_cost_micro_usd INTEGER NOT NULL, cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE usage_hourly_rollups (device_id TEXT NOT NULL, provider_id TEXT NOT NULL, model_id TEXT NOT NULL, bucket_start_ms INTEGER NOT NULL, request_count INTEGER NOT NULL, observable_step_count INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, estimated_cost_micro_usd INTEGER NOT NULL, cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0, total_duration_ms INTEGER NOT NULL, tps_sample_count INTEGER NOT NULL, tps_sum REAL NOT NULL, max_output_tps REAL NOT NULL, updated_at_ms INTEGER NOT NULL, PRIMARY KEY (device_id, provider_id, model_id, bucket_start_ms));
            CREATE TABLE usage_daily_rollups (device_id TEXT NOT NULL, provider_id TEXT NOT NULL, model_id TEXT NOT NULL, bucket_start_ms INTEGER NOT NULL, request_count INTEGER NOT NULL, observable_step_count INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, estimated_cost_micro_usd INTEGER NOT NULL, cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0, total_duration_ms INTEGER NOT NULL, tps_sample_count INTEGER NOT NULL, tps_sum REAL NOT NULL, max_output_tps REAL NOT NULL, updated_at_ms INTEGER NOT NULL, PRIMARY KEY (device_id, provider_id, model_id, bucket_start_ms));
            CREATE TABLE model_pricing (model_pattern TEXT PRIMARY KEY, provider_id TEXT NOT NULL, input_per_m_micro_usd INTEGER NOT NULL, output_per_m_micro_usd INTEGER NOT NULL, cache_read_per_m_micro_usd INTEGER NOT NULL, cache_write_per_m_micro_usd INTEGER NOT NULL, reasoning_per_m_micro_usd INTEGER NOT NULL, is_user_override INTEGER NOT NULL, effective_from TEXT NOT NULL);
            CREATE TABLE subscription_plans (plan_id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, plan_name TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, monthly_fee_micro_usd INTEGER NOT NULL, windows_json TEXT NOT NULL, updated_at_ms INTEGER NOT NULL);
            CREATE TABLE observed_rate_limits (provider_id TEXT NOT NULL, window_name TEXT NOT NULL, used_percent REAL NOT NULL, window_minutes INTEGER, resets_at_ms INTEGER, observed_at_ms INTEGER NOT NULL, PRIMARY KEY (provider_id, window_name));
            CREATE TABLE app_preferences (id INTEGER PRIMARY KEY CHECK (id = 1), currency TEXT NOT NULL, usd_to_cny_rate TEXT NOT NULL, theme TEXT NOT NULL, minimize_to_tray INTEGER NOT NULL, launch_at_startup INTEGER NOT NULL, otlp_enabled INTEGER NOT NULL, otlp_port INTEGER NOT NULL, device_display_name TEXT NOT NULL, updated_at_ms INTEGER NOT NULL);
            "#,
        )
        .expect("create v1 schema");

        conn.execute(
            "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (1, ?1)",
            params![seed_ts],
        )
        .expect("v1 version");
        conn.execute(
            "INSERT INTO devices (device_id, device_name, platform, created_at_ms) VALUES ('dev-1', 'win', 'windows', ?1)",
            params![seed_ts],
        )
        .expect("device");
        conn.execute(
            "INSERT INTO app_preferences (id, currency, usd_to_cny_rate, theme, minimize_to_tray, launch_at_startup, otlp_enabled, otlp_port, device_display_name, updated_at_ms) VALUES (1, 'USD', '7.25', 'dark', 1, 0, 0, 4318, 'win', ?1)",
            params![seed_ts],
        )
        .expect("prefs");

        let default_windows = serde_json::to_string(&vec![
            SubscriptionWindowRule {
                window_id: "5h".to_string(),
                label: "5h 滚动窗口".to_string(),
                duration_hours: 5,
                rolling: true,
                unit: QuotaUnit::MicroUsd,
                limit_value: 15_000_000,
                warning_percent: 80,
                critical_percent: 95,
            },
            SubscriptionWindowRule {
                window_id: "7d".to_string(),
                label: "7d 周额度".to_string(),
                duration_hours: 168,
                rolling: true,
                unit: QuotaUnit::MicroUsd,
                limit_value: 75_000_000,
                warning_percent: 80,
                critical_percent: 95,
            },
        ])
        .expect("windows json");

        // Untouched v0.1.0 preset within 5_000 ms of device seed -> must be deleted
        conn.execute(
            "INSERT INTO subscription_plans (plan_id, provider_id, plan_name, enabled, monthly_fee_micro_usd, windows_json, updated_at_ms) VALUES ('claude_pro', 'claude_code', 'Claude Pro ($20/mo)', 1, 20000000, ?1, ?2)",
            params![default_windows, seed_ts + 200],
        )
        .expect("untouched preset");

        // Customized v0.1.0 manual preset (fee altered) -> preserved at v2, then purged at v11 with all manual plans
        conn.execute(
            "INSERT INTO subscription_plans (plan_id, provider_id, plan_name, enabled, monthly_fee_micro_usd, windows_json, updated_at_ms) VALUES ('codex_plus', 'codex', 'ChatGPT Plus / Codex ($20/mo)', 1, 25000000, ?1, ?2)",
            params![default_windows, seed_ts + 200],
        )
        .expect("customized preset");

        // Valid event + fast distorted event + tiny output event
        conn.execute(
            "INSERT INTO usage_events VALUES ('ev-ok', 'dev-1', 'claude_code', 's1', 'proj', 'claude-sonnet-4-5', ?1, 100, 100, 0, 0, 0, 0, 2000, NULL, 50.0, 1000, 0)",
            params![seed_ts],
        )
        .expect("ev-ok");
        conn.execute(
            "INSERT INTO usage_events VALUES ('ev-fast', 'dev-1', 'claude_code', 's1', 'proj', 'claude-sonnet-4-5', ?1, 100, 500, 0, 0, 0, 0, 10, NULL, 50000.0, 2000, 0)",
            params![seed_ts + 1_000],
        )
        .expect("ev-fast");
        conn.execute(
            "INSERT INTO usage_events VALUES ('ev-tiny', 'dev-1', 'claude_code', 's1', 'proj', 'claude-sonnet-4-5', ?1, 100, 4, 0, 0, 0, 0, 500, NULL, 8.0, 500, 0)",
            params![seed_ts + 2_000],
        )
        .expect("ev-tiny");
    }

    let storage = Storage::open_file(&db_path).expect("migrate v1 -> v2");
    let plans = storage.list_subscription_plans().expect("plans");
    assert!(
        plans.is_empty(),
        "v1 -> current schema migration must purge untouched v0.1.0 presets at v2 and manual plans at v11"
    );

    let prefs = storage.get_preferences().expect("prefs");
    assert_eq!(prefs.bg_mode, "none");
    assert_eq!(prefs.theme, "dark");
    assert!(prefs.auto_sync_pricing);
    assert!(prefs.exchange_rate_auto);

    storage
        .with_conn(|conn| {
            let ver: i64 =
                conn.query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))?;
            assert_eq!(ver, crate::storage::migrations::CURRENT_SCHEMA_VERSION);

            let ok_tps: Option<f64> = conn.query_row(
                "SELECT output_tps FROM usage_events WHERE event_id = 'ev-ok'",
                [],
                |r| r.get(0),
            )?;
            let fast_tps: Option<f64> = conn.query_row(
                "SELECT output_tps FROM usage_events WHERE event_id = 'ev-fast'",
                [],
                |r| r.get(0),
            )?;
            let tiny_tps: Option<f64> = conn.query_row(
                "SELECT output_tps FROM usage_events WHERE event_id = 'ev-tiny'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(ok_tps, Some(50.0));
            assert_eq!(fast_tps, None);
            assert_eq!(tiny_tps, None);

            let (h_cnt, h_sum, h_max): (i64, f64, f64) = conn.query_row(
                "SELECT SUM(tps_sample_count), SUM(tps_sum), MAX(max_output_tps) FROM usage_hourly_rollups",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            assert_eq!(h_cnt, 1);
            assert!((h_sum - 50.0).abs() < 1e-6);
            assert!((h_max - 50.0).abs() < 1e-6);

            let (d_cnt, d_sum, d_max): (i64, f64, f64) = conn.query_row(
                "SELECT SUM(tps_sample_count), SUM(tps_sum), MAX(max_output_tps) FROM usage_daily_rollups",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            assert_eq!(d_cnt, 1);
            assert!((d_sum - 50.0).abs() < 1e-6);
            assert!((d_max - 50.0).abs() < 1e-6);
            Ok(())
        })
        .expect("verify migrated state");
}

#[test]
fn v2_to_v3_migration_preserves_settings_and_adds_sync_columns() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("legacy_v2.db");
    let seed_ts = 1_760_000_000_000i64;

    {
        let conn = Connection::open(&db_path).expect("open v2");
        conn.execute_batch(
            r#"
            CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at_ms INTEGER NOT NULL);
            CREATE TABLE devices (device_id TEXT PRIMARY KEY, device_name TEXT NOT NULL, platform TEXT NOT NULL, created_at_ms INTEGER NOT NULL);
            CREATE TABLE provider_configs (provider_id TEXT PRIMARY KEY, enabled INTEGER NOT NULL, collection_mode TEXT NOT NULL, custom_log_paths_json TEXT NOT NULL, has_credential INTEGER NOT NULL, last_sync_at_ms INTEGER, last_error TEXT, updated_at_ms INTEGER NOT NULL);
            CREATE TABLE ingestion_cursors (source_path TEXT PRIMARY KEY, provider_id TEXT NOT NULL, file_identity TEXT NOT NULL, byte_offset INTEGER NOT NULL, last_line_hash TEXT NOT NULL, updated_at_ms INTEGER NOT NULL);
            CREATE TABLE usage_events (event_id TEXT PRIMARY KEY, device_id TEXT NOT NULL, provider_id TEXT NOT NULL, session_id TEXT NOT NULL, project_label TEXT NOT NULL, model_id TEXT NOT NULL, timestamp_ms INTEGER NOT NULL, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, observable_only INTEGER NOT NULL DEFAULT 0, duration_ms INTEGER, ttft_ms INTEGER, output_tps REAL, estimated_cost_micro_usd INTEGER NOT NULL, cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE usage_hourly_rollups (device_id TEXT NOT NULL, provider_id TEXT NOT NULL, model_id TEXT NOT NULL, bucket_start_ms INTEGER NOT NULL, request_count INTEGER NOT NULL, observable_step_count INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, estimated_cost_micro_usd INTEGER NOT NULL, cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0, total_duration_ms INTEGER NOT NULL, tps_sample_count INTEGER NOT NULL, tps_sum REAL NOT NULL, max_output_tps REAL NOT NULL, updated_at_ms INTEGER NOT NULL, PRIMARY KEY (device_id, provider_id, model_id, bucket_start_ms));
            CREATE TABLE usage_daily_rollups (device_id TEXT NOT NULL, provider_id TEXT NOT NULL, model_id TEXT NOT NULL, bucket_start_ms INTEGER NOT NULL, request_count INTEGER NOT NULL, observable_step_count INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, estimated_cost_micro_usd INTEGER NOT NULL, cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0, total_duration_ms INTEGER NOT NULL, tps_sample_count INTEGER NOT NULL, tps_sum REAL NOT NULL, max_output_tps REAL NOT NULL, updated_at_ms INTEGER NOT NULL, PRIMARY KEY (device_id, provider_id, model_id, bucket_start_ms));
            CREATE TABLE model_pricing (model_pattern TEXT PRIMARY KEY, provider_id TEXT NOT NULL, input_per_m_micro_usd INTEGER NOT NULL, output_per_m_micro_usd INTEGER NOT NULL, cache_read_per_m_micro_usd INTEGER NOT NULL, cache_write_per_m_micro_usd INTEGER NOT NULL, reasoning_per_m_micro_usd INTEGER NOT NULL, is_user_override INTEGER NOT NULL, effective_from TEXT NOT NULL);
            CREATE TABLE subscription_plans (plan_id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, plan_name TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, monthly_fee_micro_usd INTEGER NOT NULL, windows_json TEXT NOT NULL, is_user_configured INTEGER NOT NULL DEFAULT 0, updated_at_ms INTEGER NOT NULL);
            CREATE TABLE observed_rate_limits (provider_id TEXT NOT NULL, window_name TEXT NOT NULL, used_percent REAL NOT NULL, window_minutes INTEGER, resets_at_ms INTEGER, observed_at_ms INTEGER NOT NULL, PRIMARY KEY (provider_id, window_name));
            CREATE TABLE app_preferences (id INTEGER PRIMARY KEY CHECK (id = 1), currency TEXT NOT NULL, usd_to_cny_rate TEXT NOT NULL, theme TEXT NOT NULL, minimize_to_tray INTEGER NOT NULL, launch_at_startup INTEGER NOT NULL, otlp_enabled INTEGER NOT NULL, otlp_port INTEGER NOT NULL, device_display_name TEXT NOT NULL, bg_mode TEXT NOT NULL DEFAULT 'none', bg_image_path TEXT, bg_preset_id TEXT NOT NULL DEFAULT 'aurora_slate', bg_opacity REAL NOT NULL DEFAULT 0.18, bg_blur_px INTEGER NOT NULL DEFAULT 16, updated_at_ms INTEGER NOT NULL);
            "#,
        )
        .expect("create v2 schema");
        conn.execute(
            "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (1, ?1), (2, ?1)",
            params![seed_ts],
        )
        .expect("versions");
        conn.execute(
            "INSERT INTO devices (device_id, device_name, platform, created_at_ms) VALUES ('dev-2', 'win', 'windows', ?1)",
            params![seed_ts],
        )
        .expect("device");
        conn.execute(
            "INSERT INTO app_preferences (id, currency, usd_to_cny_rate, theme, minimize_to_tray, launch_at_startup, otlp_enabled, otlp_port, device_display_name, bg_mode, bg_preset_id, bg_opacity, bg_blur_px, updated_at_ms) VALUES (1, 'CNY', '7.31', 'light', 1, 0, 0, 4318, 'win', 'gradient', 'ocean', 0.22, 12, ?1)",
            params![seed_ts],
        )
        .expect("prefs");
        conn.execute(
            "INSERT INTO model_pricing VALUES ('claude-sonnet-4-5', 'claude_code', 999, 999, 0, 0, 999, 1, '2026-10-01')",
            [],
        )
        .expect("override pricing");
    }

    let storage = Storage::open_file(&db_path).expect("migrate v2 -> v3");
    let prefs = storage.get_preferences().expect("prefs");
    assert_eq!(prefs.currency, CurrencyCode::Cny);
    assert_eq!(prefs.usd_to_cny_rate, "7.31");
    assert!(prefs.auto_sync_pricing);
    assert!(prefs.exchange_rate_auto);
    assert_eq!(prefs.last_rate_sync_at_ms, None);
    assert_eq!(prefs.last_pricing_sync_at_ms, None);

    let catalog = storage.list_model_pricing().expect("catalog");
    let sonnet = catalog
        .iter()
        .find(|p| p.model_pattern == "claude-sonnet-4-5")
        .expect("sonnet");
    assert!(sonnet.is_user_override);
    assert_eq!(sonnet.input_per_m_micro_usd, 999);
}

#[test]
fn synced_rate_respects_manual_lock_and_preserves_timestamps_across_pref_saves() {
    let storage = Storage::open_in_memory().expect("db");

    // Auto mode updates rate & timestamp
    let changed = storage
        .apply_synced_exchange_rate("7.1852", 1_760_100_000_000)
        .expect("apply rate");
    assert!(changed);
    let mut prefs = storage.get_preferences().expect("prefs");
    assert_eq!(prefs.usd_to_cny_rate, "7.1852");
    assert_eq!(prefs.last_rate_sync_at_ms, Some(1_760_100_000_000));

    // Lock manual rate -> saving UI preferences must not wipe sync timestamp
    prefs.exchange_rate_auto = false;
    prefs.usd_to_cny_rate = "7.35".to_string();
    prefs.last_rate_sync_at_ms = None;
    storage.update_preferences(&prefs).expect("save manual prefs");

    let reloaded = storage.get_preferences().expect("reloaded");
    assert!(!reloaded.exchange_rate_auto);
    assert_eq!(reloaded.usd_to_cny_rate, "7.35");
    assert_eq!(reloaded.last_rate_sync_at_ms, Some(1_760_100_000_000));

    // Subsequent synced rate must be ignored when locked
    let locked_changed = storage
        .apply_synced_exchange_rate("7.10", 1_760_200_000_000)
        .expect("locked noop");
    assert!(!locked_changed);
    let final_prefs = storage.get_preferences().expect("final prefs");
    assert_eq!(final_prefs.usd_to_cny_rate, "7.35");
    assert_eq!(final_prefs.last_rate_sync_at_ms, Some(1_760_100_000_000));
}

#[test]
fn synced_pricing_preserves_user_overrides_and_conditionally_recomputes() {
    let storage = Storage::open_in_memory().expect("db");
    let dev = storage.local_device().expect("device").device_id;
    let ts = 1_760_000_000_000i64;

    // Add user override on claude-sonnet-4-5 ($2 / $10) and ingest two events:
    // one on overridden claude-sonnet-4-5, one on built-in gpt-5-codex
    storage
        .upsert_model_pricing(&ModelPricingEntry {
            model_pattern: "claude-sonnet-4-5".to_string(),
            provider_id: "claude_code".to_string(),
            input_per_m_micro_usd: 2_000_000,
            output_per_m_micro_usd: 10_000_000,
            cache_read_per_m_micro_usd: 200_000,
            cache_write_per_m_micro_usd: 2_500_000,
            reasoning_per_m_micro_usd: 10_000_000,
            is_user_override: true,
            effective_from: "2026-11-01".to_string(),
        })
        .expect("override");

    let events = vec![
        NormalizedUsageEvent {
            event_id: compute_event_id("claude_code", "s1", "m1", ts),
            device_id: dev.clone(),
            provider_id: "claude_code".to_string(),
            session_id: "s1".to_string(),
            project_label: "app".to_string(),
            model_id: "claude-sonnet-4-5".to_string(),
            timestamp_ms: ts,
            input_tokens: 1_000_000,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: None,
            ttft_ms: None,
            output_tps: None,
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
        NormalizedUsageEvent {
            event_id: compute_event_id("codex", "s2", "m2", ts),
            device_id: dev,
            provider_id: "codex".to_string(),
            session_id: "s2".to_string(),
            project_label: "cli".to_string(),
            model_id: "gpt-5-codex".to_string(),
            timestamp_ms: ts,
            input_tokens: 1_000_000,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            observable_only: false,
            duration_ms: None,
            ttft_ms: None,
            output_tps: None,
            estimated_cost_micro_usd: 0,
            cache_saved_micro_usd: 0,
        },
    ];
    storage
        .ingest_events_with_cursor(&events, None, &[])
        .expect("ingest");

    // Sync catalog changing ONLY unused gemini-3-flash & trying to change overridden sonnet
    let first_remote = vec![
        ModelPricingEntry {
            model_pattern: "claude-sonnet-4-5".to_string(),
            provider_id: "claude_code".to_string(),
            input_per_m_micro_usd: 99_000_000,
            output_per_m_micro_usd: 99_000_000,
            cache_read_per_m_micro_usd: 0,
            cache_write_per_m_micro_usd: 0,
            reasoning_per_m_micro_usd: 99_000_000,
            is_user_override: false,
            effective_from: "2026-11-16".to_string(),
        },
        ModelPricingEntry {
            model_pattern: "gemini-3-flash".to_string(),
            provider_id: "antigravity".to_string(),
            input_per_m_micro_usd: 450_000,
            output_per_m_micro_usd: 2_800_000,
            cache_read_per_m_micro_usd: 45_000,
            cache_write_per_m_micro_usd: 0,
            reasoning_per_m_micro_usd: 2_800_000,
            is_user_override: false,
            effective_from: "2026-11-16".to_string(),
        },
    ];

    let (updated_1, _, recomputed_1) = storage
        .apply_synced_pricing_catalog(&first_remote, ts + 10_000)
        .expect("first sync");
    assert!(updated_1);
    assert_eq!(recomputed_1, 0); // Neither active model's effective pricing changed!

    // Identical second sync short-circuits via hash without updating catalog
    let (updated_2, _, recomputed_2) = storage
        .apply_synced_pricing_catalog(&first_remote, ts + 20_000)
        .expect("second sync");
    assert!(!updated_2);
    assert_eq!(recomputed_2, 0);

    // Now change non-overridden active model gpt-5-codex price -> triggers recompute for 1 event!
    let second_remote = vec![ModelPricingEntry {
        model_pattern: "gpt-5-codex".to_string(),
        provider_id: "codex".to_string(),
        input_per_m_micro_usd: 2_500_000,
        output_per_m_micro_usd: 15_000_000,
        cache_read_per_m_micro_usd: 250_000,
        cache_write_per_m_micro_usd: 0,
        reasoning_per_m_micro_usd: 15_000_000,
        is_user_override: false,
        effective_from: "2026-11-16".to_string(),
    }];
    let (updated_3, _, recomputed_3) = storage
        .apply_synced_pricing_catalog(&second_remote, ts + 30_000)
        .expect("third sync");
    assert!(updated_3);
    assert_eq!(recomputed_3, 1);

    let catalog = storage.list_model_pricing().expect("catalog");
    let sonnet = catalog
        .iter()
        .find(|p| p.model_pattern == "claude-sonnet-4-5")
        .expect("sonnet");
    assert!(sonnet.is_user_override);
    assert_eq!(sonnet.input_per_m_micro_usd, 2_000_000);
}

#[test]
fn v3_to_v4_migration_preserves_data_and_adds_new_tables() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("legacy_v3.db");
    let seed_ts = 1_760_000_000_000i64;

    {
        let conn = Connection::open(&db_path).expect("open v3");
        conn.execute_batch(
            r#"
            CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at_ms INTEGER NOT NULL);
            CREATE TABLE devices (device_id TEXT PRIMARY KEY, device_name TEXT NOT NULL, platform TEXT NOT NULL, created_at_ms INTEGER NOT NULL);
            CREATE TABLE provider_configs (provider_id TEXT PRIMARY KEY, enabled INTEGER NOT NULL, collection_mode TEXT NOT NULL, custom_log_paths_json TEXT NOT NULL, has_credential INTEGER NOT NULL, last_sync_at_ms INTEGER, last_error TEXT, updated_at_ms INTEGER NOT NULL);
            CREATE TABLE ingestion_cursors (source_path TEXT PRIMARY KEY, provider_id TEXT NOT NULL, file_identity TEXT NOT NULL, byte_offset INTEGER NOT NULL, last_line_hash TEXT NOT NULL, updated_at_ms INTEGER NOT NULL);
            CREATE TABLE usage_events (event_id TEXT PRIMARY KEY, device_id TEXT NOT NULL, provider_id TEXT NOT NULL, session_id TEXT NOT NULL, project_label TEXT NOT NULL, model_id TEXT NOT NULL, timestamp_ms INTEGER NOT NULL, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, observable_only INTEGER NOT NULL DEFAULT 0, duration_ms INTEGER, ttft_ms INTEGER, output_tps REAL, estimated_cost_micro_usd INTEGER NOT NULL, cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE usage_hourly_rollups (device_id TEXT NOT NULL, provider_id TEXT NOT NULL, model_id TEXT NOT NULL, bucket_start_ms INTEGER NOT NULL, request_count INTEGER NOT NULL, observable_step_count INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, estimated_cost_micro_usd INTEGER NOT NULL, cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0, total_duration_ms INTEGER NOT NULL, tps_sample_count INTEGER NOT NULL, tps_sum REAL NOT NULL, max_output_tps REAL NOT NULL, updated_at_ms INTEGER NOT NULL, PRIMARY KEY (device_id, provider_id, model_id, bucket_start_ms));
            CREATE TABLE usage_daily_rollups (device_id TEXT NOT NULL, provider_id TEXT NOT NULL, model_id TEXT NOT NULL, bucket_start_ms INTEGER NOT NULL, request_count INTEGER NOT NULL, observable_step_count INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, estimated_cost_micro_usd INTEGER NOT NULL, cache_saved_micro_usd INTEGER NOT NULL DEFAULT 0, total_duration_ms INTEGER NOT NULL, tps_sample_count INTEGER NOT NULL, tps_sum REAL NOT NULL, max_output_tps REAL NOT NULL, updated_at_ms INTEGER NOT NULL, PRIMARY KEY (device_id, provider_id, model_id, bucket_start_ms));
            CREATE TABLE model_pricing (model_pattern TEXT PRIMARY KEY, provider_id TEXT NOT NULL, input_per_m_micro_usd INTEGER NOT NULL, output_per_m_micro_usd INTEGER NOT NULL, cache_read_per_m_micro_usd INTEGER NOT NULL, cache_write_per_m_micro_usd INTEGER NOT NULL, reasoning_per_m_micro_usd INTEGER NOT NULL, is_user_override INTEGER NOT NULL, effective_from TEXT NOT NULL);
            CREATE TABLE subscription_plans (plan_id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, plan_name TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, monthly_fee_micro_usd INTEGER NOT NULL, windows_json TEXT NOT NULL, is_user_configured INTEGER NOT NULL DEFAULT 0, updated_at_ms INTEGER NOT NULL);
            CREATE TABLE observed_rate_limits (provider_id TEXT PRIMARY KEY, window_name TEXT NOT NULL, used_percent REAL NOT NULL, window_minutes INTEGER, resets_at_ms INTEGER, observed_at_ms INTEGER NOT NULL);
            DROP TABLE IF EXISTS observed_rate_limits;
            CREATE TABLE observed_rate_limits (provider_id TEXT NOT NULL, window_name TEXT NOT NULL, used_percent REAL NOT NULL, window_minutes INTEGER, resets_at_ms INTEGER, observed_at_ms INTEGER NOT NULL, PRIMARY KEY (provider_id, window_name));
            CREATE TABLE app_preferences (id INTEGER PRIMARY KEY CHECK (id = 1), currency TEXT NOT NULL, usd_to_cny_rate TEXT NOT NULL, theme TEXT NOT NULL, minimize_to_tray INTEGER NOT NULL, launch_at_startup INTEGER NOT NULL, otlp_enabled INTEGER NOT NULL, otlp_port INTEGER NOT NULL, device_display_name TEXT NOT NULL, bg_mode TEXT NOT NULL DEFAULT 'none', bg_image_path TEXT, bg_preset_id TEXT NOT NULL DEFAULT 'aurora_slate', bg_opacity REAL NOT NULL DEFAULT 0.18, bg_blur_px INTEGER NOT NULL DEFAULT 16, auto_sync_pricing INTEGER NOT NULL DEFAULT 1, exchange_rate_auto INTEGER NOT NULL DEFAULT 1, last_rate_sync_at_ms INTEGER, last_pricing_sync_at_ms INTEGER, pricing_catalog_hash TEXT, updated_at_ms INTEGER NOT NULL);
            CREATE TABLE remote_sync_meta (meta_key TEXT PRIMARY KEY, meta_value TEXT NOT NULL, updated_at_ms INTEGER NOT NULL);
            "#,
        )
        .expect("v3 schema");

        conn.execute(
            "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (3, ?1)",
            params![seed_ts],
        )
        .expect("v3 version");
        conn.execute(
            "INSERT INTO devices (device_id, device_name, platform, created_at_ms) VALUES ('dev-v3', 'ws-v3', 'windows', ?1)",
            params![seed_ts],
        )
        .expect("dev");
        conn.execute(
            "INSERT INTO app_preferences (id, currency, usd_to_cny_rate, theme, minimize_to_tray, launch_at_startup, otlp_enabled, otlp_port, device_display_name, updated_at_ms) VALUES (1, 'CNY', '7.20', 'light', 1, 1, 1, 4318, 'ws-v3', ?1)",
            params![seed_ts],
        )
        .expect("prefs");
        conn.execute(
            "INSERT INTO model_pricing VALUES ('deepseek-chat', 'deepseek', 270000, 1100000, 70000, 0, 1100000, 0, '2026-01-01')",
            [],
        )
        .expect("ds pricing");
    }

    let storage = Storage::open_file(&db_path).expect("migrate v3 -> v4");
    let prefs = storage.get_preferences().expect("prefs");
    assert!(prefs.auto_check_updates);
    assert_eq!(prefs.device_display_name, "ws-v3");

    let configs = storage.list_provider_configs().expect("providers");
    assert_eq!(configs.len(), 8);

    let pricing = storage.list_model_pricing().expect("pricing");
    let ds = pricing
        .iter()
        .find(|p| p.model_pattern == "deepseek-chat")
        .expect("ds migrated");
    assert_eq!(ds.provider_id, "deepseek_harness");

    let sync_cfg = storage.get_sync_config_state().expect("sync state");
    assert!(!sync_cfg.enabled);
    assert!(storage
        .list_provider_balances()
        .expect("balances")
        .is_empty());
    assert!(storage
        .list_custom_provider_rules()
        .expect("custom rules")
        .is_empty());
}

#[test]
fn apply_remote_sync_envelope_is_idempotent_and_multi_device_safe() {
    use crate::domain::{SyncEnvelopeV1, SyncHourlyDelta, SyncSubscriptionSnapshot};

    let storage = Storage::open_in_memory().expect("db");
    let ts = 1_760_000_000_000i64;
    let hour = ts.div_euclid(HOUR_MS) * HOUR_MS;
    let env = SyncEnvelopeV1 {
        schema_version: 1,
        device_id: "macbook-air".to_string(),
        device_display_name: "MacBook Air".to_string(),
        generated_at_ms: ts + 10_000,
        since_watermark_ms: 0,
        hourly_deltas: vec![SyncHourlyDelta {
            provider_id: "kimi_code".to_string(),
            model_id: "kimi-k2-0905-preview".to_string(),
            bucket_start_ms: hour,
            request_count: 4,
            observable_step_count: 0,
            input_tokens: 10_000,
            output_tokens: 2_000,
            cache_read_tokens: 3_000,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            estimated_cost_micro_usd: 150_000,
            cache_saved_micro_usd: 5_000,
            total_duration_ms: 4_000,
            tps_sample_count: 4,
            tps_sum: 200.0,
            max_output_tps: 65.0,
            updated_at_ms: ts + 10_000,
        }],
        subscription_snapshots: vec![SyncSubscriptionSnapshot {
            plan_id: "kimi-pro".to_string(),
            provider_id: "kimi_code".to_string(),
            plan_name: "Kimi Pro".to_string(),
            window_id: "5h".to_string(),
            window_label: "5h 窗口".to_string(),
            used_percent: 33.5,
            resets_at_ms: Some(ts + 3_600_000),
            source: "estimated".to_string(),
            monthly_fee_micro_usd: 0,
        }],
        payg_snapshots: Vec::new(),
    };

    let r1 = storage
        .apply_remote_sync_envelope_v1(&env, ts + 11_000)
        .expect("first apply");
    assert_eq!(r1.hourly_buckets_applied, 1);
    assert_eq!(r1.subscription_snapshots_applied, 1);

    // Replay identical envelope must stay idempotent and never double-count daily rollups
    let r2 = storage
        .apply_remote_sync_envelope_v1(&env, ts + 12_000)
        .expect("replay apply");
    assert_eq!(r2.hourly_buckets_applied, 0);

    storage
        .with_conn(|conn| {
            let (h_tok, h_req): (i64, i64) = conn.query_row(
                "SELECT SUM(input_tokens + output_tokens + cache_read_tokens), SUM(request_count) FROM usage_hourly_rollups WHERE device_id = 'macbook-air'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let (d_tok, d_req): (i64, i64) = conn.query_row(
                "SELECT SUM(input_tokens + output_tokens + cache_read_tokens), SUM(request_count) FROM usage_daily_rollups WHERE device_id = 'macbook-air'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            assert_eq!((h_tok, h_req), (15_000, 4));
            assert_eq!((d_tok, d_req), (15_000, 4));
            Ok(())
        })
        .expect("verify rollup totals");

    let devices = storage.list_devices().expect("devices");
    assert!(devices.iter().any(|d| d.device_id == "macbook-air"));
    let remote_snaps = storage
        .list_remote_subscription_snapshots()
        .expect("remote snaps");
    assert_eq!(remote_snaps.len(), 1);
    assert_eq!(remote_snaps[0].device_display_name, "MacBook Air");
}

#[test]
fn auto_subscription_discovery_respects_user_edits_dismissal_and_resets() {
    let storage = Storage::open_in_memory().expect("db");
    let now_ms = Utc::now().timestamp_millis();

    // 1. Fresh DB starts with zero subscriptions (Bug 4 preserves zero-assumption for non-observable providers).
    assert!(storage
        .query_subscriptions(now_ms)
        .expect("initial subs")
        .is_empty());

    // 2. Observed rate-limit from official API creates auto_kimi_code in "我的订阅配置".
    storage
        .upsert_observed_rate_limits(&[
            ObservedRateLimitWindow {
                provider_id: provider_ids::KIMI_CODE.to_string(),
                window_name: "5h".to_string(),
                used_percent: 25.0,
                window_minutes: Some(300),
                resets_at_ms: Some(now_ms + 3_600_000),
                observed_at_ms: now_ms,
                source: Some("official_api".to_string()),
                plan_hint: Some("LEVEL_INTERMEDIATE".to_string()),
            },
            ObservedRateLimitWindow {
                provider_id: provider_ids::KIMI_CODE.to_string(),
                window_name: "7d".to_string(),
                used_percent: 40.0,
                window_minutes: Some(10_080),
                resets_at_ms: Some(now_ms + 3 * DAY_MS),
                observed_at_ms: now_ms,
                source: Some("official_api".to_string()),
                plan_hint: Some("LEVEL_INTERMEDIATE".to_string()),
            },
        ])
        .expect("upsert kimi observed");

    let subs = storage.query_subscriptions(now_ms).expect("subs after api");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].plan.plan_id, "auto_kimi_code");
    assert_eq!(subs[0].plan.auto_source.as_deref(), Some("official_api"));
    // The membership level maps onto the catalog tier, so the name and monthly fee both come
    // from the catalog instead of a hardcoded zero fee.
    assert_eq!(subs[0].plan.plan_name, "Kimi Pro (Allegretto)");
    assert_eq!(subs[0].plan.monthly_fee_micro_usd, 39_000_000);
    // Only the 2 API-returned windows (5h, 7d) are shown; no fake local 30d estimate is synthesized.
    assert_eq!(subs[0].windows.len(), 2);
    assert_eq!(subs[0].windows[0].source, "official_api");
    assert!((subs[0].windows[0].used_percent - 25.0).abs() < 0.01);
    assert_eq!(subs[0].windows[1].window_id, "7d");
    assert_eq!(subs[0].windows[1].source, "official_api");

    // 3. Deleting an auto-discovered plan soft-dismisses it so future polls do not resurrect it.
    storage
        .delete_subscription_plan("auto_kimi_code")
        .expect("dismiss auto");
    assert!(storage
        .query_subscriptions(now_ms)
        .expect("after dismiss")
        .is_empty());

    storage
        .upsert_observed_rate_limits(&[ObservedRateLimitWindow {
            provider_id: provider_ids::KIMI_CODE.to_string(),
            window_name: "5h".to_string(),
            used_percent: 30.0,
            window_minutes: Some(300),
            resets_at_ms: Some(now_ms + 3_600_000),
            observed_at_ms: now_ms + 5_000,
            source: Some("official_api".to_string()),
            plan_hint: Some("LEVEL_INTERMEDIATE".to_string()),
        }])
        .expect("poll while dismissed");
    assert!(storage
        .query_subscriptions(now_ms + 5_000)
        .expect("remains dismissed")
        .is_empty());

    // 4. Re-configuring API key resets dismissal and re-discovers on next fetch.
    storage
        .reset_auto_subscription_dismissal(provider_ids::KIMI_CODE)
        .expect("reset dismissal");
    storage
        .upsert_observed_rate_limits(&[ObservedRateLimitWindow {
            provider_id: provider_ids::KIMI_CODE.to_string(),
            window_name: "5h".to_string(),
            used_percent: 35.0,
            window_minutes: Some(300),
            resets_at_ms: Some(now_ms + 3_600_000),
            observed_at_ms: now_ms + 10_000,
            source: Some("official_api".to_string()),
            plan_hint: Some("LEVEL_MAX".to_string()),
        }])
        .expect("poll after reset");
    let revived = storage
        .query_subscriptions(now_ms + 10_000)
        .expect("revived");
    assert_eq!(revived.len(), 1);
    assert_eq!(revived[0].plan.plan_name, "Kimi Ultra (Vivace)");
    assert_eq!(revived[0].plan.monthly_fee_micro_usd, 199_000_000);

    // 5. User customization preserves custom plan_name while windows keep refreshing from API.
    let mut custom = revived[0].plan.clone();
    custom.plan_name = "我的 Kimi 专享套餐".to_string();
    custom.monthly_fee_micro_usd = 7_000_000;
    storage
        .upsert_subscription_plan(&custom)
        .expect("customize auto");
    storage
        .upsert_observed_rate_limits(&[ObservedRateLimitWindow {
            provider_id: provider_ids::KIMI_CODE.to_string(),
            window_name: "5h".to_string(),
            used_percent: 50.0,
            window_minutes: Some(300),
            resets_at_ms: Some(now_ms + 3_600_000),
            observed_at_ms: now_ms + 20_000,
            source: Some("official_api".to_string()),
            plan_hint: Some("LEVEL_MAX".to_string()),
        }])
        .expect("poll after customize");
    let after_edit = storage
        .query_subscriptions(now_ms + 20_000)
        .expect("after edit");
    assert_eq!(after_edit[0].plan.plan_name, "我的 Kimi 专享套餐");
    assert_eq!(after_edit[0].plan.monthly_fee_micro_usd, 7_000_000);
    assert!((after_edit[0].windows[0].used_percent - 50.0).abs() < 0.01);
}

#[test]
fn kimi_auto_plan_without_level_keeps_zero_fee_and_toggle_keeps_it_auto() {
    let storage = Storage::open_in_memory().expect("db");
    let now_ms = Utc::now().timestamp_millis();
    let window = |name: &str, minutes: i64, pct: f64, observed_at_ms: i64, hint: Option<&str>| {
        ObservedRateLimitWindow {
            provider_id: provider_ids::KIMI_CODE.to_string(),
            window_name: name.to_string(),
            used_percent: pct,
            window_minutes: Some(minutes),
            resets_at_ms: Some(observed_at_ms + 3_600_000),
            observed_at_ms,
            source: Some("official_api".to_string()),
            plan_hint: hint.map(ToString::to_string),
        }
    };

    // The usages response carries no membership level: never guess a tier price.
    storage
        .upsert_observed_rate_limits(&[
            window("5h", 300, 10.0, now_ms, None),
            window("30d", 43_200, 79.0, now_ms, None),
        ])
        .expect("observe without level");
    let subs = storage.query_subscriptions(now_ms).expect("subs");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].plan.plan_name, "Kimi Code 会员订阅");
    assert_eq!(subs[0].plan.monthly_fee_micro_usd, 0);

    // Pausing the plan must not freeze it as user-customized, and polling must not resume it.
    storage
        .set_subscription_plan_enabled("auto_kimi_code", false)
        .expect("pause");
    storage
        .upsert_observed_rate_limits(&[window(
            "5h",
            300,
            20.0,
            now_ms + 1_000,
            Some("LEVEL_ADVANCED"),
        )])
        .expect("poll while paused");
    let plans = storage.list_subscription_plans().expect("plans");
    let kimi = plans
        .iter()
        .find(|p| p.plan_id == "auto_kimi_code")
        .expect("kimi plan");
    assert!(!kimi.enabled);
    assert_eq!(kimi.plan_name, "Kimi Max (Allegro)");
    assert_eq!(kimi.monthly_fee_micro_usd, 99_000_000);

    // Resuming keeps following official observations as well.
    storage
        .set_subscription_plan_enabled("auto_kimi_code", true)
        .expect("resume");
    storage
        .upsert_observed_rate_limits(&[window(
            "5h",
            300,
            30.0,
            now_ms + 2_000,
            Some("LEVEL_BASIC"),
        )])
        .expect("poll after resume");
    let subs = storage.query_subscriptions(now_ms + 2_000).expect("subs");
    assert!(subs[0].plan.enabled);
    assert_eq!(subs[0].plan.plan_name, "Kimi Plus (Moderato)");
    assert_eq!(subs[0].plan.monthly_fee_micro_usd, 19_000_000);

    assert!(storage
        .set_subscription_plan_enabled("missing_plan", true)
        .is_err());
}

#[test]
fn v6_migration_cleans_tps_preserves_remote_rollups_and_handles_stream_upgrades() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("v6.sqlite");
    let storage = Storage::open_file(&db_path).expect("open v6");
    let dev = storage.local_device().expect("device").device_id;

    // Preferences persist floating_window_enabled
    let mut prefs = storage.get_preferences().expect("prefs");
    assert!(!prefs.floating_window_enabled);
    prefs.floating_window_enabled = true;
    storage.update_preferences(&prefs).expect("save prefs");
    assert!(storage.get_preferences().expect("reloaded").floating_window_enabled);

    // Insert a remote pure-rollup device that has no rows in usage_events
    let bucket = 1_794_816_000_000i64;
    storage
        .with_conn(|conn| {
            conn.execute(
                "INSERT INTO devices (device_id, device_name, platform, created_at_ms) VALUES ('remote-mac', 'Mac', 'macos', 100)",
                [],
            )?;
            conn.execute(
                r#"
                INSERT INTO usage_hourly_rollups (
                    device_id, provider_id, model_id, bucket_start_ms,
                    request_count, observable_step_count, input_tokens, output_tokens,
                    cache_read_tokens, cache_write_tokens, reasoning_tokens,
                    estimated_cost_micro_usd, cache_saved_micro_usd,
                    total_duration_ms, tps_sample_count, tps_sum, max_output_tps, updated_at_ms
                ) VALUES ('remote-mac', 'claude_code', 'claude-sonnet-4-5', ?1, 2, 0, 1000, 200, 0, 0, 0, 5000, 0, 4000, 2, 100.0, 55.0, 100)
                "#,
                params![bucket],
            )?;
            Ok(())
        })
        .expect("seed remote");

    // Ingest a partial streaming chunk, then an upgraded final chunk + session title
    let partial = NormalizedUsageEvent {
        event_id: "claude_code:s1:msg1".to_string(),
        device_id: dev.clone(),
        provider_id: "claude_code".to_string(),
        session_id: "s1".to_string(),
        project_label: "TokenVane".to_string(),
        model_id: "claude-sonnet-4-5".to_string(),
        timestamp_ms: bucket + 1_000,
        input_tokens: 500,
        output_tokens: 20,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        reasoning_tokens: 0,
        observable_only: false,
        duration_ms: Some(400),
        ttft_ms: None,
        output_tps: Some(600.0), // invalid outlier (>450 + <500ms), must be dropped
        estimated_cost_micro_usd: 0,
        cache_saved_micro_usd: 0,
    };
    let title_derived = SessionTitleRecord {
        session_key: "claude_code:s1".to_string(),
        provider_id: "claude_code".to_string(),
        title: "<environment_context><cwd>/tmp</cwd></environment_context>Fix TPS spike"
            .to_string(),
        source: "derived".to_string(),
        updated_at_ms: bucket + 1_000,
    };
    storage
        .ingest_events_with_cursor_and_titles(&[partial], None, &[], &[title_derived])
        .expect("partial");

    let final_chunk = NormalizedUsageEvent {
        event_id: "claude_code:s1:msg1".to_string(),
        device_id: dev.clone(),
        provider_id: "claude_code".to_string(),
        session_id: "s1".to_string(),
        project_label: "TokenVane".to_string(),
        model_id: "claude-sonnet-4-5".to_string(),
        timestamp_ms: bucket + 3_000,
        input_tokens: 500,
        output_tokens: 180,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        reasoning_tokens: 0,
        observable_only: false,
        duration_ms: Some(3_000),
        ttft_ms: None,
        output_tps: Some(60.0),
        estimated_cost_micro_usd: 0,
        cache_saved_micro_usd: 0,
    };
    let title_explicit = SessionTitleRecord {
        session_key: "claude_code:s1".to_string(),
        provider_id: "claude_code".to_string(),
        title: "Final Explicit Session Title".to_string(),
        source: "explicit".to_string(),
        updated_at_ms: bucket + 3_000,
    };
    storage
        .ingest_events_with_cursor_and_titles(&[final_chunk], None, &[], &[title_explicit])
        .expect("final");

    storage.recompute_all_costs_and_rollups().expect("recompute");

    storage
        .with_conn(|conn| {
            let (out_tok, tps): (i64, Option<f64>) = conn.query_row(
                "SELECT output_tokens, output_tps FROM usage_events WHERE event_id = 'claude_code:s1:msg1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            assert_eq!(out_tok, 180);
            assert_eq!(tps, Some(60.0));

            let local_reqs: i64 = conn.query_row(
                "SELECT request_count FROM usage_hourly_rollups WHERE device_id = ?1",
                params![dev],
                |r| r.get(0),
            )?;
            assert_eq!(local_reqs, 1);

            // Remote rollup must still exist intact!
            let remote_reqs: i64 = conn.query_row(
                "SELECT request_count FROM usage_hourly_rollups WHERE device_id = 'remote-mac'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(remote_reqs, 2);

            let stored_title: String = conn.query_row(
                "SELECT title FROM session_titles WHERE session_key = 'claude_code:s1'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(stored_title, "Final Explicit Session Title");
            Ok(())
        })
        .expect("verify");
}

#[test]
fn antigravity_auto_subscription_monitors_5h_and_7d_rolling_windows() {
    let storage = Storage::open_in_memory().expect("db");
    let dev = storage.local_device().expect("device").device_id;
    let now_ms = 1_794_820_000_000i64;

    let ev = NormalizedUsageEvent {
        event_id: "antigravity:conv-1:step-1".to_string(),
        device_id: dev,
        provider_id: provider_ids::ANTIGRAVITY.to_string(),
        session_id: "conv-1".to_string(),
        project_label: "antigravity".to_string(),
        model_id: "Orchestrator (Carbon)".to_string(),
        timestamp_ms: now_ms - 60_000,
        input_tokens: 1_000_000,
        output_tokens: 100_000,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        reasoning_tokens: 0,
        observable_only: false,
        duration_ms: Some(4_000),
        ttft_ms: Some(600),
        output_tps: Some(25.0),
        estimated_cost_micro_usd: 0,
        cache_saved_micro_usd: 0,
    };
    let hint_obs = ObservedRateLimitWindow {
        provider_id: provider_ids::ANTIGRAVITY.to_string(),
        window_name: "tier_hint".to_string(),
        used_percent: 0.0,
        window_minutes: None,
        resets_at_ms: None,
        observed_at_ms: now_ms - 60_000,
        source: Some("local_log_hint".to_string()),
        plan_hint: Some("ultra_5x".to_string()),
    };

    storage
        .ingest_events_with_cursor(&[ev], None, &[hint_obs])
        .expect("ingest antigravity");

    // Internal tier_hint rows must NOT leak into list_observed_rate_limits()
    assert!(storage
        .list_observed_rate_limits()
        .expect("observed")
        .is_empty());

    let subs = storage.query_subscriptions(now_ms).expect("subs");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].plan.plan_id, "auto_antigravity");
    assert_eq!(subs[0].plan.plan_name, "Google AI Ultra 5x");
    assert_eq!(subs[0].plan.monthly_fee_micro_usd, 99_990_000);
    assert_eq!(subs[0].plan.auto_source.as_deref(), Some("local_log"));
    // Antigravity has no official quota API, so misleading local_estimation windows must be empty.
    // Instead, factual API-equivalent spend ($3.20) is reported accurately.
    assert!(subs[0].windows.is_empty());
    assert_eq!(subs[0].today_cost_micro_usd, 3_200_000);
    assert_eq!(subs[0].month_cost_micro_usd, 3_200_000);
    assert_eq!(subs[0].month_tokens, 1_100_000);
}

#[test]
fn v9_fixes_session_title_lock_antigravity_false_pro_and_payg_accounts() {
    let storage = Storage::open_in_memory().expect("db");
    let dev = storage.local_device().expect("device").device_id;
    let now_ms = 1_795_000_000_000i64;

    // 1. Antigravity event with Gemini Pro model (WITHOUT Carbon/Ultra or explicit tier hint)
    //    MUST NOT fabricate "Google AI Pro".
    let pro_ev = NormalizedUsageEvent {
        event_id: "ag-pro-model-only".to_string(),
        device_id: dev.clone(),
        provider_id: provider_ids::ANTIGRAVITY.to_string(),
        session_id: "conv-gemini".to_string(),
        project_label: "antigravity".to_string(),
        model_id: "Gemini 3.1 Pro (High)".to_string(),
        timestamp_ms: now_ms - 120_000,
        input_tokens: 100_000,
        output_tokens: 20_000,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        reasoning_tokens: 0,
        observable_only: false,
        duration_ms: Some(3_000),
        ttft_ms: None,
        output_tps: Some(25.0),
        estimated_cost_micro_usd: 0,
        cache_saved_micro_usd: 0,
    };
    let first_prompt = SessionTitleRecord {
        session_key: "antigravity:conv-gemini".to_string(),
        provider_id: provider_ids::ANTIGRAVITY.to_string(),
        title: "First user prompt for this session".to_string(),
        source: "user_prompt".to_string(),
        updated_at_ms: now_ms - 120_000,
    };
    let later_prompt = SessionTitleRecord {
        session_key: "antigravity:conv-gemini".to_string(),
        provider_id: provider_ids::ANTIGRAVITY.to_string(),
        title: "Second user prompt much later".to_string(),
        source: "user_prompt".to_string(),
        updated_at_ms: now_ms - 10_000,
    };
    storage
        .ingest_events_with_cursor_and_titles(&[pro_ev], None, &[], &[first_prompt, later_prompt])
        .expect("ingest gemini pro");

    assert!(
        storage
            .query_subscriptions(now_ms)
            .expect("subs")
            .is_empty(),
        "Gemini Pro model alone must not fabricate Google AI Pro auto subscription"
    );

    let stored_title: String = storage
        .with_conn(|conn| {
            conn.query_row(
                "SELECT title FROM session_titles WHERE session_key = 'antigravity:conv-gemini'",
                [],
                |r| r.get(0),
            )
            .map_err(CoreError::from)
        })
        .expect("title query");
    assert_eq!(stored_title, "First user prompt for this session");

    // Explicit rename (e.g. session_index.jsonl) overrides user_prompt, and newer explicit rename wins
    storage
        .upsert_session_titles(&[SessionTitleRecord {
            session_key: "antigravity:conv-gemini".to_string(),
            provider_id: provider_ids::ANTIGRAVITY.to_string(),
            title: "Explicit Thread Name v1".to_string(),
            source: "explicit".to_string(),
            updated_at_ms: now_ms - 5_000,
        }])
        .expect("explicit v1");
    storage
        .upsert_session_titles(&[SessionTitleRecord {
            session_key: "antigravity:conv-gemini".to_string(),
            provider_id: provider_ids::ANTIGRAVITY.to_string(),
            title: "Explicit Thread Name v2".to_string(),
            source: "explicit".to_string(),
            updated_at_ms: now_ms,
        }])
        .expect("explicit v2");
    let renamed: String = storage
        .with_conn(|conn| {
            conn.query_row(
                "SELECT title FROM session_titles WHERE session_key = 'antigravity:conv-gemini'",
                [],
                |r| r.get(0),
            )
            .map_err(CoreError::from)
        })
        .expect("renamed query");
    assert_eq!(renamed, "Explicit Thread Name v2");

    // 2. Orchestrator (Carbon) infers Google AI Ultra (without any hint_obs)
    let carbon_ev = NormalizedUsageEvent {
        event_id: "ag-carbon-model".to_string(),
        device_id: dev.clone(),
        provider_id: provider_ids::ANTIGRAVITY.to_string(),
        session_id: "conv-carbon".to_string(),
        project_label: "antigravity".to_string(),
        model_id: "Orchestrator (Carbon)".to_string(),
        timestamp_ms: now_ms - 30_000,
        input_tokens: 100_000,
        output_tokens: 20_000,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        reasoning_tokens: 0,
        observable_only: false,
        duration_ms: Some(3_000),
        ttft_ms: None,
        output_tps: Some(25.0),
        estimated_cost_micro_usd: 0,
        cache_saved_micro_usd: 0,
    };
    storage
        .ingest_events_with_cursor(&[carbon_ev], None, &[])
        .expect("ingest carbon");
    let subs = storage.query_subscriptions(now_ms).expect("ultra sub");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].plan.plan_name, "Google AI Ultra");
    assert_eq!(subs[0].plan.monthly_fee_micro_usd, 199_990_000);

    // 3. PAYG auto-discovery from Codex Credits + DeepSeek CNY + manual budget spend tracking
    let codex_ev = NormalizedUsageEvent {
        event_id: "codex-payg-ev".to_string(),
        device_id: dev,
        provider_id: provider_ids::CODEX.to_string(),
        session_id: "s-payg".to_string(),
        project_label: "app".to_string(),
        model_id: "gpt-5-codex".to_string(),
        timestamp_ms: now_ms - 5_000,
        input_tokens: 1_000_000,
        output_tokens: 100_000,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        reasoning_tokens: 0,
        observable_only: false,
        duration_ms: None,
        ttft_ms: None,
        output_tps: None,
        estimated_cost_micro_usd: 2_500_000,
        cache_saved_micro_usd: 0,
    };
    storage
        .ingest_events_with_cursor(&[codex_ev], None, &[])
        .expect("codex ev");

    storage
        .upsert_provider_balance_if_newer(&ProviderBalanceSnapshot {
            provider_id: provider_ids::CODEX.to_string(),
            currency: "CREDITS".to_string(),
            total_balance_micro: Some(766_760_000),
            granted_balance_micro: None,
            topped_up_balance_micro: None,
            daily_quota_used_micro: None,
            daily_quota_limit_micro: None,
            fetched_at_ms: now_ms - 2_000,
        })
        .expect("codex credits");
    storage
        .upsert_provider_balance(&ProviderBalanceSnapshot {
            provider_id: provider_ids::DEEPSEEK_HARNESS.to_string(),
            currency: "CNY".to_string(),
            total_balance_micro: Some(45_200_000),
            granted_balance_micro: Some(10_000_000),
            topped_up_balance_micro: Some(35_200_000),
            daily_quota_used_micro: None,
            daily_quota_limit_micro: None,
            fetched_at_ms: now_ms - 1_000,
        })
        .expect("deepseek balance");
    storage
        .upsert_payg_account(&PaygAccount {
            account_id: "payg_claude_manual".to_string(),
            provider_id: provider_ids::CLAUDE_CODE.to_string(),
            account_name: "Anthropic Console".to_string(),
            enabled: true,
            manual_balance_micro_usd: Some(50_000_000),
            balance_anchor_ms: Some(now_ms - 60_000),
            monthly_budget_micro_usd: Some(100_000_000),
            is_auto: false,
            updated_at_ms: now_ms,
        })
        .expect("manual payg");

    let payg_list = storage.query_payg_accounts(now_ms, 480).expect("query payg");
    assert_eq!(payg_list.len(), 3);
    let codex_payg = payg_list
        .iter()
        .find(|p| p.account.provider_id == provider_ids::CODEX)
        .expect("codex payg");
    assert_eq!(codex_payg.account.account_name, "Codex Credits");
    assert_eq!(codex_payg.month_cost_micro_usd, 2_500_000);
    assert_eq!(
        codex_payg
            .official_balance
            .as_ref()
            .and_then(|b| b.total_balance_micro),
        Some(766_760_000)
    );
}

#[test]
fn v10_migration_and_subscription_catalog_sync_work_end_to_end() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("v9_to_v10.sqlite");
    let now_ms = 1_795_000_000_000i64;

    let storage = Storage::open_file(&db_path).expect("seed base db");
    let dev = storage.local_device().expect("device").device_id;
    storage
        .with_conn(|conn| {
            conn.execute(
                r#"
                INSERT INTO usage_events (
                    event_id, device_id, provider_id, session_id, project_label,
                    model_id, timestamp_ms, input_tokens, output_tokens,
                    cache_read_tokens, cache_write_tokens, reasoning_tokens,
                    observable_only, duration_ms, ttft_ms, output_tps,
                    estimated_cost_micro_usd, cache_saved_micro_usd
                ) VALUES (
                    'stale-codex-ev', ?1, 'codex', 's-real', 'proj',
                    'gpt-5-codex', ?2, 1000, 100, 0, 0, 0, 0, NULL, NULL, NULL, 5000, 0
                )
                "#,
                params![dev, now_ms],
            )?;
            conn.execute(
                r#"
                INSERT INTO ingestion_cursors (
                    source_path, provider_id, file_identity, byte_offset, last_line_hash, updated_at_ms
                ) VALUES ('C:/tmp/rollout-1.jsonl', 'codex', 'id-1', 1234, 'abc', ?1)
                "#,
                params![now_ms],
            )?;
            conn.execute(
                r#"
                INSERT INTO session_titles (session_key, provider_id, title, source, updated_at_ms)
                VALUES ('codex:s-bg', 'codex', 'Guardian review', 'explicit', ?1)
                "#,
                params![now_ms],
            )?;
            conn.execute("DELETE FROM schema_migrations WHERE version >= 10", [])?;
            Ok(())
        })
        .expect("inject pre-v10 data");
    drop(storage);

    let reopened = Storage::open_file(&db_path).expect("run v10 migration");
    reopened
        .with_conn(|conn| {
            let stale_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM usage_events WHERE event_id = 'stale-codex-ev'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(stale_count, 0);

            let cursor_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM ingestion_cursors WHERE provider_id = 'codex'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(cursor_count, 0);

            let bg_source: String = conn.query_row(
                "SELECT source FROM session_titles WHERE session_key = 'codex:s-bg'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(bg_source, "background");
            Ok(())
        })
        .expect("verify v10 migration");

    let mut synced = subscription_plan_templates();
    synced.push(SubscriptionPlanTemplate {
        template_id: "cursor_pro_plus".to_string(),
        provider_id: provider_ids::CURSOR.to_string(),
        plan_name: "Cursor Pro+".to_string(),
        monthly_fee_micro_usd: 60_000_000,
        windows: Vec::new(),
        description: "Cursor Pro+ 60 USD".to_string(),
        live_windows: Vec::new(),
    });
    let (changed, count) = reopened
        .apply_synced_subscription_catalog(&synced, now_ms + 1_000)
        .expect("sync sub catalog");
    assert!(changed);
    assert_eq!(count, synced.len());

    // Second identical catalog sync short-circuits with changed = false
    let (changed_again, _) = reopened
        .apply_synced_subscription_catalog(&synced, now_ms + 2_000)
        .expect("idempotent sub catalog");
    assert!(!changed_again);

    let templates = reopened
        .query_subscription_templates(now_ms + 3_000)
        .expect("templates include synced cursor_pro_plus");
    assert!(templates.iter().any(|t| t.template_id == "cursor_pro_plus"));
    assert_eq!(
        reopened
            .get_preferences()
            .expect("prefs")
            .last_subscription_sync_at_ms,
        Some(now_ms + 2_000)
    );
}

#[test]
fn claude_code_official_cli_usage_auto_seeds_pro_plan_with_live_windows() {
    let storage = Storage::open_in_memory().expect("in-memory");
    let now_ms = 1_794_840_000_000i64;
    let cli_window = |name: &str, pct: f64, mins: i64, reset_in: i64| ObservedRateLimitWindow {
        provider_id: provider_ids::CLAUDE_CODE.to_string(),
        window_name: name.to_string(),
        used_percent: pct,
        window_minutes: Some(mins),
        resets_at_ms: Some(now_ms + reset_in),
        observed_at_ms: now_ms,
        source: Some("official_cli".to_string()),
        plan_hint: Some("pro".to_string()),
    };

    storage
        .replace_provider_source_rate_limits(
            provider_ids::CLAUDE_CODE,
            "official_cli",
            &[
                cli_window("5h", 49.0, 300, 3_600_000),
                cli_window("7d", 7.0, 10_080, 5 * 86_400_000),
            ],
        )
        .expect("replace official_cli windows");

    let subs = storage.query_subscriptions(now_ms).expect("subs");
    let claude: Vec<_> = subs
        .iter()
        .filter(|s| s.plan.provider_id == provider_ids::CLAUDE_CODE)
        .collect();
    assert_eq!(claude.len(), 1);
    let plan = &claude[0].plan;
    assert_eq!(plan.plan_id, "auto_claude_code");
    assert_eq!(plan.plan_name, "Claude Pro");
    assert_eq!(plan.monthly_fee_micro_usd, 20_000_000);
    assert_eq!(plan.auto_source.as_deref(), Some("official_cli"));
    assert_eq!(claude[0].windows.len(), 2);
    assert_eq!(claude[0].windows[0].window_id, "5h");
    assert_eq!(claude[0].windows[0].source, "official_cli");
    assert!((claude[0].windows[0].used_percent - 49.0).abs() < 1e-9);
    assert_eq!(claude[0].windows[1].window_id, "7d");
    assert!((claude[0].windows[1].used_percent - 7.0).abs() < 1e-9);
}

#[test]
fn antigravity_official_cli_rate_limits_auto_seed_and_upgrade_with_carbon_usage() {
    let storage = Storage::open_in_memory().expect("in-memory");
    let dev = storage.local_device().expect("device").device_id;
    let now_ms = 1_794_840_000_000i64;

    let cli_windows = vec![
        ObservedRateLimitWindow {
            provider_id: provider_ids::ANTIGRAVITY.to_string(),
            window_name: "gemini-5h".to_string(),
            used_percent: 36.5,
            window_minutes: Some(300),
            resets_at_ms: Some(now_ms + 3_600_000),
            observed_at_ms: now_ms,
            source: Some("official_cli".to_string()),
            plan_hint: None,
        },
        ObservedRateLimitWindow {
            provider_id: provider_ids::ANTIGRAVITY.to_string(),
            window_name: "gemini-weekly".to_string(),
            used_percent: 63.9,
            window_minutes: Some(10_080),
            resets_at_ms: Some(now_ms + 86_400_000),
            observed_at_ms: now_ms,
            source: Some("official_cli".to_string()),
            plan_hint: None,
        },
        ObservedRateLimitWindow {
            provider_id: provider_ids::ANTIGRAVITY.to_string(),
            window_name: "3p-5h".to_string(),
            used_percent: 0.0,
            window_minutes: Some(300),
            resets_at_ms: Some(now_ms + 7_200_000),
            observed_at_ms: now_ms,
            source: Some("official_cli".to_string()),
            plan_hint: None,
        },
        ObservedRateLimitWindow {
            provider_id: provider_ids::ANTIGRAVITY.to_string(),
            window_name: "3p-weekly".to_string(),
            used_percent: 0.45,
            window_minutes: Some(10_080),
            resets_at_ms: Some(now_ms + 90_000_000),
            observed_at_ms: now_ms,
            source: Some("official_cli".to_string()),
            plan_hint: None,
        },
    ];

    storage
        .replace_provider_source_rate_limits(
            provider_ids::ANTIGRAVITY,
            "official_cli",
            &cli_windows,
        )
        .expect("replace official_cli windows");

    let subs = storage.query_subscriptions(now_ms).expect("subs");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].plan.plan_id, "auto_antigravity");
    assert_eq!(subs[0].plan.auto_source.as_deref(), Some("official_cli"));
    // Must not falsely assume Pro tier when no tier hint is present
    assert_eq!(subs[0].plan.plan_name, "Antigravity 官方配额");
    assert_eq!(subs[0].plan.monthly_fee_micro_usd, 0);
    assert_eq!(subs[0].windows.len(), 4);
    assert_eq!(subs[0].windows[0].window_id, "gemini-5h");
    assert_eq!(subs[0].windows[0].source, "official_cli");
    assert_eq!(subs[0].windows[3].window_id, "3p-weekly");

    // Record Carbon model usage and verify next CLI sync promotes auto plan to Google AI Ultra
    storage
        .ingest_events_with_cursor(
            &[NormalizedUsageEvent {
                event_id: "agy-carbon-1".to_string(),
                device_id: dev,
                provider_id: provider_ids::ANTIGRAVITY.to_string(),
                session_id: "s-carbon".to_string(),
                project_label: "TokenVane".to_string(),
                model_id: "Gemini 4 Carbon (High)".to_string(),
                timestamp_ms: now_ms,
                input_tokens: 1_000,
                output_tokens: 500,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: Some(1_000),
                ttft_ms: None,
                output_tps: Some(50.0),
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            }],
            None,
            &[],
        )
        .expect("ingest carbon event");

    storage
        .replace_provider_source_rate_limits(
            provider_ids::ANTIGRAVITY,
            "official_cli",
            &cli_windows[..2],
        )
        .expect("replace with 2 buckets");
    let updated_subs = storage.query_subscriptions(now_ms).expect("updated subs");
    assert_eq!(updated_subs[0].plan.plan_name, "Google AI Ultra");
    // Previously removed 3p buckets must no longer linger after replace_provider_source_rate_limits
    assert_eq!(updated_subs[0].windows.len(), 2);
}

#[test]
fn v11_migration_purges_manual_subscriptions_and_keeps_authoritative_auto_plans() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("v10_to_v11.sqlite");
    let now_ms = 1_795_100_000_000i64;

    let storage = Storage::open_file(&db_path).expect("seed base db");
    storage
        .upsert_subscription_plan(&SubscriptionPlan {
            plan_id: "manual_claude_max".to_string(),
            provider_id: provider_ids::CLAUDE_CODE.to_string(),
            plan_name: "Claude Max 5x".to_string(),
            enabled: true,
            monthly_fee_micro_usd: 100_000_000,
            windows: Vec::new(),
            is_user_configured: true,
            auto_source: None,
            updated_at_ms: now_ms,
        })
        .expect("insert manual plan");
    storage
        .upsert_observed_rate_limits(&[ObservedRateLimitWindow {
            provider_id: provider_ids::KIMI_CODE.to_string(),
            window_name: "5h".to_string(),
            used_percent: 20.0,
            window_minutes: Some(300),
            resets_at_ms: Some(now_ms + 3_600_000),
            observed_at_ms: now_ms,
            source: Some("official_api".to_string()),
            plan_hint: Some("LEVEL_MAX".to_string()),
        }])
        .expect("seed auto kimi plan");
    storage
        .with_conn(|conn| {
            conn.execute("DELETE FROM schema_migrations WHERE version >= 11", [])?;
            Ok(())
        })
        .expect("rewind to v10");
    drop(storage);

    let migrated = Storage::open_file(&db_path).expect("run v11 migration");
    let subs = migrated.query_subscriptions(now_ms).expect("query subs");
    assert!(!subs.iter().any(|s| s.plan.plan_id == "manual_claude_max"));
    assert!(subs.iter().any(|s| s.plan.plan_id == "auto_kimi_code"));
}

#[test]
fn v12_migration_heals_poisoned_antigravity_gemini_3_1_models_and_resets_cursor() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("v11_to_v12.sqlite");
    let now_ms = 1_795_200_000_000i64;

    let storage = Storage::open_file(&db_path).expect("seed base db");
    let dev = storage.local_device().expect("device").device_id;
    let poisoned_ev = NormalizedUsageEvent {
        event_id: "agy-poison-31".to_string(),
        device_id: dev.clone(),
        provider_id: provider_ids::ANTIGRAVITY.to_string(),
        session_id: "conv-poison".to_string(),
        project_label: "TokenVane".to_string(),
        model_id: "Gemini 3.1 Pro (High)".to_string(),
        timestamp_ms: now_ms - 60_000,
        input_tokens: 4_000,
        output_tokens: 1_000,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        reasoning_tokens: 0,
        observable_only: false,
        duration_ms: Some(1_200),
        ttft_ms: None,
        output_tps: Some(40.0),
        estimated_cost_micro_usd: 15_000,
        cache_saved_micro_usd: 0,
    };
    let cursor = IngestionCursor {
        source_path: "/tmp/conv-poison/transcript.jsonl".to_string(),
        provider_id: provider_ids::ANTIGRAVITY.to_string(),
        file_identity: "inode-31".to_string(),
        byte_offset: 1024,
        last_line_hash: "hash-31".to_string(),
        updated_at_ms: now_ms - 60_000,
    };
    storage
        .ingest_events_with_cursor(std::slice::from_ref(&poisoned_ev), Some(&cursor), &[])
        .expect("seed poisoned event and cursor");

    storage
        .with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO model_pricing (
                    model_pattern, provider_id, input_per_m_micro_usd, output_per_m_micro_usd,
                    cache_read_per_m_micro_usd, cache_write_per_m_micro_usd, reasoning_per_m_micro_usd,
                    is_user_override, effective_from
                ) VALUES ('gemini-3.1-pro', 'antigravity', 2000000, 12000000, 200000, 0, 12000000, 0, '2026-01-01')",
                [],
            )?;
            conn.execute("DELETE FROM schema_migrations WHERE version >= 12", [])?;
            Ok(())
        })
        .expect("rewind to v11");
    drop(storage);

    let migrated = Storage::open_file(&db_path).expect("run v12 migration");
    migrated
        .with_conn(|conn| {
            let healed_model: String = conn.query_row(
                "SELECT model_id FROM usage_events WHERE event_id = 'agy-poison-31'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(healed_model, "antigravity-default");

            let poisoned_rollups: i64 = conn.query_row(
                "SELECT COUNT(*) FROM usage_daily_rollups WHERE provider_id = 'antigravity' AND LOWER(model_id) LIKE '%gemini 3.1%'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(poisoned_rollups, 0);

            let agy_cursors: i64 = conn.query_row(
                "SELECT COUNT(*) FROM ingestion_cursors WHERE provider_id = 'antigravity'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(agy_cursors, 0);

            let obsolete_pricing: i64 = conn.query_row(
                "SELECT COUNT(*) FROM model_pricing WHERE model_pattern = 'gemini-3.1-pro'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(obsolete_pricing, 0);
            Ok(())
        })
        .expect("verify v12 cleanup");

    // Re-ingesting the same event with its corrected real model upgrades both the event and rollups.
    let mut repaired_ev = poisoned_ev;
    repaired_ev.model_id = "Gemini 4 Carbon (High)".to_string();
    migrated
        .ingest_events_with_cursor(&[repaired_ev], None, &[])
        .expect("re-ingest repaired model");
    migrated
        .with_conn(|conn| {
            let updated_model: String = conn.query_row(
                "SELECT model_id FROM usage_events WHERE event_id = 'agy-poison-31'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(updated_model, "Gemini 4 Carbon (High)");

            let carbon_rollup: i64 = conn.query_row(
                "SELECT COUNT(*) FROM usage_daily_rollups WHERE provider_id = 'antigravity' AND model_id = 'Gemini 4 Carbon (High)'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(carbon_rollup, 1);
            Ok(())
        })
        .expect("verify re-ingest upgrade");
}

#[test]
fn v12_to_v13_migration_clears_local_codex_end_to_end_speed_and_resets_cursors() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("v12.db");
    let ts = 1_793_178_458_000i64;
    {
        let storage = Storage::open_file(&db_path).expect("open current");
        let local = storage.local_device().expect("device").device_id;
        storage
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO devices (device_id, device_name, platform, created_at_ms) VALUES ('remote-dev', 'mac', 'macos', ?1)",
                    params![ts + 10_000_000_000],
                )?;
                for (id, dev, pid) in [
                    ("ev-codex-local", local.as_str(), "codex"),
                    ("ev-codex-remote", "remote-dev", "codex"),
                    ("ev-claude-local", local.as_str(), "claude_code"),
                ] {
                    conn.execute(
                        "INSERT INTO usage_events (event_id, device_id, provider_id, session_id, project_label, model_id, timestamp_ms, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens, observable_only, duration_ms, ttft_ms, output_tps, estimated_cost_micro_usd, cache_saved_micro_usd) VALUES (?1, ?2, ?3, 's', 'p', 'gpt-6-astra', ?4, 1000, 370, 0, 0, 0, 0, 20000, 3000, 18.5, 0, 0)",
                        params![id, dev, pid, ts],
                    )?;
                }
                for (path, pid) in [("C:/codex/rollout-1.jsonl", "codex"), ("C:/claude/a.jsonl", "claude_code")] {
                    conn.execute(
                        "INSERT INTO ingestion_cursors (source_path, provider_id, file_identity, byte_offset, last_line_hash, updated_at_ms) VALUES (?1, ?2, 'id', 4096, 'h', ?3)",
                        params![path, pid, ts],
                    )?;
                }
                conn.execute("DELETE FROM schema_migrations WHERE version >= 13", [])?;
                Ok(())
            })
            .expect("seed v12 state");
    }

    let storage = Storage::open_file(&db_path).expect("migrate v12 -> v14");
    storage
        .with_conn(|conn| {
            let ver: i64 =
                conn.query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))?;
            assert_eq!(ver, crate::storage::migrations::CURRENT_SCHEMA_VERSION);
            let has_v13: i64 = conn.query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 13",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(has_v13, 1);
            let timing = |id: &str| -> rusqlite::Result<(Option<i64>, Option<i64>, Option<f64>)> {
                conn.query_row(
                    "SELECT duration_ms, ttft_ms, output_tps FROM usage_events WHERE event_id = ?1",
                    params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
            };
            assert_eq!(timing("ev-codex-local")?, (None, None, None));
            assert_eq!(timing("ev-codex-remote")?, (Some(20_000), Some(3_000), Some(18.5)));
            assert_eq!(timing("ev-claude-local")?, (Some(20_000), Some(3_000), Some(18.5)));
            let cursors: Vec<String> = conn
                .prepare("SELECT provider_id FROM ingestion_cursors ORDER BY provider_id")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            assert_eq!(cursors, vec!["claude_code".to_string()]);
            Ok(())
        })
        .expect("verify v13");
}

#[test]
fn v13_to_v14_migration_adds_remote_subscription_fee_and_payg_snapshots() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("v13.db");
    {
        let storage = Storage::open_file(&db_path).expect("open current");
        storage
            .with_conn(|conn| {
                conn.execute("DROP TABLE IF EXISTS remote_payg_snapshots", [])?;
                conn.execute("DELETE FROM schema_migrations WHERE version >= 14", [])?;
                Ok(())
            })
            .expect("rewind to v13");
    }

    let storage = Storage::open_file(&db_path).expect("migrate v13 -> v14");
    storage
        .with_conn(|conn| {
            let ver: i64 =
                conn.query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))?;
            assert_eq!(ver, crate::storage::migrations::CURRENT_SCHEMA_VERSION);
            let has_fee_col: i64 = conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('remote_subscription_snapshots') WHERE name = 'monthly_fee_micro_usd'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(has_fee_col, 1);
            let has_payg_table: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'remote_payg_snapshots'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(has_payg_table, 1);
            Ok(())
        })
        .expect("verify v14");
}

#[test]
fn v14_to_v15_migration_adds_background_cli_quota_poll_and_cli_auth_pause() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("v14.db");
    {
        let storage = Storage::open_file(&db_path).expect("open current");
        storage
            .with_conn(|conn| {
                conn.execute(
                    "ALTER TABLE app_preferences DROP COLUMN background_cli_quota_poll",
                    [],
                )?;
                conn.execute(
                    "ALTER TABLE provider_configs DROP COLUMN cli_auth_paused_reason",
                    [],
                )?;
                conn.execute("DELETE FROM schema_migrations WHERE version >= 15", [])?;
                Ok(())
            })
            .expect("rewind to v14");
    }

    let storage = Storage::open_file(&db_path).expect("migrate v14 -> v15");
    storage
        .with_conn(|conn| {
            let ver: i64 =
                conn.query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))?;
            assert_eq!(ver, crate::storage::migrations::CURRENT_SCHEMA_VERSION);
            let has_col: i64 = conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('app_preferences') WHERE name = 'background_cli_quota_poll'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(has_col, 1);
            let has_pause_col: i64 = conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('provider_configs') WHERE name = 'cli_auth_paused_reason'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(has_pause_col, 1);
            Ok(())
        })
        .expect("verify v15 schema");

    let mut prefs = storage.get_preferences().expect("prefs");
    assert!(prefs.background_cli_quota_poll);
    prefs.background_cli_quota_poll = false;
    storage.update_preferences(&prefs).expect("disable cli poll");
    assert!(!storage
        .get_preferences()
        .expect("reload disabled")
        .background_cli_quota_poll);
    prefs.background_cli_quota_poll = true;
    storage.update_preferences(&prefs).expect("enable cli poll");
    assert!(storage
        .get_preferences()
        .expect("reload enabled")
        .background_cli_quota_poll);

    assert_eq!(
        storage
            .get_cli_auth_paused_reason(provider_ids::ANTIGRAVITY)
            .expect("initial auth pause"),
        None
    );
    storage
        .set_cli_auth_paused_reason(provider_ids::ANTIGRAVITY, Some("  agy CLI Not signed in  "))
        .expect("set auth pause");
    assert_eq!(
        storage
            .get_cli_auth_paused_reason(provider_ids::ANTIGRAVITY)
            .expect("persisted auth pause")
            .as_deref(),
        Some("agy CLI Not signed in")
    );
    // Regular sync status updates must not clear cli_auth_paused_reason.
    storage
        .set_provider_sync_status(provider_ids::ANTIGRAVITY, Some(12345), None)
        .expect("set sync status");
    assert_eq!(
        storage
            .get_cli_auth_paused_reason(provider_ids::ANTIGRAVITY)
            .expect("retained auth pause")
            .as_deref(),
        Some("agy CLI Not signed in")
    );
    storage
        .set_cli_auth_paused_reason(provider_ids::ANTIGRAVITY, Some("   "))
        .expect("blank clears auth pause");
    assert_eq!(
        storage
            .get_cli_auth_paused_reason(provider_ids::ANTIGRAVITY)
            .expect("cleared auth pause"),
        None
    );
}
