import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  AnalyticsFilter,
  AppPreferences,
  BalanceRefreshOutcome,
  BootstrapState,
  CatalogSyncResult,
  CloudSyncProvisionInput,
  CloudSyncProvisionResult,
  CloudSyncProvisionStep,
  CustomProviderRule,
  CustomRuleTestResult,
  DeviceRecord,
  ExportReportResult,
  IngestionCursor,
  ModelPricingEntry,
  ModelStatsReport,
  MonthlyQuery,
  MonthlyReport,
  ObservedRateLimitWindow,
  OfficialApiReconcileResult,
  OverviewReport,
  PaygAccount,
  PaygAccountStatus,
  ProviderBalanceSnapshot,
  ProviderConfig,
  RemoteSubscriptionSnapshot,
  ScanReport,
  SpeedLatencyReport,
  SubscriptionPlan,
  SubscriptionPlanStatus,
  SubscriptionPlanTemplate,
  SyncEnvelopeV1,
  SyncPairingBundle,
  SyncRunReport,
  SyncStatusView,
  TraySummary,
  TrendsQuery,
  TrendsReport,
} from '../types/ipc';

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

export function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && Boolean(window.__TAURI_INTERNALS__);
}

function nowMinusHours(hours: number): number {
  return Date.now() - hours * 3_600_000;
}

let previewFloatingVisible = false;

function defaultPreviewPreferences(now: number): AppPreferences {
  return {
    currency: 'USD',
    usd_to_cny_rate: '7.25',
    theme: 'dark',
    minimize_to_tray: true,
    launch_at_startup: false,
    otlp_enabled: false,
    otlp_port: 4318,
    device_display_name: 'win-workstation',
    bg_mode: 'none',
    bg_image_path: null,
    bg_preset_id: 'aurora_slate',
    bg_opacity: 0.18,
    bg_blur_px: 16,
    auto_sync_pricing: true,
    exchange_rate_auto: true,
    auto_check_updates: true,
    floating_window_enabled: previewFloatingVisible,
    background_cli_quota_poll: true,
    last_rate_sync_at_ms: null,
    last_pricing_sync_at_ms: null,
    last_subscription_sync_at_ms: null,
    updated_at_ms: now,
  };
}

function previewSubscriptionTemplates(): SubscriptionPlanTemplate[] {
  const mkRollingPair = (
    limit5h: number,
    limit7d: number
  ): SubscriptionPlanTemplate['windows'] => [
    {
      window_id: '5h',
      label: '5h 滚动窗口',
      duration_hours: 5,
      rolling: true,
      unit: 'micro_usd',
      limit_value: limit5h,
      warning_percent: 80,
      critical_percent: 95,
    },
    {
      window_id: '7d',
      label: '7d 周额度',
      duration_hours: 168,
      rolling: true,
      unit: 'micro_usd',
      limit_value: limit7d,
      warning_percent: 80,
      critical_percent: 95,
    },
  ];

  return [
    {
      template_id: 'claude_pro',
      provider_id: 'claude_code',
      plan_name: 'Claude Pro',
      monthly_fee_micro_usd: 20_000_000,
      description: 'Anthropic Claude Pro 个人订阅（含 5h 滚动与 7d 周参考窗口）',
      windows: mkRollingPair(15_000_000, 75_000_000),
      live_windows: [],
    },
    {
      template_id: 'claude_max_5x',
      provider_id: 'claude_code',
      plan_name: 'Claude Max 5x',
      monthly_fee_micro_usd: 100_000_000,
      description: 'Anthropic Claude Max 5x 高配额订阅（5 倍 Pro 参考额度）',
      windows: mkRollingPair(75_000_000, 375_000_000),
      live_windows: [],
    },
    {
      template_id: 'claude_max_20x',
      provider_id: 'claude_code',
      plan_name: 'Claude Max 20x',
      monthly_fee_micro_usd: 200_000_000,
      description: 'Anthropic Claude Max 20x 重度编码订阅（20 倍 Pro 参考额度）',
      windows: mkRollingPair(200_000_000, 1_200_000_000),
      live_windows: [],
    },
    {
      template_id: 'codex_plus',
      provider_id: 'codex',
      plan_name: 'ChatGPT Plus / Codex',
      monthly_fee_micro_usd: 20_000_000,
      description: 'OpenAI ChatGPT Plus（可自动对齐 Codex 日志官方 5h / 7d 限额）',
      windows: mkRollingPair(18_000_000, 90_000_000),
      live_windows: [],
    },
    {
      template_id: 'codex_pro',
      provider_id: 'codex',
      plan_name: 'ChatGPT Pro / Codex',
      monthly_fee_micro_usd: 200_000_000,
      description: 'OpenAI ChatGPT Pro（可自动对齐 Codex 日志官方 5h / 7d 限额）',
      windows: mkRollingPair(150_000_000, 900_000_000),
      live_windows: [],
    },
    {
      template_id: 'google_ai_pro',
      provider_id: 'antigravity',
      plan_name: 'Google AI Pro',
      monthly_fee_micro_usd: 19_990_000,
      description: 'Google AI Pro / Antigravity 个人订阅参考窗口',
      windows: mkRollingPair(20_000_000, 100_000_000),
      live_windows: [],
    },
    {
      template_id: 'google_ai_ultra',
      provider_id: 'antigravity',
      plan_name: 'Google AI Ultra',
      monthly_fee_micro_usd: 249_990_000,
      description: 'Google AI Ultra / Antigravity 旗舰订阅参考窗口',
      windows: mkRollingPair(250_000_000, 1_250_000_000),
      live_windows: [],
    },
    {
      template_id: 'kimi_code_allegretto',
      provider_id: 'kimi_code',
      plan_name: 'Kimi Code Allegretto',
      monthly_fee_micro_usd: 7_000_000,
      description: 'Kimi Code 个人会员（配置 `sk-kimi-...` 官方 Key 后自动对接 `/coding/v1/usages` 实时限额）',
      windows: mkRollingPair(8_000_000, 40_000_000),
      live_windows: [],
    },
    {
      template_id: 'minimax_coding_plan',
      provider_id: 'minimax_code',
      plan_name: 'MiniMax Coding Plan',
      monthly_fee_micro_usd: 15_000_000,
      description: 'MiniMax Coding Plan 编码订阅参考窗口（支持官方 Coding Plan 额度接口自动同步）',
      windows: mkRollingPair(12_000_000, 60_000_000),
      live_windows: [],
    },
    {
      template_id: 'minimax_token_plan',
      provider_id: 'minimax_code',
      plan_name: 'MiniMax Token Plan',
      monthly_fee_micro_usd: 49_000_000,
      description: 'MiniMax Token Plan 按 Token 计量订阅参考模板（可按需修改额度）',
      windows: [
        {
          window_id: '5h',
          label: '5h Token 窗口',
          duration_hours: 5,
          rolling: true,
          unit: 'tokens',
          limit_value: 20_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
        {
          window_id: '30d',
          label: '月度 Token 额度',
          duration_hours: 720,
          rolling: false,
          unit: 'tokens',
          limit_value: 120_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
      ],
      live_windows: [],
    },
    {
      template_id: 'qcode_daily_plan',
      provider_id: 'qcode',
      plan_name: 'QCode 日额度套餐',
      monthly_fee_micro_usd: 15_000_000,
      description: 'QCode 每日额度参考模板（配置官方 Key 后自动同步日额度用量）',
      windows: [
        {
          window_id: '24h',
          label: '24h 日额度',
          duration_hours: 24,
          rolling: false,
          unit: 'micro_usd',
          limit_value: 10_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
      ],
      live_windows: [],
    },
    {
      template_id: 'cursor_pro',
      provider_id: 'cursor',
      plan_name: 'Cursor Pro',
      monthly_fee_micro_usd: 20_000_000,
      description: 'Cursor Pro 月度订阅（USD 20/月，来自 AI Subscription Catalog）',
      windows: [],
      live_windows: [],
    },
    {
      template_id: 'github_copilot_pro',
      provider_id: 'github_copilot',
      plan_name: 'GitHub Copilot Pro',
      monthly_fee_micro_usd: 10_000_000,
      description: 'GitHub Copilot Pro 月度订阅（USD 10/月，来自 AI Subscription Catalog）',
      windows: [],
      live_windows: [],
    },
    {
      template_id: 'windsurf_pro',
      provider_id: 'windsurf',
      plan_name: 'Windsurf Pro',
      monthly_fee_micro_usd: 15_000_000,
      description: 'Windsurf Pro 月度订阅（USD 15/月，来自 AI Subscription Catalog）',
      windows: [],
      live_windows: [],
    },
  ];
}

/**
 * Lightweight browser-preview fallback used ONLY when the React UI is loaded directly
 * in Vite/Vitest outside Tauri native runtime. Inside Tauri desktop, all calls go directly
 * to Rust + SQLite via `invoke`.
 */
function buildPreviewFallback<T>(command: string, args?: Record<string, unknown>): T {
  const now = Date.now();
  const baseHour = Math.floor(now / 3_600_000) * 3_600_000;
  switch (command) {
    case 'get_bootstrap_state': {
      const data: BootstrapState = {
        db_path: '%LOCALAPPDATA%\\TokenVane\\tokenvane.db',
        device: {
          device_id: 'preview-local-device',
          device_name: 'win-workstation',
          platform: 'windows',
          created_at_ms: nowMinusHours(72),
        },
        preferences: defaultPreviewPreferences(now),
        providers: [
          {
            provider_id: 'claude_code',
            enabled: true,
            collection_mode: 'local_log',
            custom_log_paths: [],
            has_credential: false,
            last_sync_at_ms: now - 15_000,
            last_error: null,
            updated_at_ms: now,
          },
          {
            provider_id: 'codex',
            enabled: true,
            collection_mode: 'local_log',
            custom_log_paths: [],
            has_credential: false,
            last_sync_at_ms: now - 20_000,
            last_error: null,
            updated_at_ms: now,
          },
          {
            provider_id: 'antigravity',
            enabled: true,
            collection_mode: 'local_log',
            custom_log_paths: [],
            has_credential: false,
            last_sync_at_ms: now - 10_000,
            last_error: null,
            updated_at_ms: now,
          },
          {
            provider_id: 'kimi_code',
            enabled: true,
            collection_mode: 'local_log',
            custom_log_paths: [],
            has_credential: false,
            last_sync_at_ms: null,
            last_error: null,
            updated_at_ms: now,
          },
          {
            provider_id: 'workbuddy',
            enabled: true,
            collection_mode: 'local_log',
            custom_log_paths: [],
            has_credential: false,
            last_sync_at_ms: null,
            last_error: null,
            updated_at_ms: now,
          },
          {
            provider_id: 'deepseek_harness',
            enabled: true,
            collection_mode: 'local_log',
            custom_log_paths: [],
            has_credential: false,
            last_sync_at_ms: null,
            last_error: null,
            updated_at_ms: now,
          },
          {
            provider_id: 'qcode',
            enabled: true,
            collection_mode: 'local_log',
            custom_log_paths: [],
            has_credential: false,
            last_sync_at_ms: null,
            last_error: null,
            updated_at_ms: now,
          },
          {
            provider_id: 'minimax_code',
            enabled: true,
            collection_mode: 'local_log',
            custom_log_paths: [],
            has_credential: false,
            last_sync_at_ms: null,
            last_error: null,
            updated_at_ms: now,
          },
        ],
        collector_watching: true,
        otlp_running: false,
        otlp_port: null,
        last_scan: {
          files_scanned: 0,
          events_inserted: 0,
          events_deduplicated: 0,
          errors: [],
          scanned_at_ms: now,
        },
      };
      return data as T;
    }
    case 'get_overview': {
      const data: OverviewReport = {
        total_tokens: 0,
        total_cost_micro_usd: 0,
        today_tokens: 0,
        today_cost_micro_usd: 0,
        today_requests: 0,
        today_cache_hit_ratio: 0,
        today_cache_saved_micro_usd: 0,
        today_tokens_delta_pct: null,
        today_cost_delta_pct: null,
        today_hourly_pulse: [],
        cache_hit_ratio: 0,
        avg_output_tps: 0,
        peak_output_tps: 0,
        latest_output_tps: null,
        total_requests: 0,
        observable_steps: 0,
        tokens_delta_pct: null,
        cost_delta_pct: null,
        cache_delta_pct: null,
        tps_delta_pct: null,
        series_granularity: 'hourly',
        series: [
          {
            bucket_start_ms: baseHour,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            cost_micro_usd: 0,
            requests: 0,
          },
        ],
        provider_series: [],
        providers: [],
        provider_shares: [],
        top_model_shares: [],
        active_sessions: [],
      };
      return data as T;
    }
    case 'get_trends':
      return {
        buckets: [],
        series: [],
        comparisons: [],
        effective_granularity: 'daily',
      } as T;
    case 'get_monthly':
      return {
        year: new Date().getFullYear(),
        month: new Date().getMonth() + 1,
        api_equivalent_cost_micro_usd: 0,
        subscription_monthly_fee_micro_usd: 0,
        roi_multiplier: null,
        month_total_tokens: 0,
        month_total_requests: 0,
        month_cache_saved_micro_usd: 0,
        provider_shares: [],
        model_shares: [],
        daily_ledger: [],
        heatmap_365d: [],
      } as T;
    case 'get_models_pricing':
      return {
        rows: [],
        total_cost_micro_usd: 0,
        total_cache_saved_micro_usd: 0,
        catalog: [],
        subscription_allocations: [],
      } as T;
    case 'get_subscriptions':
    case 'get_provider_configs':
    case 'get_ingestion_cursors':
    case 'get_observed_rate_limits':
    case 'list_custom_provider_rules':
    case 'upsert_custom_provider_rule':
    case 'delete_custom_provider_rule':
    case 'get_provider_balances':
      return [] as T;
    case 'test_custom_provider_sample_line':
      return {
        matched: false,
        provider_id: 'custom_preview',
        model_id: null,
        timestamp_ms: null,
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        reasoning_tokens: 0,
        estimated_cost_micro_usd: 0,
        diagnostic: '浏览器预览模式示例结果',
      } as T;
    case 'refresh_provider_balance': {
      const targetProvider = (args?.providerId as string | undefined) ?? 'kimi_code';
      const isQuotaCli = targetProvider === 'antigravity' || targetProvider === 'claude_code';
      const cliEndpoint =
        targetProvider === 'claude_code'
          ? 'claude auth status --json + claude -p -- /usage'
          : 'agy -p "/usage" --output-format json';
      return {
        snapshot: {
          provider_id: targetProvider,
          currency: isQuotaCli ? 'QUOTA' : 'CNY',
          total_balance_micro: null,
          granted_balance_micro: null,
          topped_up_balance_micro: null,
          daily_quota_used_micro: null,
          daily_quota_limit_micro: null,
          fetched_at_ms: now,
        },
        rate_limits_updated: isQuotaCli ? (targetProvider === 'claude_code' ? 2 : 4) : 0,
        endpoint: isQuotaCli ? cliEndpoint : 'preview://balance',
        from_cache: !isQuotaCli,
        cooldown_remaining_secs: isQuotaCli ? 0 : 60,
      } as T;
    }
    case 'launch_provider_cli_login':
      return '已在系统终端打开登录窗口' as T;
    case 'get_subscription_templates':
      return previewSubscriptionTemplates() as T;
    case 'get_speed_latency':
      return { providers: [], models: [] } as T;
    case 'get_tray_summary':
      return {
        today_tokens: 0,
        today_cost_micro_usd: 0,
        today_requests: 0,
        latest_output_tps: null,
        subscriptions: [],
        provider_shares: [],
        active_sessions: [],
        currency: 'USD',
        usd_to_cny_rate: '7.25',
        active_providers_count: 3,
        total_ingested_events: 0,
      } as T;
    case 'get_preferences':
      return defaultPreviewPreferences(now) as T;
    case 'update_preferences':
      return ((args?.prefs as AppPreferences | undefined) ?? defaultPreviewPreferences(now)) as T;
    case 'get_background_image_data_url':
      return null as T;
    case 'import_background_image':
      return ((args?.sourcePath as string | undefined) || 'preview-wallpaper.png') as T;
    case 'delete_subscription_plan':
    case 'set_subscription_plan_enabled':
    case 'clear_background_image':
      return undefined as T;
    case 'trigger_scan_now':
      return {
        files_scanned: 0,
        events_inserted: 0,
        events_deduplicated: 0,
        rate_limits_updated: 0,
        errors: [],
        scanned_at_ms: now,
      } as T;
    case 'sync_pricing_and_rate':
      return {
        rate_updated: false,
        usd_to_cny_rate: '7.25',
        pricing_updated: false,
        synced_models_count: 0,
        recomputed_events_count: 0,
        subscriptions_updated: false,
        synced_plans_count: 0,
        synced_at_ms: now,
        rate_source: null,
        pricing_source: null,
        subscription_source: null,
        warning: null,
      } as T;
    case 'preview_sync_envelope_v1':
      return {
        schema_version: 1,
        device_id: 'preview-local-device',
        device_display_name: 'win-workstation',
        generated_at_ms: now,
        since_watermark_ms: 0,
        hourly_deltas: [],
        subscription_snapshots: [],
      } as T;
    case 'list_devices':
      return [
        {
          device_id: 'preview-local-device',
          device_name: 'Local Workstation',
          platform: 'windows',
          created_at_ms: now - 86_400_000,
        },
      ] as T;
    case 'get_sync_status':
      return {
        enabled: false,
        worker_url: null,
        e2ee_enabled: false,
        e2ee_kdf_salt_hex: null,
        has_bearer_token: false,
        has_e2ee_passphrase: false,
        sse_connected: false,
        last_push_watermark_ms: 0,
        last_pull_watermark_ms: 0,
        last_sync_at_ms: null,
        last_sync_error: null,
        updated_at_ms: now,
      } as T;
    case 'update_sync_config': {
      const enabled = Boolean(args?.enabled);
      const workerUrl = (args?.workerUrl as string | null | undefined) ?? null;
      const e2eeEnabled = Boolean(args?.e2eeEnabled);
      return {
        enabled,
        worker_url: workerUrl,
        e2ee_enabled: e2eeEnabled,
        e2ee_kdf_salt_hex: e2eeEnabled ? '00112233445566778899aabbccddeeff' : null,
        has_bearer_token: false,
        has_e2ee_passphrase: false,
        sse_connected: false,
        last_push_watermark_ms: 0,
        last_pull_watermark_ms: 0,
        last_sync_at_ms: null,
        last_sync_error: null,
        updated_at_ms: now,
      } as T;
    }
    case 'set_sync_credentials':
      return {
        enabled: false,
        worker_url: null,
        e2ee_enabled: false,
        e2ee_kdf_salt_hex: null,
        has_bearer_token: Boolean(args?.bearerToken),
        has_e2ee_passphrase: Boolean(args?.e2eePassphrase),
        sse_connected: false,
        last_push_watermark_ms: 0,
        last_pull_watermark_ms: 0,
        last_sync_at_ms: null,
        last_sync_error: null,
        updated_at_ms: now,
      } as T;
    case 'clear_sync_credentials':
      return {
        enabled: false,
        worker_url: null,
        e2ee_enabled: false,
        e2ee_kdf_salt_hex: null,
        has_bearer_token: false,
        has_e2ee_passphrase: false,
        sse_connected: false,
        last_push_watermark_ms: 0,
        last_pull_watermark_ms: 0,
        last_sync_at_ms: null,
        last_sync_error: null,
        updated_at_ms: now,
      } as T;
    case 'save_cloud_sync_setup': {
      const workerUrl = (args?.workerUrl as string | null) ?? 'https://sync.example.workers.dev';
      const enabled = Boolean(args?.enabled ?? true);
      const e2eeEnabled = Boolean(args?.e2eeEnabled ?? false);
      return {
        enabled,
        worker_url: workerUrl,
        e2ee_enabled: e2eeEnabled,
        e2ee_kdf_salt_hex: e2eeEnabled ? 'preview-salt' : null,
        has_bearer_token: Boolean(args?.bearerToken),
        has_e2ee_passphrase: Boolean(args?.e2eePassphrase),
        sse_connected: false,
        last_push_watermark_ms: 0,
        last_pull_watermark_ms: 0,
        last_sync_at_ms: null,
        last_sync_error: null,
        updated_at_ms: now,
      } as T;
    }
    case 'get_sync_pairing_bundle': {
      const bundle: SyncPairingBundle = {
        pairing_uri: 'tokenvane://pair?v=1&url=https%3A%2F%2Fsync.example.workers.dev&token=preview&e2ee=0',
        pairing_json: JSON.stringify({
          v: 1,
          url: 'https://sync.example.workers.dev',
          token: 'preview',
          e2ee: false,
        }),
        worker_url: 'https://sync.example.workers.dev',
        e2ee_enabled: false,
        includes_passphrase: Boolean(args?.includePassphrase),
      };
      return bundle as T;
    }
    case 'provision_cloud_sync_worker': {
      const input = args?.input as CloudSyncProvisionInput | undefined;
      const workerName = input?.worker_name?.trim() || 'tokenvane-sync-worker';
      const workerUrl = `https://${workerName}.preview.workers.dev`;
      const bearerToken = input?.bearer_token?.trim() || 'preview-generated-sync-token';
      const e2eeEnabled = Boolean(input?.e2ee_enabled);
      const passphrase = e2eeEnabled
        ? input?.e2ee_passphrase?.trim() || 'preview-generated-e2ee-key'
        : null;
      const params = new URLSearchParams({
        v: '1',
        url: workerUrl,
        token: bearerToken,
        e2ee: e2eeEnabled ? '1' : '0',
      });
      if (e2eeEnabled && passphrase) {
        params.set('key', passphrase);
      }
      const result: CloudSyncProvisionResult = {
        mode: input?.mode ?? 'wrangler_cli',
        worker_name: workerName,
        worker_url: workerUrl,
        bearer_token: bearerToken,
        e2ee_passphrase: passphrase,
        initial_sync_ok: true,
        status: {
          enabled: true,
          worker_url: workerUrl,
          e2ee_enabled: e2eeEnabled,
          e2ee_kdf_salt_hex: e2eeEnabled ? 'preview-salt' : null,
          has_bearer_token: true,
          has_e2ee_passphrase: Boolean(passphrase),
          has_cf_api_token: Boolean(input?.cf_api_token) || input?.mode === 'cloudflare_api',
          sse_connected: true,
          last_push_watermark_ms: now,
          last_pull_watermark_ms: now,
          last_sync_at_ms: now,
          last_sync_error: null,
          updated_at_ms: now,
        },
        pairing: {
          pairing_uri: `tokenvane://pair?${params.toString()}`,
          pairing_json: JSON.stringify({
            v: 1,
            url: workerUrl,
            token: bearerToken,
            e2ee: e2eeEnabled,
            ...(passphrase ? { key: passphrase } : {}),
          }),
          worker_url: workerUrl,
          e2ee_enabled: e2eeEnabled,
          includes_passphrase: Boolean(passphrase),
        },
      };
      return result as T;
    }
    case 'trigger_cloud_sync_now':
      return {
        pushed_hourly_deltas: 0,
        pushed_subscriptions: 0,
        pulled_items: 0,
        applied_hourly_buckets: 0,
        applied_subscriptions: 0,
        server_seq_ms: null,
        synced_at_ms: now,
      } as T;
    case 'get_remote_subscription_snapshots':
    case 'get_payg_accounts':
      return [] as T;
    case 'upsert_payg_account':
    case 'delete_payg_account':
      return undefined as T;
    case 'toggle_floating_window':
      previewFloatingVisible = !previewFloatingVisible;
      return previewFloatingVisible as T;
    case 'get_floating_window_visible':
      return previewFloatingVisible as T;
    case 'set_floating_window_compact':
      return undefined as T;
    default:
      return {} as T;
  }
}

/** Rust `IpcError { code, message }` surfaced as a real `Error` so UI handlers can read `.message`. */
export class IpcCallError extends Error {
  readonly code: string;
  readonly command: string;

  constructor(command: string, code: string, message: string) {
    super(message);
    this.name = 'IpcCallError';
    this.code = code;
    this.command = command;
  }
}

export function toIpcCallError(command: string, raw: unknown): Error {
  if (raw instanceof Error) {
    return raw;
  }
  if (typeof raw === 'string' && raw.trim()) {
    return new IpcCallError(command, 'IPC_ERROR', raw.trim());
  }
  if (raw !== null && typeof raw === 'object') {
    const record = raw as Record<string, unknown>;
    const message = typeof record.message === 'string' ? record.message.trim() : '';
    const code = typeof record.code === 'string' ? record.code : 'IPC_ERROR';
    if (message) {
      return new IpcCallError(command, code, message);
    }
    return new IpcCallError(command, code, `${command} 调用异常（${code}）`);
  }
  return new IpcCallError(command, 'IPC_ERROR', `${command} 调用异常`);
}

async function callIpc<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauriRuntime()) {
    return buildPreviewFallback<T>(command, args);
  }
  try {
    return await invoke<T>(command, args);
  } catch (raw: unknown) {
    throw toIpcCallError(command, raw);
  }
}

/** Subscribe to one-click Worker provisioning progress (no-op outside Tauri). */
export async function onCloudSyncProvisionProgress(
  handler: (step: CloudSyncProvisionStep) => void
): Promise<UnlistenFn> {
  if (!isTauriRuntime()) {
    return () => {};
  }
  return listen<CloudSyncProvisionStep>('cloud-sync-provision-progress', (event) => {
    handler(event.payload);
  });
}

export const ipc = {
  getBootstrapState: (): Promise<BootstrapState> => callIpc('get_bootstrap_state'),

  getOverview: (filter: AnalyticsFilter): Promise<OverviewReport> =>
    callIpc('get_overview', { filter }),

  getTrends: (query: TrendsQuery): Promise<TrendsReport> =>
    callIpc('get_trends', { query }),

  getMonthly: (query: MonthlyQuery): Promise<MonthlyReport> =>
    callIpc('get_monthly', { query }),

  getModelsPricing: (filter: AnalyticsFilter): Promise<ModelStatsReport> =>
    callIpc('get_models_pricing', { filter }),

  upsertModelPricingOverride: (
    entry: ModelPricingEntry,
    recomputeHistory: boolean
  ): Promise<number> =>
    callIpc('upsert_model_pricing_override', {
      entry,
      recomputeHistory,
    }),

  deleteModelPricingOverride: (
    modelPattern: string,
    recomputeHistory: boolean
  ): Promise<number> =>
    callIpc('delete_model_pricing_override', {
      modelPattern,
      recomputeHistory,
    }),

  recomputeAllCosts: (): Promise<number> => callIpc('recompute_all_costs'),

  syncPricingAndRate: (force = false): Promise<CatalogSyncResult> =>
    callIpc('sync_pricing_and_rate', { force }),

  getSubscriptions: (): Promise<SubscriptionPlanStatus[]> => callIpc('get_subscriptions'),

  upsertSubscriptionPlan: (plan: SubscriptionPlan): Promise<void> =>
    callIpc('upsert_subscription_plan', { plan }),

  setSubscriptionPlanEnabled: (planId: string, enabled: boolean): Promise<void> =>
    callIpc('set_subscription_plan_enabled', { planId, enabled }),

  deleteSubscriptionPlan: (planId: string): Promise<void> =>
    callIpc('delete_subscription_plan', { planId }),

  getSubscriptionTemplates: (): Promise<SubscriptionPlanTemplate[]> =>
    callIpc('get_subscription_templates'),

  getObservedRateLimits: (): Promise<ObservedRateLimitWindow[]> =>
    callIpc('get_observed_rate_limits'),

  getSpeedLatency: (filter: AnalyticsFilter): Promise<SpeedLatencyReport> =>
    callIpc('get_speed_latency', { filter }),

  getTraySummary: (tzOffsetMinutes: number): Promise<TraySummary> =>
    callIpc('get_tray_summary', { tzOffsetMinutes }),

  getProviderConfigs: (): Promise<ProviderConfig[]> => callIpc('get_provider_configs'),

  updateProviderConfig: (config: ProviderConfig): Promise<void> =>
    callIpc('update_provider_config', { config }),

  getIngestionCursors: (): Promise<IngestionCursor[]> => callIpc('get_ingestion_cursors'),

  triggerScanNow: (): Promise<ScanReport> => callIpc('trigger_scan_now'),

  setProviderApiKey: (providerId: string, apiKey: string): Promise<void> =>
    callIpc('set_provider_api_key', { providerId, apiKey }),

  deleteProviderApiKey: (providerId: string): Promise<void> =>
    callIpc('delete_provider_api_key', { providerId }),

  reconcileProviderOfficialApi: (
    providerId: string,
    lookbackDays: number
  ): Promise<OfficialApiReconcileResult> =>
    callIpc('reconcile_provider_official_api', { providerId, lookbackDays }),

  getPreferences: (): Promise<AppPreferences> => callIpc('get_preferences'),

  updatePreferences: (prefs: AppPreferences): Promise<AppPreferences> =>
    callIpc('update_preferences', { prefs }),

  importBackgroundImage: (sourcePath: string): Promise<string> =>
    callIpc('import_background_image', { sourcePath }),

  clearBackgroundImage: (): Promise<void> => callIpc('clear_background_image'),

  getBackgroundImageDataUrl: (): Promise<string | null> =>
    callIpc('get_background_image_data_url'),

  exportMonthlyReport: (
    query: MonthlyQuery,
    format: 'csv' | 'json',
    savePath?: string | null
  ): Promise<ExportReportResult> =>
    callIpc('export_monthly_report', {
      query,
      format,
      savePath: savePath ?? null,
    }),

  /** Writes a user-chosen PNG (share receipt) to `savePath`; returns bytes written. */
  saveShareImage: (savePath: string, bytes: number[]): Promise<number> =>
    callIpc('save_share_image', { savePath, bytes }),

  previewSyncEnvelopeV1: (sinceWatermarkMs: number): Promise<SyncEnvelopeV1> =>
    callIpc('preview_sync_envelope_v1', { sinceWatermarkMs }),

  listCustomProviderRules: (): Promise<CustomProviderRule[]> =>
    callIpc('list_custom_provider_rules'),

  upsertCustomProviderRule: (rule: CustomProviderRule): Promise<CustomProviderRule[]> =>
    callIpc('upsert_custom_provider_rule', { rule }),

  deleteCustomProviderRule: (ruleId: string): Promise<CustomProviderRule[]> =>
    callIpc('delete_custom_provider_rule', { ruleId }),

  testCustomProviderSampleLine: (
    rule: CustomProviderRule,
    sampleLine: string
  ): Promise<CustomRuleTestResult> =>
    callIpc('test_custom_provider_sample_line', { rule, sampleLine }),

  getProviderBalances: (): Promise<ProviderBalanceSnapshot[]> =>
    callIpc('get_provider_balances'),

  refreshProviderBalance: (providerId: string): Promise<BalanceRefreshOutcome> =>
    callIpc('refresh_provider_balance', { providerId }),

  launchProviderCliLogin: (providerId: string): Promise<string> =>
    callIpc('launch_provider_cli_login', { providerId }),

  getPaygAccounts: (tzOffsetMinutes?: number): Promise<PaygAccountStatus[]> =>
    callIpc('get_payg_accounts', {
      tzOffsetMinutes: tzOffsetMinutes ?? -new Date().getTimezoneOffset(),
    }),

  upsertPaygAccount: (account: PaygAccount): Promise<void> =>
    callIpc('upsert_payg_account', { account }),

  deletePaygAccount: (accountId: string): Promise<void> =>
    callIpc('delete_payg_account', { accountId }),

  listDevices: (): Promise<DeviceRecord[]> => callIpc('list_devices'),

  getSyncStatus: (): Promise<SyncStatusView> => callIpc('get_sync_status'),

  updateSyncConfig: (
    enabled: boolean,
    workerUrl: string | null,
    e2eeEnabled: boolean
  ): Promise<SyncStatusView> =>
    callIpc('update_sync_config', { enabled, workerUrl, e2eeEnabled }),

  setSyncCredentials: (
    bearerToken: string | null,
    e2eePassphrase: string | null
  ): Promise<SyncStatusView> =>
    callIpc('set_sync_credentials', { bearerToken, e2eePassphrase }),

  clearSyncCredentials: (): Promise<SyncStatusView> => callIpc('clear_sync_credentials'),

  saveCloudSyncSetup: (
    enabled: boolean,
    workerUrl: string | null,
    bearerToken: string | null,
    e2eeEnabled: boolean,
    e2eePassphrase: string | null
  ): Promise<SyncStatusView> =>
    callIpc('save_cloud_sync_setup', {
      enabled,
      workerUrl,
      bearerToken,
      e2eeEnabled,
      e2eePassphrase,
    }),

  provisionCloudSyncWorker: (
    input: CloudSyncProvisionInput
  ): Promise<CloudSyncProvisionResult> =>
    callIpc('provision_cloud_sync_worker', { input }),

  getSyncPairingBundle: (includePassphrase: boolean): Promise<SyncPairingBundle> =>
    callIpc('get_sync_pairing_bundle', { includePassphrase }),

  triggerCloudSyncNow: (): Promise<SyncRunReport> => callIpc('trigger_cloud_sync_now'),

  getRemoteSubscriptionSnapshots: (): Promise<RemoteSubscriptionSnapshot[]> =>
    callIpc('get_remote_subscription_snapshots'),

  openMainWorkspace: (page?: string, providerId?: string | null): Promise<void> =>
    callIpc('open_main_workspace', {
      page: page ?? null,
      providerId: providerId ?? null,
    }),

  quitApplication: (): Promise<void> => callIpc('quit_application'),

  toggleFloatingWindow: (): Promise<boolean> => callIpc('toggle_floating_window'),

  getFloatingWindowVisible: (): Promise<boolean> => callIpc('get_floating_window_visible'),

  setFloatingWindowCompact: (
    compact: boolean,
    width?: number,
    height?: number
  ): Promise<void> =>
    callIpc(
      'set_floating_window_compact',
      width !== undefined && height !== undefined ? { compact, width, height } : { compact }
    ),
};
