import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render } from '@testing-library/react';
import { useWorkspaceStore } from '../stores/workspaceStore';
import type {
  AppPreferences,
  BootstrapState,
  MonthlyReport,
  OverviewReport,
  SpeedLatencyReport,
  SubscriptionPlanStatus,
  SubscriptionPlanTemplate,
  TraySummary,
  TrendsReport,
} from '../types/ipc';

export interface EChartMockOption {
  series?: Array<{
    type?: string;
    stack?: string;
    name?: unknown;
    lineStyle?: { width?: number; opacity?: number };
    itemStyle?: { color?: string; borderWidth?: number; borderColor?: string };
    areaStyle?: { opacity?: number };
    data?: unknown;
  }>;
  visualMap?: { type?: string; dimension?: number; pieces?: Array<{ value: number; color: string }> };
}

export const originalMatchMedia = window.matchMedia;

export function installMatchMediaStub(lightMatches: boolean) {
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: (query: string) => ({
      matches: query === '(prefers-color-scheme: light)' ? lightMatches : false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }),
  });
}

export function renderWithQuery(ui: React.ReactElement) {
  const client = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
      },
    },
  });
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

export function makePreferences(overrides: Partial<AppPreferences> = {}): AppPreferences {
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
    floating_window_enabled: false,
    background_cli_quota_poll: true,
    last_rate_sync_at_ms: null,
    last_pricing_sync_at_ms: null,
    last_subscription_sync_at_ms: null,
    updated_at_ms: 1_770_000_000_000,
    ...overrides,
  };
}

export function makeBootstrap(prefs: AppPreferences = makePreferences()): BootstrapState {
  return {
    db_path: 'C:\\Users\\dev\\AppData\\Local\\TokenVane\\tokenvane.db',
    device: {
      device_id: 'dev-test-1',
      device_name: 'win-workstation',
      platform: 'windows',
      created_at_ms: 1_770_000_000_000,
    },
    preferences: prefs,
    providers: [],
    collector_watching: true,
    otlp_running: false,
    otlp_port: null,
    last_scan: null,
  };
}

export function makeTraySummary(overrides: Partial<TraySummary> = {}): TraySummary {
  return {
    today_tokens: 150_000,
    today_cost_micro_usd: 2_500_000,
    today_requests: 12,
    latest_output_tps: 68.4,
    subscriptions: [],
    provider_shares: [],
    active_sessions: [],
    currency: 'USD',
    usd_to_cny_rate: '7.25',
    active_providers_count: 2,
    total_ingested_events: 48,
    ...overrides,
  };
}

export function makeOverviewReport(overrides: Partial<OverviewReport> = {}): OverviewReport {
  const bucketTs = 1_770_000_000_000;
  return {
    total_tokens: 420_000,
    total_cost_micro_usd: 6_450_000,
    today_tokens: 150_000,
    today_cost_micro_usd: 2_500_000,
    today_requests: 12,
    today_cache_hit_ratio: 0.62,
    today_cache_saved_micro_usd: 450_000,
    today_tokens_delta_pct: 14.5,
    today_cost_delta_pct: 9.8,
    today_hourly_pulse: [
      {
        bucket_start_ms: bucketTs,
        input_tokens: 40_000,
        output_tokens: 20_000,
        cache_read_tokens: 80_000,
        cache_write_tokens: 10_000,
        reasoning_tokens: 0,
        cost_micro_usd: 2_500_000,
        requests: 12,
      },
    ],
    cache_hit_ratio: 0.64,
    avg_output_tps: 74.2,
    peak_output_tps: 142.0,
    latest_output_tps: 88.1,
    total_requests: 35,
    observable_steps: 90,
    tokens_delta_pct: 18.4,
    cost_delta_pct: 12.5,
    cache_delta_pct: 4.2,
    tps_delta_pct: -2.1,
    series_granularity: 'daily',
    series: [
      {
        bucket_start_ms: bucketTs,
        input_tokens: 120_000,
        output_tokens: 60_000,
        cache_read_tokens: 200_000,
        cache_write_tokens: 40_000,
        reasoning_tokens: 0,
        cost_micro_usd: 6_450_000,
        requests: 35,
      },
    ],
    provider_series: [
      {
        key: 'claude_code',
        label: 'Claude Code',
        total_tokens: 270_000,
        total_cost_micro_usd: 4_200_000,
        points: [
          {
            bucket_start_ms: bucketTs,
            tokens: 270_000,
            cost_micro_usd: 4_200_000,
            requests: 20,
          },
        ],
      },
      {
        key: 'codex',
        label: 'Codex',
        total_tokens: 150_000,
        total_cost_micro_usd: 2_250_000,
        points: [
          {
            bucket_start_ms: bucketTs,
            tokens: 150_000,
            cost_micro_usd: 2_250_000,
            requests: 15,
          },
        ],
      },
    ],
    providers: [
      {
        provider_id: 'claude_code',
        enabled: true,
        period_tokens: 270_000,
        period_cost_micro_usd: 4_200_000,
        today_tokens: 90_000,
        today_cost_micro_usd: 1_400_000,
        month_tokens: 270_000,
        month_cost_micro_usd: 4_200_000,
        cache_hit_ratio: 0.71,
        observable_steps: 55,
        active_window_label: null,
        active_window_used_percent: null,
        last_sync_at_ms: bucketTs,
        last_error: null,
      },
      {
        provider_id: 'codex',
        enabled: true,
        period_tokens: 150_000,
        period_cost_micro_usd: 2_250_000,
        today_tokens: 60_000,
        today_cost_micro_usd: 1_100_000,
        month_tokens: 150_000,
        month_cost_micro_usd: 2_250_000,
        cache_hit_ratio: 0.52,
        observable_steps: 35,
        active_window_label: null,
        active_window_used_percent: null,
        last_sync_at_ms: bucketTs,
        last_error: null,
      },
    ],
    provider_shares: [],
    top_model_shares: [],
    active_sessions: [],
    ...overrides,
  };
}

export function makeTrendsReport(overrides: Partial<TrendsReport> = {}): TrendsReport {
  const b1 = 1_769_904_000_000;
  const b2 = 1_769_990_400_000;
  return {
    buckets: [b1, b2],
    effective_granularity: 'daily',
    series: [
      {
        key: 'claude_code',
        label: 'Claude Code',
        total_tokens: 310_000,
        total_cost_micro_usd: 4_800_000,
        points: [
          {
            bucket_start_ms: b1,
            tokens: 140_000,
            cost_micro_usd: 2_100_000,
            requests: 11,
            cache_hit_ratio: 0.58,
          },
          {
            bucket_start_ms: b2,
            tokens: 170_000,
            cost_micro_usd: 2_700_000,
            requests: 14,
            cache_hit_ratio: 0.68,
          },
        ],
      },
      {
        key: 'antigravity',
        label: 'Antigravity',
        total_tokens: 180_000,
        total_cost_micro_usd: 2_100_000,
        points: [
          {
            bucket_start_ms: b1,
            tokens: 80_000,
            cost_micro_usd: 900_000,
            requests: 6,
            cache_hit_ratio: 0.42,
          },
          {
            bucket_start_ms: b2,
            tokens: 100_000,
            cost_micro_usd: 1_200_000,
            requests: 8,
            cache_hit_ratio: 0.51,
          },
        ],
      },
    ],
    comparisons: [
      {
        key: 'claude_code',
        label: 'Claude Code',
        current_tokens: 310_000,
        previous_tokens: 200_000,
        delta_pct: 55.0,
        current_cost_micro_usd: 4_800_000,
        previous_cost_micro_usd: 3_000_000,
        cost_delta_pct: 60.0,
        cache_hit_ratio: 0.64,
        requests: 25,
      },
      {
        key: 'antigravity',
        label: 'Antigravity',
        current_tokens: 180_000,
        previous_tokens: 150_000,
        delta_pct: 20.0,
        current_cost_micro_usd: 2_100_000,
        previous_cost_micro_usd: 1_700_000,
        cost_delta_pct: 23.5,
        cache_hit_ratio: 0.47,
        requests: 14,
      },
    ],
    ...overrides,
  };
}

export function makeMonthlyReport(overrides: Partial<MonthlyReport> = {}): MonthlyReport {
  return {
    year: 2026,
    month: 11,
    api_equivalent_cost_micro_usd: 30_000_000,
    subscription_monthly_fee_micro_usd: 0,
    roi_multiplier: null,
    month_total_tokens: 1_850_000,
    month_total_requests: 142,
    month_cache_saved_micro_usd: 12_400_000,
    provider_shares: [
      {
        key: 'claude_code',
        label: 'Claude Code',
        tokens: 1_250_000,
        token_share_percent: 67.6,
        cost_micro_usd: 21_000_000,
        cost_share_percent: 70.0,
      },
      {
        key: 'codex',
        label: 'Codex',
        tokens: 600_000,
        token_share_percent: 32.4,
        cost_micro_usd: 9_000_000,
        cost_share_percent: 30.0,
      },
    ],
    model_shares: [
      {
        key: 'claude-opus-4-6',
        label: 'claude-opus-4-6',
        tokens: 1_250_000,
        token_share_percent: 67.6,
        cost_micro_usd: 21_000_000,
        cost_share_percent: 70.0,
      },
    ],
    daily_ledger: [
      {
        date: '2026-11-15',
        requests: 32,
        observable_steps: 78,
        input_tokens: 90_000,
        output_tokens: 45_000,
        cache_read_tokens: 320_000,
        cache_write_tokens: 25_000,
        reasoning_tokens: 12_000,
        total_tokens: 492_000,
        cost_micro_usd: 8_200_000,
        cache_saved_micro_usd: 4_100_000,
        top_model: 'claude-opus-4-6',
      },
    ],
    heatmap_365d: [
      {
        date: '2026-11-15',
        total_tokens: 492_000,
        cost_micro_usd: 8_200_000,
        requests: 32,
        top_model: 'claude-opus-4-6',
      },
    ],
    ...overrides,
  };
}

export function makeSpeedLatencyReport(
  overrides: Partial<SpeedLatencyReport> = {}
): SpeedLatencyReport {
  return {
    providers: [
      {
        provider_id: 'claude_code',
        sample_count: 18,
        mean_tps: 64.2,
        p50_tps: 62.0,
        p95_tps: 88.5,
        max_tps: 94.0,
        min_tps: 38.0,
        avg_duration_ms: 4_200,
        avg_ttft_ms: 680,
      },
      {
        provider_id: 'codex',
        sample_count: 9,
        mean_tps: 108.4,
        p50_tps: 105.0,
        p95_tps: 132.0,
        max_tps: 140.0,
        min_tps: 78.0,
        avg_duration_ms: 2_100,
        avg_ttft_ms: 420,
      },
    ],
    models: [
      {
        model_id: 'claude-opus-4-6',
        provider_id: 'claude_code',
        sample_count: 18,
        mean_tps: 64.2,
        p50_tps: 62.0,
        p95_tps: 88.5,
        max_tps: 94.0,
        min_tps: 38.0,
        latest_tps: 67.8,
        avg_duration_ms: 4_200,
        avg_ttft_ms: 680,
      },
      {
        model_id: 'gpt-5.3-codex',
        provider_id: 'codex',
        sample_count: 9,
        mean_tps: 108.4,
        p50_tps: 105.0,
        p95_tps: 132.0,
        max_tps: 140.0,
        min_tps: 78.0,
        latest_tps: 112.5,
        avg_duration_ms: 2_100,
        avg_ttft_ms: 420,
      },
    ],
    ...overrides,
  };
}

export function makePlanStatus(overrides: Partial<SubscriptionPlanStatus> = {}): SubscriptionPlanStatus {
  const now = 1_770_000_000_000;
  return {
    plan: {
      plan_id: 'claude_max_5x',
      provider_id: 'claude_code',
      plan_name: 'Claude Max 5x',
      enabled: true,
      monthly_fee_micro_usd: 100_000_000,
      is_user_configured: true,
      updated_at_ms: now,
      windows: [
        {
          window_id: '5h',
          label: '5h 滚动窗口',
          duration_hours: 5,
          rolling: true,
          unit: 'micro_usd',
          limit_value: 75_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
      ],
    },
    windows: [
      {
        window_id: '5h',
        label: '5h 滚动窗口',
        duration_hours: 5,
        rolling: true,
        unit: 'micro_usd',
        used_value: 25_000_000,
        limit_value: 75_000_000,
        used_percent: 33.3,
        warning_percent: 80,
        critical_percent: 95,
        burn_rate_per_hour: 5_000_000,
        projected_exhaustion_hours: 10.0,
        resets_at_ms: now + 7200_000,
        resets_in_seconds: 7200,
        source: 'local_estimate',
        alert_level: 'normal',
      },
    ],
    today_cost_micro_usd: 4_000_000,
    week_cost_micro_usd: 22_000_000,
    month_cost_micro_usd: 85_000_000,
    month_tokens: 1_200_000,
    ...overrides,
  };
}

export function makeTemplates(): SubscriptionPlanTemplate[] {
  return [
    {
      template_id: 'claude_max_5x',
      provider_id: 'claude_code',
      plan_name: 'Claude Max 5x ($100/mo)',
      monthly_fee_micro_usd: 100_000_000,
      description: 'Anthropic Claude Max 5x 高配额订阅',
      windows: [
        {
          window_id: '5h',
          label: '5h 滚动窗口',
          duration_hours: 5,
          rolling: true,
          unit: 'micro_usd',
          limit_value: 75_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
      ],
    },
    {
      template_id: 'google_ai_ultra',
      provider_id: 'antigravity',
      plan_name: 'Google AI Ultra ($249.99/mo)',
      monthly_fee_micro_usd: 249_990_000,
      description: 'Google AI Ultra Antigravity 高配额订阅',
      windows: [
        {
          window_id: '5h',
          label: '5h 参考窗口',
          duration_hours: 5,
          rolling: true,
          unit: 'micro_usd',
          limit_value: 200_000_000,
          warning_percent: 80,
          critical_percent: 95,
        },
      ],
    },
  ];
}

export function resetWorkspaceState() {
  useWorkspaceStore.setState({
    activePage: 'overview',
    timePreset: '7d',
    customPrecise: false,
    customEndMs: null,
    providerFilter: 'all',
    deviceFilter: 'all',
    currency: 'USD',
    usdToCnyRate: '7.25',
    theme: 'dark',
    resolvedTheme: 'dark',
  });
  document.documentElement.dataset.theme = 'dark';
}
