use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tempfile::tempdir;
use tokvane_collectors::otlp::{
    ingest_otlp_logs_payload, ingest_otlp_metrics_payload, start_loopback_otlp_server,
    validate_loopback_addr,
};
use tokvane_collectors::providers::antigravity::AntigravityParser;
use tokvane_collectors::providers::claude_code::{read_claude_stats_cache, ClaudeCodeParser};
use tokvane_collectors::providers::codex::CodexParser;
use tokvane_collectors::providers::deepseek_harness::DeepSeekHarnessParser;
use tokvane_collectors::providers::kimi_code::KimiCodeParser;
use tokvane_collectors::providers::minimax_code::MiniMaxCodeParser;
use tokvane_collectors::providers::qcode::QCodeParser;
use tokvane_collectors::providers::workbuddy::WorkbuddyParser;
use tokvane_collectors::watcher::{discover_files_for_parser, tail_file_incremental};
use tokvane_collectors::{
    refresh_provider_balance_with_fetcher, AuthHttpFetcher, BalanceApiEndpoints,
    CollectorCoordinator, MemoryCredentialStore, CredentialManager,
};
use tokvane_core::analytics::AnalyticsFilter;
use tokvane_core::domain::{
    provider_ids, CustomProtocolKind, CustomProviderRule, DeclarativeMappingSpec,
};
use tokvane_core::Storage;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

#[test]
fn replays_claude_codex_and_antigravity_fixtures_accurately() {
    let storage = Storage::open_in_memory().expect("in-memory db");
    let dev_id = storage.local_device().expect("device").device_id;
    let fx = fixtures_dir();

    // 1. Claude Code
    let cc_parser = ClaudeCodeParser::new();
    let cc_file = fx.join("claude_code").join("project_alpha").join("sess-cc-001.jsonl");
    let cc_out = tail_file_incremental(&storage, &cc_parser, &cc_file, &dev_id).expect("cc tail");
    assert_eq!(cc_out.events_inserted, 2);

    let stats_cache =
        read_claude_stats_cache(&fx.join("claude_code").join("stats-cache.json"))
            .expect("read stats")
            .expect("some stats");
    assert_eq!(stats_cache.total_sessions, 12);
    assert_eq!(stats_cache.total_messages, 480);

    // 2. Codex
    let codex_parser = CodexParser::new();
    let codex_file = fx
        .join("codex")
        .join("2026")
        .join("11")
        .join("15")
        .join("rollout-2026-11-15T10-05-00-sess-codex-001.jsonl");
    let codex_out =
        tail_file_incremental(&storage, &codex_parser, &codex_file, &dev_id).expect("codex tail");
    assert_eq!(codex_out.events_inserted, 2);

    let rate_limits = storage.list_observed_rate_limits().expect("rate limits");
    assert_eq!(rate_limits.len(), 2);
    assert!(rate_limits
        .iter()
        .any(|r| r.window_name == "primary" && (r.used_percent - 35.5).abs() < 0.01));

    // 3. Antigravity (must skip transcript_full.jsonl)
    let ag_parser = AntigravityParser::new();
    let ag_root = fx.join("antigravity");
    let ag_cfg = tokvane_core::domain::ProviderConfig {
        provider_id: provider_ids::ANTIGRAVITY.to_string(),
        enabled: true,
        collection_mode: tokvane_core::domain::CollectionMode::LocalLog,
        custom_log_paths: vec![ag_root.to_string_lossy().to_string()],
        has_credential: false,
        last_sync_at_ms: None,
        last_error: None,
        updated_at_ms: 0,
    };
    let ag_files = discover_files_for_parser(&ag_parser, Some(&ag_cfg));
    assert!(ag_files.iter().any(|p| p.ends_with("transcript.jsonl")));
    assert!(!ag_files.iter().any(|p| p.ends_with("transcript_full.jsonl")));

    let ag_file = ag_root
        .join("conv-001")
        .join(".system_generated")
        .join("logs")
        .join("transcript.jsonl");
    let ag_out =
        tail_file_incremental(&storage, &ag_parser, &ag_file, &dev_id).expect("ag tail");
    assert_eq!(ag_out.events_inserted, 2);

    let filter = AnalyticsFilter {
        start_ms: 0,
        end_ms: i64::MAX / 2,
        provider_id: None,
        device_id: None,
        tz_offset_minutes: 480,
        all_time: false,
    };
    let ov = storage.query_overview(&filter, 1_800_000_000_000).expect("overview");
    assert_eq!(ov.total_requests, 6);
    assert_eq!(ov.observable_steps, 1);

    // Re-running all 3 files is strictly idempotent
    let cc_again = tail_file_incremental(&storage, &cc_parser, &cc_file, &dev_id).expect("re-tail");
    assert_eq!(cc_again.events_inserted, 0);
}

#[test]
fn incremental_tailer_handles_partial_lines_concurrent_writers_truncation_and_rotation() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("tail-test.db");
    let storage = Storage::open_file(&db_path).expect("db");
    let dev_id = storage.local_device().expect("device").device_id;

    let log_path = dir.path().join("sess-live.jsonl");
    let parser = ClaudeCodeParser::new();

    // Hold a concurrent write handle open while tailing (verifies Windows shared read lock compatibility)
    let mut writer = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .expect("open writer");

    let line1 = r#"{"type":"assistant","timestamp":"2026-11-15T12:00:00Z","sessionId":"s-live","uuid":"u1","message":{"id":"m1","model":"claude-sonnet-4-5","usage":{"input_tokens":1000,"output_tokens":200}}}"#;
    writeln!(writer, "{line1}").expect("write line1");
    // Write an incomplete line without '\n'
    let partial_prefix = r#"{"type":"assistant","timestamp":"2026-11-15T12:00:05Z","sessionId":"s-live","uuid":"u2","message":{"id":"m2""#;
    write!(writer, "{partial_prefix}").expect("write partial");
    writer.flush().expect("flush");

    let out1 = tail_file_incremental(&storage, &parser, &log_path, &dev_id).expect("tail 1");
    assert_eq!(out1.events_inserted, 1);

    // Finish writing line2 while writer remains open
    let line2_rest = r#","model":"claude-sonnet-4-5","usage":{"input_tokens":500,"output_tokens":100}}}"#;
    writeln!(writer, "{line2_rest}").expect("finish line2");
    writer.flush().expect("flush2");

    let out2 = tail_file_incremental(&storage, &parser, &log_path, &dev_id).expect("tail 2");
    assert_eq!(out2.events_inserted, 1);
    drop(writer);

    // Simulate truncation + rewrite with old line1 + new line3 (must NOT double count line1!)
    {
        let mut trunc = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&log_path)
            .expect("truncate");
        writeln!(trunc, "{line1}").expect("rewrite 1");
        let line3 = r#"{"type":"assistant","timestamp":"2026-11-15T12:01:00Z","sessionId":"s-live","uuid":"u3","message":{"id":"m3","model":"claude-sonnet-4-5","usage":{"input_tokens":200,"output_tokens":50}}}"#;
        writeln!(trunc, "{line3}").expect("write 3");
        trunc.flush().expect("flush3");
    }

    let out3 = tail_file_incremental(&storage, &parser, &log_path, &dev_id).expect("tail 3");
    assert!(out3.reset_detected);
    assert_eq!(out3.events_inserted, 1);
    assert_eq!(out3.events_deduplicated, 1);

    // Verify exact total across all steps = 3 requests, 2050 tokens
    let filter = AnalyticsFilter {
        start_ms: 0,
        end_ms: i64::MAX / 2,
        provider_id: None,
        device_id: None,
        tz_offset_minutes: 480,
        all_time: false,
    };
    let ov = storage.query_overview(&filter, 1_800_000_000_000).expect("ov");
    assert_eq!(ov.total_requests, 3);
    assert_eq!(ov.total_tokens, 2_050);
}

#[tokio::test]
async fn loopback_otlp_server_receives_and_persists_metrics_and_logs() {
    // 1. Strictly enforce loopback-only binding at validation boundary
    assert!(validate_loopback_addr(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 4318).is_err());
    assert!(validate_loopback_addr(IpAddr::V4(Ipv4Addr::LOCALHOST), 4318).is_ok());

    // 2. Bind real 127.0.0.1 loopback TCP listener and verify clean graceful shutdown
    let storage = Storage::open_in_memory().expect("db");
    let handle = start_loopback_otlp_server(storage.clone(), 0)
        .await
        .expect("start loopback otlp");
    assert!(handle.port > 0);

    // 3. Ingest OTLP/JSON logs payload and verify idempotent deduplication
    let log_payload = serde_json::json!({
        "resourceLogs": [{
            "resource": { "attributes": [{ "key": "service.name", "value": { "stringValue": "claude-code" } }] },
            "scopeLogs": [{
                "logRecords": [{
                    "timeUnixNano": "1760000000000000000",
                    "attributes": [
                        { "key": "event.name", "value": { "stringValue": "claude_code.api_request" } },
                        { "key": "model", "value": { "stringValue": "claude-sonnet-4-5" } },
                        { "key": "session.id", "value": { "stringValue": "otlp-e2e" } },
                        { "key": "input_tokens", "value": { "intValue": "2500" } },
                        { "key": "output_tokens", "value": { "intValue": "500" } },
                        { "key": "duration_ms", "value": { "intValue": "2500" } }
                    ]
                }]
            }]
        }]
    });

    let log_out = ingest_otlp_logs_payload(&storage, &log_payload).expect("ingest otlp logs");
    assert_eq!(log_out.received, 1);
    assert_eq!(log_out.inserted, 1);
    assert_eq!(log_out.deduplicated, 0);

    let log_replay = ingest_otlp_logs_payload(&storage, &log_payload).expect("replay otlp logs");
    assert_eq!(log_replay.inserted, 0);
    assert_eq!(log_replay.deduplicated, 1);

    // 4. Ingest OTLP/JSON metrics payload (`claude_code.token.usage`)
    let metrics_payload = serde_json::json!({
        "resourceMetrics": [{
            "resource": { "attributes": [{ "key": "service.name", "value": { "stringValue": "claude-code" } }] },
            "scopeMetrics": [{
                "metrics": [{
                    "name": "claude_code.token.usage",
                    "sum": {
                        "dataPoints": [
                            {
                                "timeUnixNano": "1760000060000000000",
                                "asInt": "1200",
                                "attributes": [
                                    { "key": "model", "value": { "stringValue": "claude-sonnet-4-5" } },
                                    { "key": "session.id", "value": { "stringValue": "otlp-metric-s1" } },
                                    { "key": "type", "value": { "stringValue": "input" } }
                                ]
                            },
                            {
                                "timeUnixNano": "1760000060000000000",
                                "asInt": "300",
                                "attributes": [
                                    { "key": "model", "value": { "stringValue": "claude-sonnet-4-5" } },
                                    { "key": "session.id", "value": { "stringValue": "otlp-metric-s1" } },
                                    { "key": "type", "value": { "stringValue": "output" } }
                                ]
                            }
                        ]
                    }
                }]
            }]
        }]
    });

    let metrics_out =
        ingest_otlp_metrics_payload(&storage, &metrics_payload).expect("ingest otlp metrics");
    assert_eq!(metrics_out.received, 1);
    assert_eq!(metrics_out.inserted, 1);

    // 5. Verify total persisted tokens (2500+500 + 1200+300 = 4500) across 2 requests
    let filter = AnalyticsFilter {
        start_ms: 0,
        end_ms: i64::MAX / 2,
        provider_id: Some(provider_ids::CLAUDE_CODE.to_string()),
        device_id: None,
        tz_offset_minutes: 480,
        all_time: false,
    };
    let ov = storage
        .query_overview(&filter, 1_800_000_000_000)
        .expect("overview");
    assert_eq!(ov.total_requests, 2);
    assert_eq!(ov.total_tokens, 4_500);

    handle.stop().await;
}

/// Claude Code parser restricted to configured custom paths: `default_roots` is
/// emptied so tests never walk the developer's real `~/.claude/projects`.
struct FixtureOnlyClaudeParser(ClaudeCodeParser);

impl tokvane_collectors::providers::ProviderLogParser for FixtureOnlyClaudeParser {
    fn provider_id(&self) -> &str {
        self.0.provider_id()
    }
    fn default_roots(&self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn matches_file(&self, path: &std::path::Path) -> bool {
        self.0.matches_file(path)
    }
    fn init_context(
        &self,
        path: &std::path::Path,
        device_id: &str,
    ) -> tokvane_collectors::providers::FileParseContext {
        self.0.init_context(path, device_id)
    }
    fn parse_line(
        &self,
        line: &str,
        ctx: &mut tokvane_collectors::providers::FileParseContext,
    ) -> Option<tokvane_collectors::providers::ParsedLineResult> {
        self.0.parse_line(line, ctx)
    }
    fn finalize_batch(
        &self,
        events: Vec<tokvane_core::domain::NormalizedUsageEvent>,
    ) -> Vec<tokvane_core::domain::NormalizedUsageEvent> {
        self.0.finalize_batch(events)
    }
}

#[test]
fn background_watcher_spawns_from_non_tokio_main_thread_without_panic() {
    use std::sync::mpsc;
    use std::time::Duration;
    use tokvane_collectors::CollectorCoordinator;

    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("watcher-boot.db");
    let storage = Storage::open_file(&db_path).expect("db");

    // Disable default log roots and point Claude Code at a controlled fixture copy
    let cc_dir = dir.path().join("cc_logs");
    std::fs::create_dir_all(&cc_dir).expect("mkdir");
    let cc_log = cc_dir.join("sess-boot.jsonl");
    std::fs::write(
        &cc_log,
        concat!(
            r#"{"type":"assistant","timestamp":"2026-11-15T12:00:00Z","sessionId":"s-boot","uuid":"u-boot-1","message":{"id":"m-boot-1","model":"claude-sonnet-4-5","usage":{"input_tokens":120,"output_tokens":30}}}"#,
            "\n"
        ),
    )
    .expect("write log");

    for pid in [
        provider_ids::CLAUDE_CODE,
        provider_ids::CODEX,
        provider_ids::ANTIGRAVITY,
    ] {
        let mut cfg = tokvane_core::domain::ProviderConfig {
            provider_id: pid.to_string(),
            enabled: pid == provider_ids::CLAUDE_CODE,
            collection_mode: tokvane_core::domain::CollectionMode::LocalLog,
            custom_log_paths: if pid == provider_ids::CLAUDE_CODE {
                vec![cc_log.to_string_lossy().to_string()]
            } else {
                Vec::new()
            },
            has_credential: false,
            last_sync_at_ms: None,
            last_error: None,
            updated_at_ms: 0,
        };
        if pid == provider_ids::CLAUDE_CODE {
            cfg.enabled = true;
        }
        storage.upsert_provider_config(&cfg).expect("upsert cfg");
    }

    // Hermetic parser set: only the fixture log is scanned, never real home-directory logs.
    let coord = CollectorCoordinator::with_parsers(
        storage.clone(),
        vec![Arc::new(FixtureOnlyClaudeParser(ClaudeCodeParser::new()))],
    );
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let rt_handle = rt.handle().clone();

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let (scan_done_tx, scan_done_rx) = mpsc::channel::<usize>();

    // Simulate Tauri `.setup(...)`: plain OS thread with NO entered Tokio runtime context.
    let spawn_thread = std::thread::spawn(move || {
        assert!(tokio::runtime::Handle::try_current().is_err());
        rt_handle.spawn(coord.run_background_watcher_with_hook(shutdown_rx, move |report| {
            let _ = scan_done_tx.send(report.events_inserted);
        }))
    });

    let watcher_task = spawn_thread
        .join()
        .expect("spawning watcher from non-Tokio thread must not panic");

    let inserted = scan_done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("initial startup scan should complete");
    assert!(inserted >= 1);

    shutdown_tx.send(true).expect("signal shutdown");
    rt.block_on(watcher_task)
        .expect("watcher task should exit cleanly");
}

#[tokio::test]
async fn catalog_sync_handles_primary_fallback_offline_and_locks() {
    use std::sync::Mutex;
    use tokvane_collectors::catalog_sync::BoxFuture;
    use tokvane_collectors::{
        sync_catalog_and_rate_with_fetcher, CatalogSyncEndpoints, ReqwestTextFetcher,
        TextHttpFetcher,
    };
    use tokvane_core::domain::{compute_event_id, ModelPricingEntry, NormalizedUsageEvent};

    // Ensure production reqwest client builds cleanly with rustls + timeout settings.
    assert!(ReqwestTextFetcher::new().is_ok());

    let sample_models_dev = serde_json::json!({
        "anthropic": {
            "id": "anthropic",
            "models": {
                "claude-sonnet-4-5": {
                    "id": "claude-sonnet-4-5",
                    "cost": { "input": 3.0, "output": 15.0, "cache_read": 0.3, "cache_write": 3.75 }
                },
                "claude-opus-4-6": {
                    "id": "claude-opus-4-6",
                    "cost": { "input": 15.0, "output": 75.0, "cache_read": 1.5, "cache_write": 18.75 }
                }
            }
        },
        "openai": {
            "id": "openai",
            "models": {
                "gpt-5-codex": {
                    "id": "gpt-5-codex",
                    "cost": { "input": 2.5, "output": 12.0, "cache_read": 0.25 }
                },
                "gpt-5.2-codex": {
                    "id": "gpt-5.2-codex",
                    "cost": { "input": 1.75, "output": 14.0, "cache_read": 0.175 }
                }
            }
        },
        "google": {
            "id": "google",
            "models": {
                "gemini-3-flash": {
                    "id": "gemini-3-flash",
                    "cost": { "input": 0.5, "output": 3.0, "cache_read": 0.05 }
                }
            }
        }
    })
    .to_string();

    let sample_litellm = serde_json::json!({
        "claude-sonnet-4-5": {
            "litellm_provider": "anthropic",
            "mode": "chat",
            "input_cost_per_token": 0.000003,
            "output_cost_per_token": 0.000015,
            "cache_read_input_token_cost": 0.0000003
        },
        "claude-haiku-4-5": {
            "litellm_provider": "anthropic",
            "mode": "chat",
            "input_cost_per_token": 0.000001,
            "output_cost_per_token": 0.000005
        },
        "gpt-5-codex": {
            "litellm_provider": "openai",
            "mode": "chat",
            "input_cost_per_token": 0.0000015,
            "output_cost_per_token": 0.000012
        },
        "gemini-2.5-pro": {
            "litellm_provider": "gemini",
            "mode": "chat",
            "input_cost_per_token": 0.00000125,
            "output_cost_per_token": 0.00001
        },
        "deepseek/deepseek-chat": {
            "litellm_provider": "deepseek",
            "mode": "chat",
            "input_cost_per_token": 0.00000028,
            "output_cost_per_token": 0.00000042
        }
    })
    .to_string();

    let mut sample_sub_plans: Vec<serde_json::Value> =
        tokvane_core::subscription::subscription_plan_templates()
            .into_iter()
            .take(12)
            .map(|mut t| {
                if t.template_id == "google_ai_ultra" {
                    t.plan_name = "Google AI Ultra 20x".to_string();
                }
                let mut v = serde_json::to_value(&t).expect("json");
                v.as_object_mut()
                    .expect("obj")
                    .insert("status".to_string(), serde_json::json!("active"));
                v
            })
            .collect();
    sample_sub_plans.push(serde_json::json!({
        "template_id": "cursor_plus",
        "provider_id": "cursor",
        "plan_name": "Cursor Plus",
        "monthly_fee_micro_usd": 20000000,
        "windows": [],
        "description": "Cursor Plus plan",
        "status": "active"
    }));
    let sample_sub_catalog = serde_json::json!({
        "schema_version": "1.0.0",
        "plans": sample_sub_plans
    })
    .to_string();

    struct MockHttpFetcher {
        models_dev_body: String,
        litellm_body: String,
        sub_catalog_body: String,
        requests: Mutex<Vec<String>>,
    }

    impl TextHttpFetcher for MockHttpFetcher {
        fn fetch_text<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, String>> {
            if let Ok(mut guard) = self.requests.lock() {
                guard.push(url.to_string());
            }
            let res = match url {
                "https://mock.local/frankfurter" => {
                    Ok(r#"{"amount":1.0,"base":"USD","rates":{"CNY":7.1852}}"#.to_string())
                }
                "https://mock.local/open-er" => {
                    Ok(r#"{"result":"success","base_code":"USD","rates":{"CNY":7.2100}}"#.to_string())
                }
                "https://mock.local/models-dev" => Ok(self.models_dev_body.clone()),
                "https://mock.local/litellm" => Ok(self.litellm_body.clone()),
                "https://mock.local/sub-pages.github.io"
                | "https://cdn.jsdelivr.net/mock-sub" => Ok(self.sub_catalog_body.clone()),
                _ => Err("HTTP 500".to_string()),
            };
            Box::pin(async move { res })
        }
    }

    let fetcher = MockHttpFetcher {
        models_dev_body: sample_models_dev,
        litellm_body: sample_litellm,
        sub_catalog_body: sample_sub_catalog,
        requests: Mutex::new(Vec::new()),
    };

    // Scenario 1: Primary endpoints succeed, recomputing existing codex event cost
    let storage = Storage::open_in_memory().expect("db");
    let dev = storage.local_device().expect("dev").device_id;
    let ts = 1_760_000_000_000i64;
    storage
        .ingest_events_with_cursor(
            &[NormalizedUsageEvent {
                event_id: compute_event_id("codex", "s-sync", "m-sync", ts),
                device_id: dev,
                provider_id: provider_ids::CODEX.to_string(),
                session_id: "s-sync".to_string(),
                project_label: "proj".to_string(),
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
            }],
            None,
            &[],
        )
        .expect("ingest");

    let primary_endpoints = CatalogSyncEndpoints {
        frankfurter_url: "https://mock.local/frankfurter".to_string(),
        open_er_url: "https://mock.local/open-er".to_string(),
        models_dev_url: "https://mock.local/models-dev".to_string(),
        litellm_url: "https://mock.local/litellm".to_string(),
        subscription_catalog_urls: vec![
            "https://mock.local/sub-pages.github.io".to_string(),
            "https://cdn.jsdelivr.net/mock-sub".to_string(),
        ],
    };
    let res1 = sync_catalog_and_rate_with_fetcher(&storage, true, &primary_endpoints, &fetcher)
        .await
        .expect("primary sync");
    assert!(res1.rate_updated, "expected rate_updated in res1: {res1:?}");
    assert_eq!(res1.usd_to_cny_rate, "7.1852");
    assert!(res1.pricing_updated);
    assert_eq!(res1.recomputed_events_count, 1);
    assert_eq!(res1.rate_source.as_deref(), Some("frankfurter.dev"));
    assert_eq!(res1.pricing_source.as_deref(), Some("models.dev"));
    assert!(res1.subscriptions_updated);
    assert_eq!(res1.synced_plans_count, 13);
    assert_eq!(res1.subscription_source.as_deref(), Some("github-pages"));
    assert!(res1.warning.is_none());

    let synced_templates = storage.query_subscription_templates(ts).expect("templates");
    assert!(synced_templates.iter().any(|t| t.template_id == "cursor_plus"));
    let ultra_tpl = synced_templates
        .iter()
        .find(|t| t.template_id == "google_ai_ultra")
        .expect("ultra");
    assert_eq!(ultra_tpl.plan_name, "Google AI Ultra");

    // Subsequent non-forced call within 12h/24h TTL makes ZERO HTTP requests
    let calls_before = fetcher.requests.lock().expect("lock").len();
    let res_ttl = sync_catalog_and_rate_with_fetcher(&storage, false, &primary_endpoints, &fetcher)
        .await
        .expect("ttl skip");
    assert!(!res_ttl.rate_updated);
    assert!(!res_ttl.pricing_updated);
    assert!(!res_ttl.subscriptions_updated);
    assert_eq!(fetcher.requests.lock().expect("lock").len(), calls_before);

    // Scenario 2: Primary HTTP 500 -> automatic fallback to open.er-api + litellm + jsdelivr
    let fallback_endpoints = CatalogSyncEndpoints {
        frankfurter_url: "https://mock.local/fail".to_string(),
        open_er_url: "https://mock.local/open-er".to_string(),
        models_dev_url: "https://mock.local/fail".to_string(),
        litellm_url: "https://mock.local/litellm".to_string(),
        subscription_catalog_urls: vec![
            "https://mock.local/fail".to_string(),
            "https://cdn.jsdelivr.net/mock-sub".to_string(),
        ],
    };
    let res2 = sync_catalog_and_rate_with_fetcher(&storage, true, &fallback_endpoints, &fetcher)
        .await
        .expect("fallback sync");
    assert!(res2.rate_updated);
    assert_eq!(res2.usd_to_cny_rate, "7.21");
    assert!(res2.pricing_updated);
    assert_eq!(res2.rate_source.as_deref(), Some("open.er-api.com"));
    assert_eq!(res2.pricing_source.as_deref(), Some("litellm"));
    assert_eq!(res2.subscription_source.as_deref(), Some("jsdelivr"));
    assert!(res2.warning.is_none());

    // Scenario 3: Both endpoints fail -> graceful offline fallback with warning, cached data intact
    let offline_endpoints = CatalogSyncEndpoints {
        frankfurter_url: "https://mock.local/fail".to_string(),
        open_er_url: "https://mock.local/fail".to_string(),
        models_dev_url: "https://mock.local/fail".to_string(),
        litellm_url: "https://mock.local/fail".to_string(),
        subscription_catalog_urls: vec!["https://mock.local/fail".to_string()],
    };
    let res3 = sync_catalog_and_rate_with_fetcher(&storage, true, &offline_endpoints, &fetcher)
        .await
        .expect("offline fallback");
    assert!(!res3.rate_updated);
    assert!(!res3.pricing_updated);
    assert!(!res3.subscriptions_updated);
    assert_eq!(res3.usd_to_cny_rate, "7.21");
    assert!(res3.warning.is_some());

    // Scenario 4: Manual exchange rate lock + pricing user override remain protected on force sync
    let mut prefs = storage.get_preferences().expect("prefs");
    prefs.exchange_rate_auto = false;
    prefs.usd_to_cny_rate = "7.40".to_string();
    storage.update_preferences(&prefs).expect("save locked");

    storage
        .upsert_model_pricing(&ModelPricingEntry {
            model_pattern: "gpt-5-codex".to_string(),
            provider_id: provider_ids::CODEX.to_string(),
            input_per_m_micro_usd: 9_000_000,
            output_per_m_micro_usd: 18_000_000,
            cache_read_per_m_micro_usd: 900_000,
            cache_write_per_m_micro_usd: 0,
            reasoning_per_m_micro_usd: 18_000_000,
            is_user_override: true,
            effective_from: "2026-11-16".to_string(),
        })
        .expect("user pricing override");

    let res4 = sync_catalog_and_rate_with_fetcher(&storage, true, &primary_endpoints, &fetcher)
        .await
        .expect("sync with locks");
    assert!(!res4.rate_updated);
    assert_eq!(res4.usd_to_cny_rate, "7.40");
    let catalog = storage.list_model_pricing().expect("catalog");
    let codex_entry = catalog
        .iter()
        .find(|p| p.model_pattern == "gpt-5-codex")
        .expect("codex");
    assert!(codex_entry.is_user_override);
    assert_eq!(codex_entry.input_per_m_micro_usd, 9_000_000);
}

#[test]
fn replays_phase2_five_providers_and_preserves_prefix_cache_and_routed_models() {
    let storage = Storage::open_in_memory().expect("in-memory db");
    let dev_id = storage.local_device().expect("device").device_id;
    let fx = fixtures_dir();

    // 1. Kimi Code
    let kimi = KimiCodeParser::new();
    let kimi_out = tail_file_incremental(
        &storage,
        &kimi,
        &fx.join("kimi_code").join("sess-kimi-001.jsonl"),
        &dev_id,
    )
    .expect("kimi tail");
    assert_eq!(kimi_out.events_inserted, 2);

    // 2. Workbuddy (multi-backend routed models)
    let wb = WorkbuddyParser::new();
    let wb_out = tail_file_incremental(
        &storage,
        &wb,
        &fx.join("workbuddy").join("sess-wb-001.jsonl"),
        &dev_id,
    )
    .expect("workbuddy tail");
    assert_eq!(wb_out.events_inserted, 2);

    // 3. DeepSeek Harness (Prefix Cache hit vs miss + reasoning)
    let dsh = DeepSeekHarnessParser::new();
    let dsh_out = tail_file_incremental(
        &storage,
        &dsh,
        &fx.join("deepseek_harness").join("dsh-001.jsonl"),
        &dev_id,
    )
    .expect("dsh tail");
    assert_eq!(dsh_out.events_inserted, 2);

    // 4. QCode (gateway multi-model)
    let qcode = QCodeParser::new();
    let qc_out = tail_file_incremental(
        &storage,
        &qcode,
        &fx.join("qcode").join("sess-qc-001.jsonl"),
        &dev_id,
    )
    .expect("qcode tail");
    assert_eq!(qc_out.events_inserted, 2);

    // 5. MiniMax Code (Anthropic-compatible usage)
    let mm = MiniMaxCodeParser::new();
    let mm_out = tail_file_incremental(
        &storage,
        &mm,
        &fx.join("minimax_code").join("sess-mm-001.jsonl"),
        &dev_id,
    )
    .expect("minimax tail");
    assert_eq!(mm_out.events_inserted, 1);

    let filter = AnalyticsFilter {
        start_ms: 0,
        end_ms: i64::MAX / 2,
        provider_id: None,
        device_id: None,
        tz_offset_minutes: 480,
        all_time: true,
    };
    let ov = storage
        .query_overview(&filter, 1_800_000_000_000)
        .expect("overview");
    assert_eq!(ov.total_requests, 9);

    // Verify Workbuddy routed model breakdown contains claude-sonnet-4-5 & deepseek-chat
    let wb_filter = AnalyticsFilter {
        provider_id: Some(provider_ids::WORKBUDDY.to_string()),
        ..filter.clone()
    };
    let wb_models = storage.query_models_pricing(&wb_filter).expect("wb models");
    assert!(wb_models.rows.iter().any(|m| m.model_id == "claude-sonnet-4-5"));
    assert!(wb_models.rows.iter().any(|m| m.model_id == "deepseek-chat"));

    // Verify DeepSeek Harness Prefix Cache separation: hit=8000+2500=10500, miss=2000+500=2500, reasoning=300
    let dsh_filter = AnalyticsFilter {
        provider_id: Some(provider_ids::DEEPSEEK_HARNESS.to_string()),
        ..filter.clone()
    };
    let dsh_models = storage.query_models_pricing(&dsh_filter).expect("dsh models");
    let ds_reasoner = dsh_models
        .rows
        .iter()
        .find(|m| m.model_id == "deepseek-reasoner")
        .expect("deepseek-reasoner row");
    assert_eq!(ds_reasoner.input_tokens, 2000);
    assert_eq!(ds_reasoner.cache_read_tokens, 8000);
    assert_eq!(ds_reasoner.reasoning_tokens, 300);
    assert_eq!(ds_reasoner.output_tokens, 500);

    // Re-running is idempotent
    let dsh_re = tail_file_incremental(
        &storage,
        &dsh,
        &fx.join("deepseek_harness").join("dsh-001.jsonl"),
        &dev_id,
    )
    .expect("dsh re-tail");
    assert_eq!(dsh_re.events_inserted, 0);
}

#[test]
fn custom_declarative_rules_and_phase2_otlp_ingest_cleanly() {
    let dir = tempdir().expect("tempdir");
    let log_file = dir.path().join("custom.ndjson");
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .expect("open log");
    // First write incomplete trailing line (no newline)
    write!(
        f,
        r#"{{"ts":1763210000,"meta":{{"model":"custom-coder-v1"}},"tok":{{"in":120,"out":80,"cached":300}}}}"#
    )
    .expect("write partial");
    f.flush().expect("flush");

    let storage = Storage::open_in_memory().expect("in-memory db");
    for mut cfg in storage.list_provider_configs().expect("list configs") {
        cfg.enabled = false;
        storage.upsert_provider_config(&cfg).expect("disable default");
    }
    let rule = CustomProviderRule {
        rule_id: "local_gateway".to_string(),
        display_name: "My Local Gateway".to_string(),
        enabled: true,
        protocol_kind: CustomProtocolKind::DeclarativeJsonl,
        log_roots: vec![dir.path().to_string_lossy().to_string()],
        mapping_spec: DeclarativeMappingSpec {
            provider_id_override: None,
            file_extensions: vec!["ndjson".to_string()],
            timestamp_path: Some("/ts".to_string()),
            model_path: Some("/meta/model".to_string()),
            session_id_path: None,
            request_id_path: None,
            input_tokens_path: Some("/tok/in".to_string()),
            output_tokens_path: Some("/tok/out".to_string()),
            cache_read_tokens_path: Some("/tok/cached".to_string()),
            cache_write_tokens_path: None,
            reasoning_tokens_path: None,
            duration_ms_path: None,
            ttft_ms_path: None,
        },
        updated_at_ms: 1_763_210_000_000,
    };
    storage
        .upsert_custom_provider_rule(&rule)
        .expect("save rule");

    // Hermetic: no built-in parsers, so only the custom rule's temp directory is
    // scanned and real AI tool logs on the developer machine never leak in.
    let coordinator = CollectorCoordinator::with_parsers(storage.clone(), Vec::new());
    let rep1 = coordinator.scan_now().expect("scan partial");
    assert_eq!(rep1.events_inserted, 0);

    // Complete first line with newline + append second line
    writeln!(f).expect("newline");
    writeln!(
        f,
        r#"{{"ts":1763210005,"meta":{{"model":"custom-coder-v1"}},"tok":{{"in":200,"out":50,"cached":100}}}}"#
    )
    .expect("line 2");
    f.flush().expect("flush 2");

    let rep2 = coordinator.scan_now().expect("scan full");
    assert_eq!(rep2.events_inserted, 2);

    // Also verify Phase 2 OTLP system attribution
    let otlp_dsh = serde_json::json!({
        "resourceLogs": [{
            "resource": { "attributes": [{ "key": "service.name", "value": { "stringValue": "deepseek-harness" } }] },
            "scopeLogs": [{
                "logRecords": [{
                    "timeUnixNano": "1763210010000000000",
                    "attributes": [
                        { "key": "event.name", "value": { "stringValue": "dsh.api_request" } },
                        { "key": "model", "value": { "stringValue": "deepseek-chat" } },
                        { "key": "input_tokens", "value": { "intValue": "500" } },
                        { "key": "output_tokens", "value": { "intValue": "150" } }
                    ]
                }]
            }]
        }]
    });
    let otlp_out = ingest_otlp_logs_payload(&storage, &otlp_dsh).expect("otlp dsh");
    assert_eq!(otlp_out.inserted, 1);

    let ov = storage
        .query_overview(
            &AnalyticsFilter {
                start_ms: 0,
                end_ms: i64::MAX / 2,
                provider_id: Some(provider_ids::DEEPSEEK_HARNESS.to_string()),
                device_id: None,
                tz_offset_minutes: 480,
                all_time: true,
            },
            1_800_000_000_000,
        )
        .expect("dsh otlp overview");
    assert_eq!(ov.total_tokens, 650);
}

struct MockAuthFetcher {
    responses: Mutex<HashMap<String, (u16, String)>>,
    calls: Mutex<Vec<(String, String)>>,
}

impl AuthHttpFetcher for MockAuthFetcher {
    fn fetch_bearer_get<'a>(
        &'a self,
        url: &'a str,
        bearer_token: &'a str,
    ) -> tokvane_collectors::providers::official_api::BoxFuture<'a, Result<(u16, String), String>>
    {
        let u = url.to_string();
        let tok = bearer_token.to_string();
        Box::pin(async move {
            self.calls.lock().expect("lock").push((u.clone(), tok));
            let map = self.responses.lock().expect("lock");
            map.get(&u)
                .cloned()
                .ok_or_else(|| format!("unexpected mock url {u}"))
        })
    }
}

#[tokio::test]
async fn official_balance_api_parses_all_vendors_and_enforces_60s_cooldown() {
    let storage = Storage::open_in_memory().expect("in-memory db");
    let creds = MemoryCredentialStore::new();
    creds
        .set_api_key(provider_ids::KIMI_CODE, "sk-moonshot-test")
        .expect("set kimi");
    creds
        .set_api_key(provider_ids::DEEPSEEK_HARNESS, "sk-ds-test")
        .expect("set ds");
    creds
        .set_api_key(provider_ids::QCODE, "qot_qcode-test")
        .expect("set qcode");
    creds
        .set_api_key(provider_ids::MINIMAX_CODE, "sk-mm-test")
        .expect("set mm");

    let endpoints = BalanceApiEndpoints {
        moonshot_url: "https://api.moonshot.cn/v1/users/me/balance".to_string(),
        moonshot_global_url: "https://api.moonshot.ai/v1/users/me/balance".to_string(),
        kimi_coding_url: "https://api.kimi.com/coding/v1/usages".to_string(),
        deepseek_url: "https://api.deepseek.com/user/balance".to_string(),
        qcode_url: "https://api.qcode.cc/api/v1/usage".to_string(),
        minimax_url: "https://www.minimaxi.com/v1/token_plan/remains".to_string(),
    };

    let mut responses = HashMap::new();
    responses.insert(
        endpoints.moonshot_url.clone(),
        (
            200,
            serde_json::json!({
                "code": 0,
                "status": true,
                "data": {
                    "available_balance": 49.5,
                    "voucher_balance": 40.0,
                    "cash_balance": 9.5
                }
            })
            .to_string(),
        ),
    );
    responses.insert(
        endpoints.deepseek_url.clone(),
        (
            200,
            serde_json::json!({
                "is_available": true,
                "balance_infos": [{
                    "currency": "CNY",
                    "total_balance": "120.25",
                    "granted_balance": "20.25",
                    "topped_up_balance": "100.00"
                }]
            })
            .to_string(),
        ),
    );
    responses.insert(
        endpoints.qcode_url.clone(),
        (
            200,
            serde_json::json!({
                "data": {
                    "balance": 18.5,
                    "daily_used": 2.4,
                    "daily_limit": 10.0,
                    "currency": "USD"
                }
            })
            .to_string(),
        ),
    );
    responses.insert(
        endpoints.minimax_url.clone(),
        (
            200,
            serde_json::json!({
                "base_resp": { "status_code": 0, "status_msg": "success" },
                "model_remains": [{
                    "model_name": "MiniMax-M2.5",
                    "current_interval_total_count": 1000,
                    "current_interval_usage_count": 250,
                    "remains_time": 7200
                }]
            })
            .to_string(),
        ),
    );

    let fetcher = Arc::new(MockAuthFetcher {
        responses: Mutex::new(responses),
        calls: Mutex::new(Vec::new()),
    });

    let now_ms = 1_763_300_000_000i64;
    let kimi_res = refresh_provider_balance_with_fetcher(
        &storage,
        provider_ids::KIMI_CODE,
        &creds,
        fetcher.as_ref(),
        &endpoints,
        now_ms,
    )
    .await
    .expect("kimi balance");
    assert!(!kimi_res.from_cache);
    assert_eq!(kimi_res.snapshot.currency, "CNY");
    assert_eq!(kimi_res.snapshot.total_balance_micro, Some(49_500_000));
    assert_eq!(kimi_res.snapshot.granted_balance_micro, Some(40_000_000));
    assert_eq!(kimi_res.snapshot.topped_up_balance_micro, Some(9_500_000));

    // Cooldown: calling again 10s later returns cached balance with 0 additional HTTP requests
    let calls_before = fetcher.calls.lock().expect("lock").len();
    let kimi_cached = refresh_provider_balance_with_fetcher(
        &storage,
        provider_ids::KIMI_CODE,
        &creds,
        fetcher.as_ref(),
        &endpoints,
        now_ms + 10_000,
    )
    .await
    .expect("kimi cached");
    assert!(kimi_cached.from_cache);
    assert_eq!(kimi_cached.cooldown_remaining_secs, 50);
    assert_eq!(fetcher.calls.lock().expect("lock").len(), calls_before);

    let ds_res = refresh_provider_balance_with_fetcher(
        &storage,
        provider_ids::DEEPSEEK_HARNESS,
        &creds,
        fetcher.as_ref(),
        &endpoints,
        now_ms,
    )
    .await
    .expect("deepseek balance");
    assert_eq!(ds_res.snapshot.total_balance_micro, Some(120_250_000));

    let qc_res = refresh_provider_balance_with_fetcher(
        &storage,
        provider_ids::QCODE,
        &creds,
        fetcher.as_ref(),
        &endpoints,
        now_ms,
    )
    .await
    .expect("qcode balance");
    assert_eq!(qc_res.snapshot.daily_quota_used_micro, Some(2_400_000));
    assert_eq!(qc_res.snapshot.daily_quota_limit_micro, Some(10_000_000));

    let mm_res = refresh_provider_balance_with_fetcher(
        &storage,
        provider_ids::MINIMAX_CODE,
        &creds,
        fetcher.as_ref(),
        &endpoints,
        now_ms,
    )
    .await
    .expect("minimax remains");
    assert_eq!(mm_res.rate_limits_updated, 1);
    let obs = storage.list_observed_rate_limits().expect("rate limits");
    assert!(obs
        .iter()
        .any(|o| o.provider_id == provider_ids::MINIMAX_CODE && (o.used_percent - 25.0).abs() < 0.01));

    // Unsupported provider strictly rejected for compliance
    let err = refresh_provider_balance_with_fetcher(
        &storage,
        provider_ids::WORKBUDDY,
        &creds,
        fetcher.as_ref(),
        &endpoints,
        now_ms,
    )
    .await
    .expect_err("workbuddy must reject balance api");
    assert!(err.to_string().contains("Compliance") || err.to_string().contains("public REST"));

    assert_eq!(storage.list_provider_balances().expect("list balances").len(), 4);
}

#[derive(Default)]
struct MockWorkerTransportState {
    expected_token: String,
    next_seq: AtomicUsize,
    records: Mutex<Vec<(i64, String, serde_json::Value)>>,
    raw_push_bodies: Mutex<Vec<String>>,
    sse_tx: Mutex<Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>>,
}

struct MockWorkerTransport {
    state: Arc<MockWorkerTransportState>,
}

struct ChannelSseStream {
    rx: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
}

impl tokvane_collectors::SseByteStream for ChannelSseStream {
    fn next_chunk(
        &mut self,
    ) -> tokvane_collectors::SyncBoxFuture<'_, tokvane_collectors::CollectorResult<Option<Vec<u8>>>>
    {
        Box::pin(async move { Ok(self.rx.recv().await) })
    }
}

impl tokvane_collectors::SyncHttpTransport for MockWorkerTransport {
    fn post_json<'a>(
        &'a self,
        url: &'a str,
        bearer_token: &'a str,
        json_body: &'a str,
    ) -> tokvane_collectors::SyncBoxFuture<'a, tokvane_collectors::CollectorResult<(u16, String)>>
    {
        let body_text = json_body.to_string();
        Box::pin(async move {
            if bearer_token.trim() != self.state.expected_token {
                return Ok((
                    401,
                    serde_json::json!({ "ok": false, "message": "Unauthorized" }).to_string(),
                ));
            }
            assert!(url.ends_with("/v1/sync/push"));

            for forbidden in [
                "\"session_id\"",
                "\"file_path\"",
                "\"source_path\"",
                "\"project_label\"",
                "\"prompt\"",
                "\"messages\"",
            ] {
                assert!(
                    !body_text.contains(forbidden),
                    "Compliance violation: push payload contained {forbidden}: {body_text}"
                );
            }

            self.state
                .raw_push_bodies
                .lock()
                .expect("raw bodies lock")
                .push(body_text.clone());

            let parsed: serde_json::Value = serde_json::from_str(&body_text)?;
            let seq = (self.state.next_seq.fetch_add(1, Ordering::SeqCst) as i64 + 1) * 1000;
            let mode = parsed
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("plaintext")
                .to_string();

            let device_id = if mode == "e2ee" {
                parsed["encrypted_envelope"]["device_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            } else {
                parsed["envelope"]["device_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            };

            let item = if mode == "e2ee" {
                serde_json::json!({
                    "mode": "e2ee",
                    "seq_ms": seq,
                    "device_id": device_id,
                    "encrypted_envelope": parsed["encrypted_envelope"]
                })
            } else {
                serde_json::json!({
                    "mode": "plaintext",
                    "seq_ms": seq,
                    "device_id": device_id,
                    "envelope": parsed["envelope"]
                })
            };

            self.state
                .records
                .lock()
                .expect("records lock")
                .push((seq, device_id.clone(), item));

            if let Some(tx) = self.state.sse_tx.lock().expect("sse lock").as_ref() {
                let frame = format!(
                    "event: sync_update\ndata: {{\"seq_ms\":{seq},\"device_id\":\"{device_id}\",\"mode\":\"{mode}\"}}\n\n"
                );
                let _ = tx.send(frame.into_bytes());
            }

            Ok((
                200,
                serde_json::json!({
                    "ok": true,
                    "server_seq_ms": seq,
                    "mode": mode
                })
                .to_string(),
            ))
        })
    }

    fn get_with_bearer<'a>(
        &'a self,
        url: &'a str,
        bearer_token: &'a str,
    ) -> tokvane_collectors::SyncBoxFuture<'a, tokvane_collectors::CollectorResult<(u16, String)>>
    {
        Box::pin(async move {
            if bearer_token.trim() != self.state.expected_token {
                return Ok((
                    401,
                    serde_json::json!({ "ok": false, "message": "Unauthorized" }).to_string(),
                ));
            }
            let parsed_url = reqwest::Url::parse(url).expect("valid pull url");
            let params: HashMap<String, String> = parsed_url
                .query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            let since: i64 = params
                .get("since")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let exclude = params.get("exclude_device").map(String::as_str).unwrap_or("");

            let guard = self.state.records.lock().expect("records lock");
            let max_seq = guard.iter().map(|(seq, _, _)| *seq).max().unwrap_or(since);
            let items: Vec<serde_json::Value> = guard
                .iter()
                .filter(|(seq, dev_id, _)| *seq > since && dev_id != exclude)
                .map(|(_, _, item)| item.clone())
                .collect();

            Ok((
                200,
                serde_json::json!({
                    "ok": true,
                    "server_seq_ms": max_seq,
                    "next_since_ms": max_seq.max(since),
                    "has_more": false,
                    "items": items
                })
                .to_string(),
            ))
        })
    }

    fn open_sse_stream<'a>(
        &'a self,
        url: &'a str,
        bearer_token: &'a str,
    ) -> tokvane_collectors::SyncBoxFuture<
        'a,
        tokvane_collectors::CollectorResult<(u16, Box<dyn tokvane_collectors::SseByteStream>)>,
    > {
        Box::pin(async move {
            assert!(url.ends_with("/v1/sync/events"));
            if bearer_token.trim() != self.state.expected_token {
                let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
                return Ok((
                    401,
                    Box::new(ChannelSseStream { rx }) as Box<dyn tokvane_collectors::SseByteStream>,
                ));
            }
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let _ = tx.send(b": connected\n\n".to_vec());
            *self.state.sse_tx.lock().expect("sse lock") = Some(tx);
            Ok((
                200,
                Box::new(ChannelSseStream { rx }) as Box<dyn tokvane_collectors::SseByteStream>,
            ))
        })
    }
}

fn seed_device_usage_event(storage: &Storage, seed_id: &str, input_tokens: i64, ts_ms: i64) {
    let dev = storage.local_device().expect("local device");
    let ev = tokvane_core::domain::NormalizedUsageEvent {
        event_id: seed_id.to_string(),
        device_id: dev.device_id,
        provider_id: provider_ids::CLAUDE_CODE.to_string(),
        session_id: "secret-session-should-never-sync".to_string(),
        project_label: "private-secret-project".to_string(),
        model_id: "claude-sonnet-4-5".to_string(),
        timestamp_ms: ts_ms,
        input_tokens,
        output_tokens: 400,
        cache_read_tokens: 150,
        cache_write_tokens: 0,
        reasoning_tokens: 0,
        observable_only: false,
        duration_ms: Some(1_500),
        ttft_ms: Some(250),
        output_tps: Some(80.0),
        estimated_cost_micro_usd: 0,
        cache_saved_micro_usd: 0,
    };
    storage
        .ingest_events_with_cursor(&[ev], None, &[])
        .expect("insert usage");
}

#[tokio::test]
async fn cloud_sync_client_supports_plaintext_e2ee_and_realtime_sse() {
    use tokvane_collectors::{
        run_sync_cycle_with_transport, set_sync_credentials, SyncClientHandle, SyncHttpTransport,
    };
    use tokvane_core::analytics::AnalyticsFilter;
    use tokvane_core::domain::{QuotaUnit, SubscriptionPlan, SubscriptionWindowRule};

    let worker_url = "https://sync.example.workers.dev";
    let hub_state = Arc::new(MockWorkerTransportState {
        expected_token: "sync-test-secret".to_string(),
        ..Default::default()
    });
    let transport: Arc<dyn SyncHttpTransport> = Arc::new(MockWorkerTransport {
        state: Arc::clone(&hub_state),
    });
    let now_ms = chrono::Utc::now().timestamp_millis();

    // 1. Plaintext sync from Device A -> Device B (verifying desensitization + remote sub snapshots)
    let storage_a = Storage::open_in_memory().expect("db a");
    storage_a
        .update_preferences(&{
            let mut p = storage_a.get_preferences().expect("prefs a");
            p.device_display_name = "Workstation A".to_string();
            p
        })
        .expect("save prefs a");
    seed_device_usage_event(&storage_a, "ev-a-1", 2_500, now_ms - 60_000);
    storage_a
        .upsert_subscription_plan(&SubscriptionPlan {
            plan_id: "plan-a-claude".to_string(),
            provider_id: provider_ids::CLAUDE_CODE.to_string(),
            plan_name: "Claude Pro A".to_string(),
            enabled: true,
            monthly_fee_micro_usd: 20_000_000,
            is_user_configured: true,
            auto_source: None,
            updated_at_ms: now_ms,
            windows: vec![SubscriptionWindowRule {
                window_id: "5h".to_string(),
                label: "5h Rolling".to_string(),
                duration_hours: 5,
                rolling: true,
                unit: QuotaUnit::Tokens,
                limit_value: 100_000,
                warning_percent: 80,
                critical_percent: 95,
            }],
        })
        .expect("upsert plan a");
    storage_a
        .upsert_observed_rate_limits(&[tokvane_core::domain::ObservedRateLimitWindow {
            provider_id: provider_ids::CLAUDE_CODE.to_string(),
            window_name: "5h".to_string(),
            used_percent: 32.0,
            window_minutes: Some(300),
            resets_at_ms: Some(now_ms + 3_600_000),
            observed_at_ms: now_ms,
            source: Some("official_api".to_string()),
            plan_hint: Some("pro".to_string()),
        }])
        .expect("upsert authoritative observed snapshot");

    let creds_a = MemoryCredentialStore::new();
    set_sync_credentials(&creds_a, Some("sync-test-secret"), None).expect("creds a");
    storage_a
        .update_sync_config_state(true, Some(worker_url), false, None)
        .expect("sync cfg a");

    let push_a = run_sync_cycle_with_transport(&storage_a, &creds_a, transport.as_ref(), true)
        .await
        .expect("device A push");
    assert_eq!(push_a.pushed_hourly_deltas, 1);
    assert_eq!(push_a.pushed_subscriptions, 1);

    // Device B pulls in plaintext mode
    let storage_b = Storage::open_in_memory().expect("db b");
    let creds_b = MemoryCredentialStore::new();
    set_sync_credentials(&creds_b, Some("sync-test-secret"), None).expect("creds b");
    storage_b
        .update_sync_config_state(true, Some(worker_url), false, None)
        .expect("sync cfg b");

    let pull_b = run_sync_cycle_with_transport(&storage_b, &creds_b, transport.as_ref(), false)
        .await
        .expect("device B pull");
    assert_eq!(pull_b.pulled_items, 1);
    assert_eq!(pull_b.applied_hourly_buckets, 1);
    assert_eq!(pull_b.applied_subscriptions, 1);
    assert_eq!(storage_b.list_devices().expect("devices in b").len(), 2);
    assert_eq!(
        storage_b
            .list_remote_subscription_snapshots()
            .expect("remote subs")
            .len(),
        1
    );

    let overview_b = storage_b
        .query_overview(
            &AnalyticsFilter {
                start_ms: now_ms - 86_400_000,
                end_ms: now_ms + 60_000,
                provider_id: None,
                device_id: None,
                tz_offset_minutes: 0,
                all_time: false,
            },
            now_ms,
        )
        .expect("overview b");
    assert_eq!(overview_b.total_tokens, 3_050);

    // Confirm sensitive secrets never appeared in pushed JSON payloads
    {
        let raw_bodies = hub_state.raw_push_bodies.lock().expect("raw bodies");
        assert!(!raw_bodies.is_empty());
        for body in raw_bodies.iter() {
            assert!(!body.contains("secret-session-should-never-sync"));
            assert!(!body.contains("private-secret-project"));
        }
    }

    // 2. E2EE mode sync + wrong passphrase detection (isolated encrypted hub)
    let e2ee_hub_state = Arc::new(MockWorkerTransportState {
        expected_token: "sync-test-secret".to_string(),
        ..Default::default()
    });
    let e2ee_transport: Arc<dyn SyncHttpTransport> = Arc::new(MockWorkerTransport {
        state: Arc::clone(&e2ee_hub_state),
    });
    let storage_e1 = Storage::open_in_memory().expect("db e1");
    seed_device_usage_event(&storage_e1, "ev-e1", 5_000, now_ms - 30_000);
    let creds_e1 = MemoryCredentialStore::new();
    set_sync_credentials(&creds_e1, Some("sync-test-secret"), Some("correct-pass-99"))
        .expect("creds e1");
    storage_e1
        .update_sync_config_state(true, Some(worker_url), true, None)
        .expect("sync cfg e1");
    run_sync_cycle_with_transport(&storage_e1, &creds_e1, e2ee_transport.as_ref(), true)
        .await
        .expect("device e1 push e2ee");

    let storage_e2 = Storage::open_in_memory().expect("db e2");
    let creds_e2 = MemoryCredentialStore::new();
    set_sync_credentials(&creds_e2, Some("sync-test-secret"), Some("correct-pass-99"))
        .expect("creds e2");
    storage_e2
        .update_sync_config_state(true, Some(worker_url), true, None)
        .expect("sync cfg e2");
    let pull_e2 =
        run_sync_cycle_with_transport(&storage_e2, &creds_e2, e2ee_transport.as_ref(), false)
            .await
            .expect("device e2 pull e2ee");
    assert!(pull_e2.applied_hourly_buckets >= 1);

    let storage_wrong = Storage::open_in_memory().expect("db wrong");
    let creds_wrong = MemoryCredentialStore::new();
    set_sync_credentials(&creds_wrong, Some("sync-test-secret"), Some("wrong-pass"))
        .expect("creds wrong");
    storage_wrong
        .update_sync_config_state(true, Some(worker_url), true, None)
        .expect("sync cfg wrong");
    let err = run_sync_cycle_with_transport(
        &storage_wrong,
        &creds_wrong,
        e2ee_transport.as_ref(),
        false,
    )
    .await
    .expect_err("wrong passphrase must fail");
    assert!(
        err.to_string().contains("E2EE") || err.to_string().contains("passphrase"),
        "unexpected error: {err}"
    );
    assert!(storage_wrong
        .get_sync_config_state()
        .expect("cfg wrong")
        .last_sync_error
        .is_some());

    // 3. Real-time SSE sync_update triggers background SyncClientHandle pull
    let storage_sse = Storage::open_in_memory().expect("db sse");
    let creds_sse = Arc::new(MemoryCredentialStore::new());
    set_sync_credentials(creds_sse.as_ref(), Some("sync-test-secret"), Some("correct-pass-99"))
        .expect("creds sse");
    let current_max_seq = {
        let g = hub_state.records.lock().expect("lock");
        g.iter().map(|(seq, _, _)| *seq).max().unwrap_or(0)
    };
    storage_sse
        .update_sync_config_state(true, Some(worker_url), false, None)
        .expect("sync cfg sse");
    storage_sse
        .record_sync_pull_success(current_max_seq, now_ms)
        .expect("advance watermark");

    let sse_handle = SyncClientHandle::with_transport(
        storage_sse.clone(),
        creds_sse,
        Arc::clone(&transport),
    );
    let applied_notify = Arc::new(tokio::sync::Notify::new());
    let applied_notify_cb = Arc::clone(&applied_notify);
    sse_handle.start_background(move |report| {
        if report.applied_hourly_buckets > 0 {
            applied_notify_cb.notify_one();
        }
    });

    // Wait briefly until SSE stream opens, then Device A pushes ev-a-2 causing SSE push frame
    for _ in 0..30 {
        if sse_handle.is_sse_connected() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(sse_handle.is_sse_connected());

    seed_device_usage_event(&storage_a, "ev-a-2", 900, now_ms - 10_000);
    run_sync_cycle_with_transport(&storage_a, &creds_a, transport.as_ref(), true)
        .await
        .expect("push ev-a-2");

    tokio::time::timeout(std::time::Duration::from_secs(5), applied_notify.notified())
        .await
        .expect("SSE sync_update must trigger remote pull within 5s");

    sse_handle.shutdown();
}

#[test]
fn kimi_code_wire_and_context_logs_parse_without_double_counting() {
    use tokvane_collectors::providers::kimi_code::KimiCodeParser;
    use tokvane_collectors::providers::ProviderLogParser;
    use tokvane_collectors::watcher::tailer::tail_file_incremental;

    let tmp = tempdir().expect("tmpdir");
    let session_dir = tmp
        .path()
        .join(".kimi-code")
        .join("sessions")
        .join("a1b2c3d4")
        .join("sess-kimi-99");
    std::fs::create_dir_all(&session_dir).expect("mkdir");
    std::fs::write(
        session_dir.join("metadata.json"),
        r#"{"work_dir":"C:\\Projects\\my-kimi-app","model":"kimi-for-coding"}"#,
    )
    .expect("write metadata");

    let wire_path = session_dir.join("wire.jsonl");
    let wire_lines = [
        r#"{"timestamp":1763300000.0,"message":{"type":"TurnBegin","payload":{"user_input":"Refactor Kimi auth module"}}}"#,
        r#"{"timestamp":1763300002.5,"message":{"type":"StatusUpdate","payload":{"token_usage":{"input_other":1200,"output":300,"input_cache_read":800,"input_cache_creation":200}}}}"#,
        r#"{"jsonrpc":"2.0","method":"event","timestamp":1763300005.0,"params":{"type":"StatusUpdate","payload":{"token_usage":{"inputOther":500,"outputTokens":150,"inputCacheRead":250,"inputCacheCreation":0}}}}"#,
        r#"{"timestamp":1763300006.0,"type":"usage.record","scope":"session","usage":{"inputOther":1700,"output":450}}"#,
    ]
    .join("\n");
    std::fs::write(&wire_path, format!("{wire_lines}\n")).expect("write wire.jsonl");

    // Sibling context.jsonl alongside wire.jsonl must be skipped to prevent double-counting.
    let context_sibling = session_dir.join("context.jsonl");
    std::fs::write(
        &context_sibling,
        "{\"role\":\"_usage\",\"token_count\":2500}\n",
    )
    .expect("write sibling context.jsonl");

    // Separate legacy session with ONLY context.jsonl (no wire.jsonl) must parse deltas accurately.
    let legacy_dir = tmp
        .path()
        .join(".kimi")
        .join("sessions")
        .join("legacy-ws")
        .join("sess-legacy-1");
    std::fs::create_dir_all(&legacy_dir).expect("mkdir legacy");
    let legacy_context = legacy_dir.join("context.jsonl");
    std::fs::write(
        &legacy_context,
        "{\"role\":\"_usage\",\"token_count\":1000,\"timestamp\":1763300010}\n{\"role\":\"_usage\",\"token_count\":1800,\"timestamp\":1763300020}\n",
    )
    .expect("write legacy context");

    let parser = KimiCodeParser::new();
    assert!(parser.matches_file(&wire_path));
    assert!(!parser.matches_file(&context_sibling));
    assert!(parser.matches_file(&legacy_context));

    let wire_ctx = parser.init_context(&wire_path, "dev-test");
    assert_eq!(wire_ctx.session_id, "sess-kimi-99");
    assert_eq!(wire_ctx.project_label, "my-kimi-app");
    assert_eq!(wire_ctx.current_model.as_deref(), Some("kimi-for-coding"));

    let storage = Storage::open_in_memory().expect("db");
    let dev_id = storage.local_device().expect("device").device_id;

    let out_wire = tail_file_incremental(&storage, &parser, &wire_path, &dev_id).expect("tail wire");
    assert_eq!(out_wire.events_inserted, 2);

    let out_legacy =
        tail_file_incremental(&storage, &parser, &legacy_context, &dev_id).expect("tail legacy");
    assert_eq!(out_legacy.events_inserted, 2);

    let ov = storage
        .query_overview(
            &AnalyticsFilter {
                start_ms: 1_763_000_000_000,
                end_ms: 1_764_000_000_000,
                provider_id: Some(provider_ids::KIMI_CODE.to_string()),
                device_id: None,
                tz_offset_minutes: 480,
                all_time: false,
            },
            1_763_300_100_000,
        )
        .expect("kimi overview");
    // wire: (1200+300+800+200=2500) + (500+150+250=900) = 3400; legacy deltas: 1000 + 800 = 1800; total = 5200
    assert_eq!(ov.total_tokens, 5_200);
    assert!(ov.active_sessions.iter().any(|s| {
        s.provider_id == provider_ids::KIMI_CODE
            && s.project_label == "my-kimi-app"
            && s.model_id == "kimi-for-coding"
            && s.session_title.as_deref() == Some("Refactor Kimi auth module")
            && s.total_tokens == 3_400
    }));

    // Verify Codex rollout with injected AGENTS.md / 工作偏好 preamble resolves the real user_message title
    let codex_file = tmp.path().join("rollout-2025-11-16T13-33-00-sess-codex-99.jsonl");
    let codex_lines = [
        r#"{"timestamp":"2025-11-16T13:33:20Z","type":"session_meta","payload":{"id":"019a9000-1111-7222-8333-444455556666","cwd":"C:\\Projects\\TokenVane"}}"#,
        r##"{"timestamp":"2025-11-16T13:33:21Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for C:\\Projects\\TokenVane\n\n<INSTRUCTIONS>\n# 工作偏好\n\n这些是跨项目的工作偏好，默认长期有效。\n</INSTRUCTIONS>"},{"type":"input_text","text":"<environment_context>\n  <cwd>C:\\Projects\\TokenVane</cwd>\n</environment_context>"}]}}"##,
        r#"{"timestamp":"2025-11-16T13:33:22Z","type":"event_msg","payload":{"type":"user_message","message":"修复悬浮窗与会话名称读取问题"}}"#,
        r#"{"timestamp":"2025-11-16T13:33:25Z","type":"event_msg","payload":{"type":"token_count","info":{"model":"gpt-5.2-codex","last_token_usage":{"input_tokens":2000,"cached_input_tokens":1500,"output_tokens":300,"reasoning_output_tokens":60}}}}"#,
    ]
    .join("\n");
    std::fs::write(&codex_file, format!("{codex_lines}\n")).expect("write codex rollout");
    let codex_parser = CodexParser::new();
    tail_file_incremental(&storage, &codex_parser, &codex_file, &dev_id).expect("tail codex");
    let codex_ov = storage
        .query_overview(
            &AnalyticsFilter {
                start_ms: 1_763_000_000_000,
                end_ms: 1_764_000_000_000,
                provider_id: Some(provider_ids::CODEX.to_string()),
                device_id: None,
                tz_offset_minutes: 480,
                all_time: false,
            },
            1_763_300_100_000,
        )
        .expect("codex overview");
    assert_eq!(
        codex_ov.active_sessions[0].session_title.as_deref(),
        Some("修复悬浮窗与会话名称读取问题")
    );
}

#[tokio::test]
async fn kimi_coding_usages_api_and_qcode_auto_materialize_subscriptions() {
    use tokvane_collectors::credentials::MemoryCredentialStore;
    use tokvane_collectors::providers::official_api::refresh_provider_balance_with_fetcher_inner;

    let storage = Storage::open_in_memory().expect("db");
    let creds = MemoryCredentialStore::new();
    let endpoints = BalanceApiEndpoints::default();
    let now_ms = 1_763_300_000_000i64;

    creds
        .set_api_key(provider_ids::KIMI_CODE, "sk-kimi-live-membership-key")
        .expect("set kimi coding key");
    creds
        .set_api_key(provider_ids::QCODE, "qot-qcode-key")
        .expect("set qcode key");

    let mut responses = HashMap::new();
    // Profile endpoint: only the plan level may be read; the other fields must be ignored.
    responses.insert(
        "https://api.kimi.com/coding/v1/me".to_string(),
        (
            200,
            serde_json::json!({
                "user_id": "u-test",
                "nickname": "tester",
                "user_level": 50,
                "user_level_name": "Allegro"
            })
            .to_string(),
        ),
    );
    responses.insert(
        endpoints.kimi_coding_url.clone(),
        (
            200,
            serde_json::json!({
                "user": { "membership": { "level": "LEVEL_INTERMEDIATE" } },
                "usage": {
                    "limit": "200",
                    "used": "40",
                    "remaining": "160",
                    "resetTime": "2026-11-22T00:00:00Z"
                },
                "limits": [
                    {
                        "window": { "duration": 300, "timeUnit": "TIME_UNIT_MINUTE" },
                        "detail": {
                            "limit": "100",
                            "used": "30",
                            "remaining": "70",
                            "resetTime": "2026-11-16T05:00:00Z"
                        }
                    }
                ]
            })
            .to_string(),
        ),
    );
    responses.insert(
        endpoints.qcode_url.clone(),
        (
            200,
            serde_json::json!({
                "data": {
                    "balance": 12.0,
                    "daily_used": 3.0,
                    "daily_limit": 10.0,
                    "currency": "USD"
                }
            })
            .to_string(),
        ),
    );

    let fetcher = MockAuthFetcher {
        responses: Mutex::new(responses),
        calls: Mutex::new(Vec::new()),
    };

    let kimi_res = refresh_provider_balance_with_fetcher_inner(
        &storage,
        provider_ids::KIMI_CODE,
        &creds,
        &fetcher,
        &endpoints,
        now_ms,
        true,
    )
    .await
    .expect("kimi coding usages");
    assert_eq!(kimi_res.endpoint, endpoints.kimi_coding_url);
    assert_eq!(kimi_res.snapshot.currency, "QUOTA");
    assert_eq!(kimi_res.rate_limits_updated, 2);

    let qcode_res = refresh_provider_balance_with_fetcher_inner(
        &storage,
        provider_ids::QCODE,
        &creds,
        &fetcher,
        &endpoints,
        now_ms,
        true,
    )
    .await
    .expect("qcode usage");
    assert_eq!(qcode_res.rate_limits_updated, 1);

    let subs = storage.query_subscriptions(now_ms).expect("auto subs");
    let kimi_sub = subs
        .iter()
        .find(|s| s.plan.provider_id == provider_ids::KIMI_CODE)
        .expect("kimi auto sub present");
    assert_eq!(kimi_sub.plan.plan_id, "auto_kimi_code");
    assert_eq!(kimi_sub.plan.auto_source.as_deref(), Some("official_api"));
    // The profile level wins over the legacy usages hint.
    assert_eq!(kimi_sub.plan.plan_name, "Kimi Max (Allegro)");
    assert_eq!(kimi_sub.plan.monthly_fee_micro_usd, 99_000_000);
    assert_eq!(kimi_sub.windows.len(), 2);

    let qcode_sub = subs
        .iter()
        .find(|s| s.plan.provider_id == provider_ids::QCODE)
        .expect("qcode auto sub present");
    assert_eq!(qcode_sub.plan.plan_id, "auto_qcode");
    assert_eq!(qcode_sub.windows[0].limit_value, 10_000_000);
    assert!((qcode_sub.windows[0].used_percent - 30.0).abs() < 0.01);
}

#[test]
fn incremental_tailing_past_large_gap_preserves_codex_model_switches_without_gpt5_codex_fallback() {
    let dir = tempfile::tempdir().expect("tempdir");
    let rollout_path = dir.path().join("rollout-large-gap.jsonl");
    let storage = Storage::open_in_memory().expect("db");
    let dev = storage.local_device().expect("dev").device_id;
    let parser = CodexParser::new();

    // 1. Write session_meta (no model), user_message, initial turn_context (gpt-5.3-codex),
    //    >80KB of bulky tool output lines + mid-file turn_context switch to gpt-5.4.
    let mut initial_content = String::new();
    initial_content.push_str(
        r#"{"timestamp":"2026-11-16T05:00:00.000Z","type":"session_meta","payload":{"id":"codex-gap-s1","cwd":"C:/Projects/TokenVane"}}"#,
    );
    initial_content.push('\n');
    initial_content.push_str(
        r#"{"timestamp":"2026-11-16T05:00:01.000Z","type":"event_msg","payload":{"type":"user_message","message":"Inspect tailer model switch"}}"#,
    );
    initial_content.push('\n');
    initial_content.push_str(
        r#"{"timestamp":"2026-11-16T05:00:02.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
    );
    initial_content.push('\n');
    let big_pad = "x".repeat(4_096);
    for idx in 0..10 {
        initial_content.push_str(&format!(
            r#"{{"timestamp":"2026-11-16T05:00:03.000Z","type":"response_item","payload":{{"role":"assistant","content":"step-{idx}-{big_pad}"}}}}"#
        ));
        initial_content.push('\n');
    }
    initial_content.push_str(
        r#"{"timestamp":"2026-11-16T05:01:00.000Z","type":"turn_context","payload":{"collaboration_mode":{"settings":{"model":"gpt-5.4"}}}}"#,
    );
    initial_content.push('\n');
    for idx in 10..32 {
        initial_content.push_str(&format!(
            r#"{{"timestamp":"2026-11-16T05:01:05.000Z","type":"response_item","payload":{{"role":"assistant","content":"post-{idx}-{big_pad}"}}}}"#
        ));
        initial_content.push('\n');
    }
    std::fs::write(&rollout_path, &initial_content).expect("write initial");

    // Advance cursor past 100KB with zero token_count events yet
    let first_pass =
        tail_file_incremental(&storage, &parser, &rollout_path, &dev).expect("initial tail");
    assert_eq!(first_pass.events_inserted, 0);

    // Append token_count event AFTER >90KB gap since gpt-5.4 turn_context
    let mut updated = initial_content;
    updated.push_str(
        r#"{"timestamp":"2026-11-16T05:02:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1200,"cached_input_tokens":200,"output_tokens":300,"reasoning_output_tokens":50},"total_token_usage":{"input_tokens":1200,"cached_input_tokens":200,"output_tokens":300,"reasoning_output_tokens":50}}}}"#,
    );
    updated.push('\n');
    std::fs::write(&rollout_path, updated).expect("append token_count");

    let second_pass =
        tail_file_incremental(&storage, &parser, &rollout_path, &dev).expect("second tail");
    assert_eq!(second_pass.events_inserted, 1);

    let report = storage
        .query_models_pricing(&AnalyticsFilter {
            start_ms: 0,
            end_ms: i64::MAX / 2,
            provider_id: Some("codex".to_string()),
            device_id: None,
            tz_offset_minutes: 480,
            all_time: true,
        })
        .expect("query models");
    assert_eq!(report.rows.len(), 1);
    assert_eq!(report.rows[0].model_id, "gpt-5.4");
}

#[tokio::test]
async fn antigravity_cli_quota_refresh_persists_pools_and_honors_cooldown() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokvane_collectors::providers::antigravity::parse_antigravity_cli_usage_output;
    use tokvane_collectors::providers::official_api::{
        refresh_antigravity_cli_quota_with_runner, ANTIGRAVITY_CLI_ENDPOINT,
    };

    let storage = Storage::open_in_memory().expect("db");
    let now_ms = 1_794_840_000_000i64;
    let calls = AtomicUsize::new(0);

    let first = refresh_antigravity_cli_quota_with_runner(&storage, now_ms, false, || {
        calls.fetch_add(1, Ordering::SeqCst);
        async move {
            let sample = r#"{
              "status": "SUCCESS",
              "command": {
                "name": "usage",
                "data": {
                  "groups": [
                    {
                      "name": "Gemini Models",
                      "buckets": [
                        {"id": "gemini-5h", "window": "5h", "remaining_fraction": 0.635, "reset_time": "2026-11-16T17:13:50Z"},
                        {"id": "gemini-weekly", "window": "weekly", "remaining_fraction": 0.361, "reset_time": "2026-11-22T14:36:11Z"}
                      ]
                    },
                    {
                      "name": "Claude and GPT models",
                      "buckets": [
                        {"id": "3p-5h", "window": "5h", "remaining_fraction": 1.0, "reset_time": "2026-11-16T20:57:37Z"},
                        {"id": "3p-weekly", "window": "weekly", "remaining_fraction": 0.995, "reset_time": "2026-11-22T16:34:57Z"}
                      ]
                    }
                  ]
                }
              }
            }"#;
            parse_antigravity_cli_usage_output(sample, now_ms, Some("ultra".to_string()))
        }
    })
    .await
    .expect("first refresh succeeds");

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!first.from_cache);
    assert_eq!(first.rate_limits_updated, 4);
    assert_eq!(first.endpoint, ANTIGRAVITY_CLI_ENDPOINT);
    assert_eq!(first.snapshot.currency, "QUOTA");

    let subs = storage.query_subscriptions(now_ms).expect("subs");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].plan.plan_name, "Google AI Ultra");
    assert_eq!(subs[0].plan.auto_source.as_deref(), Some("official_cli"));
    assert_eq!(subs[0].windows.len(), 4);
    assert_eq!(subs[0].windows[0].source, "official_cli");

    // Second call 10 seconds later returns 60s cooldown cache without spawning CLI again
    let cached = refresh_antigravity_cli_quota_with_runner(&storage, now_ms + 10_000, false, || {
        calls.fetch_add(1, Ordering::SeqCst);
        async { unreachable!("must use cooldown cache") }
    })
    .await
    .expect("cooldown cache hit");
    assert!(cached.from_cache);
    assert_eq!(cached.cooldown_remaining_secs, 50);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // Auth failure persists cli_auth_paused_reason in SQLite and blocks future runs until cleared
    let err = refresh_antigravity_cli_quota_with_runner(&storage, now_ms + 70_000, true, || {
        calls.fetch_add(1, Ordering::SeqCst);
        async {
            Err(tokvane_collectors::CollectorError::Compliance(
                "agy CLI 返回状态 ERROR: Not signed in".to_string(),
            ))
        }
    })
    .await
    .expect_err("auth error fails");
    assert!(err.to_string().contains("Not signed in"));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(storage
        .get_cli_auth_paused_reason(provider_ids::ANTIGRAVITY)
        .expect("query pause")
        .is_some());

    // Subsequent attempt short-circuits before invoking runner
    let skipped = refresh_antigravity_cli_quota_with_runner(&storage, now_ms + 140_000, true, || {
        calls.fetch_add(1, Ordering::SeqCst);
        async { unreachable!("must skip while cli_auth_paused_reason is set") }
    })
    .await
    .expect_err("paused probe short-circuits");
    assert!(skipped.to_string().contains("Not signed in"));
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    // Clearing the persisted pause (manual sync/login) resumes probing and succeeds
    storage
        .set_cli_auth_paused_reason(provider_ids::ANTIGRAVITY, None)
        .expect("clear pause");
    let resumed = refresh_antigravity_cli_quota_with_runner(&storage, now_ms + 200_000, true, || {
        calls.fetch_add(1, Ordering::SeqCst);
        async move {
            let sample = r#"{
              "status": "SUCCESS",
              "command": {
                "name": "usage",
                "data": {
                  "groups": [
                    {
                      "name": "Gemini Models",
                      "buckets": [
                        {"id": "gemini-5h", "window": "5h", "remaining_fraction": 0.8, "reset_time": "2026-11-16T17:13:50Z"}
                      ]
                    }
                  ]
                }
              }
            }"#;
            parse_antigravity_cli_usage_output(sample, now_ms + 200_000, None)
        }
    })
    .await
    .expect("resumed refresh succeeds");
    assert_eq!(resumed.rate_limits_updated, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(
        storage
            .get_cli_auth_paused_reason(provider_ids::ANTIGRAVITY)
            .expect("cleared after success"),
        None
    );
}

