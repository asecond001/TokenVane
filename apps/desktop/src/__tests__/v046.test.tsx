import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { computeLiveSpeedBreakdown } from '../components/LiveSpeedBadge';
import {
  DEFAULT_FLOATING_MODULES,
  moveFloatingModule,
  normalizeFloatingConfig,
  toggleFloatingModule,
} from '../lib/floatingConfig';
import { ipc } from '../lib/ipc';
import { buildModelComparison, buildProviderComparison, sortComparisonRows } from '../lib/modelComparison';
import { buildReceiptModel } from '../lib/receipt';
import { FloatingMonitorPage } from '../pages/FloatingMonitorPage';
import { ModelsComparePage } from '../pages/ModelsComparePage';
import { MonthlyPage } from '../pages/MonthlyPage';
import { previousPeriodRange, percentChange } from '../pages/TrendsPage';
import { useWorkspaceStore } from '../stores/workspaceStore';
import type { ActiveSessionSummary, ModelStatRow, ModelStatsReport } from '../types/ipc';
import {
  makeMonthlyReport,
  makeOverviewReport,
  makePreferences,
  makeSpeedLatencyReport,
  makeTraySummary,
  renderWithQuery,
  resetWorkspaceState,
} from './testUtils';

function session(overrides: Partial<ActiveSessionSummary>): ActiveSessionSummary {
  return {
    session_key: '#abc',
    session_title: 'Task',
    provider_id: 'claude_code',
    model_id: 'claude-sonnet-4-5',
    project_label: 'TokenVane',
    activity_state: 'active',
    started_at_ms: 0,
    last_active_at_ms: 0,
    request_count: 1,
    total_tokens: 1_000,
    input_tokens: 800,
    output_tokens: 200,
    cache_read_tokens: 0,
    estimated_cost_micro_usd: 10_000,
    latest_output_tps: 70,
    ...overrides,
  };
}

function modelRow(overrides: Partial<ModelStatRow>): ModelStatRow {
  return {
    model_id: 'claude-opus-4-6',
    provider_id: 'claude_code',
    requests: 10,
    observable_steps: 0,
    input_tokens: 400_000,
    output_tokens: 100_000,
    cache_read_tokens: 500_000,
    cache_write_tokens: 0,
    reasoning_tokens: 0,
    total_tokens: 1_000_000,
    cache_saved_micro_usd: 1_000_000,
    estimated_cost_micro_usd: 12_000_000,
    cost_share_ratio: 0.8,
    avg_output_tps: 60,
    effective_pricing: null,
    list_cost_micro_usd: 20_000_000,
    subscription_cost_micro_usd: null,
    ...overrides,
  };
}

function modelsReport(rows: ModelStatRow[]): ModelStatsReport {
  return {
    rows,
    total_cost_micro_usd: rows.reduce((a, r) => a + r.estimated_cost_micro_usd, 0),
    total_cache_saved_micro_usd: 0,
    catalog: [],
    subscription_allocations: [],
  };
}

describe('v0.4.6 desktop fixes & features', () => {
  beforeEach(() => {
    resetWorkspaceState();
    window.localStorage.removeItem('tokenvane.floating_config.v1');
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('drops the live speed once no session produced output in the last two minutes', () => {
    const now = 1_800_000_000_000;
    const fresh = session({ last_active_at_ms: now - 30_000, latest_output_tps: 80 });
    const stale = session({ last_active_at_ms: now - 5 * 60_000, latest_output_tps: 80 });
    const recent = session({ activity_state: 'recent', last_active_at_ms: now - 10_000 });

    expect(computeLiveSpeedBreakdown([fresh], null, now).primaryTps).toBe(80);
    // Backend still says "active" but the client clock has moved on.
    expect(computeLiveSpeedBreakdown([stale], null, now).primaryTps).toBeNull();
    expect(computeLiveSpeedBreakdown([recent], null, now).primaryTps).toBeNull();
    // Stale backend live speeds are ignored too.
    const breakdown = computeLiveSpeedBreakdown(
      [stale],
      [{ provider_id: 'claude_code', model_id: 'claude-sonnet-4-5', output_tps: 99, activity_state: 'active' }],
      now
    );
    expect(breakdown.primaryTps).toBeNull();
    expect(breakdown.items).toEqual([]);
  });

  it('migrates legacy floating toggles into ordered modules and supports toggle / reorder', () => {
    const migrated = normalizeFloatingConfig({
      showTokens: true,
      showCost: false,
      showSpeed: true,
      showSession: false,
      showSubscriptions: true,
    } as never);
    expect(migrated.modules).toEqual(['tokens', 'speed', 'quotas']);

    expect(normalizeFloatingConfig(null).modules).toEqual([...DEFAULT_FLOATING_MODULES]);
    expect(normalizeFloatingConfig({ modules: [] }).modules).toEqual(['tokens']);
    expect(
      normalizeFloatingConfig({ modules: ['pulse', 'bogus', 'pulse', 'models'] as never }).modules
    ).toEqual(['pulse', 'models']);

    expect(toggleFloatingModule(['tokens'], 'tokens')).toEqual(['tokens']);
    expect(toggleFloatingModule(['tokens'], 'models')).toEqual(['tokens', 'models']);
    expect(moveFloatingModule(['tokens', 'cost', 'speed'], 'speed', -1)).toEqual([
      'tokens',
      'speed',
      'cost',
    ]);
    expect(moveFloatingModule(['tokens', 'cost'], 'tokens', -1)).toEqual(['tokens', 'cost']);
  });

  it('renders floating modules in a multi-column grid and adds panels from the customize list', async () => {
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 520 });
    Object.defineProperty(window, 'innerHeight', { configurable: true, value: 420 });
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(makePreferences({ floating_window_enabled: true }));
    vi.spyOn(ipc, 'getTraySummary').mockResolvedValue(
      makeTraySummary({
        today_requests: 321,
        provider_shares: [
          { key: 'claude_code', label: 'Claude Code', tokens: 90_000, token_share_percent: 60, cost_micro_usd: 1, cost_share_percent: 60 },
          { key: 'codex', label: 'Codex', tokens: 60_000, token_share_percent: 40, cost_micro_usd: 1, cost_share_percent: 40 },
        ],
      })
    );
    const overviewSpy = vi.spyOn(ipc, 'getOverview').mockResolvedValue(makeOverviewReport());
    vi.spyOn(ipc, 'setFloatingWindowCompact').mockResolvedValue(undefined);

    renderWithQuery(<FloatingMonitorPage />);

    expect(await screen.findByText('321')).toBeInTheDocument();
    const panels = screen.getByTestId('floating-panels');
    expect(panels.style.gridTemplateColumns).toBe('repeat(2, minmax(0, 1fr))');
    expect(within(panels).getByText('提供商占比')).toBeInTheDocument();
    expect(within(panels).getByText('今日脉搏')).toBeInTheDocument();
    await waitFor(() => expect(overviewSpy).toHaveBeenCalled());

    fireEvent.click(screen.getByRole('button', { name: '自定义悬浮窗' }));
    const customize = screen.getByTestId('floating-customize');
    fireEvent.click(within(customize).getByRole('checkbox', { name: '主力模型' }));
    fireEvent.click(within(customize).getByRole('button', { name: '上移 主力模型' }));
    fireEvent.click(screen.getByRole('button', { name: '自定义悬浮窗' }));

    const stored = JSON.parse(window.localStorage.getItem('tokenvane.floating_config.v1') ?? '{}') as {
      modules: string[];
    };
    expect(stored.modules.slice(-2)).toEqual(['models', 'providers']);
    expect(within(screen.getByTestId('floating-panels')).getByText('主力模型')).toBeInTheDocument();
  });

  it('joins price and speed per model / provider and sorts with empty values last', () => {
    const models = modelsReport([
      modelRow({}),
      modelRow({
        model_id: 'gpt-5.3-codex',
        provider_id: 'codex',
        total_tokens: 2_000_000,
        input_tokens: 1_000_000,
        cache_read_tokens: 1_000_000,
        estimated_cost_micro_usd: 4_000_000,
      }),
    ]);
    const speed = makeSpeedLatencyReport();
    const rows = buildModelComparison(models, speed);
    expect(rows.map((r) => r.model_id)).toEqual(['claude-opus-4-6', 'gpt-5.3-codex']);
    expect(rows[0].cost_per_m_micro_usd).toBe(12_000_000);
    expect(rows[0].mean_tps).toBe(64.2);
    expect(rows[1].cost_per_m_micro_usd).toBe(2_000_000);
    expect(rows[1].avg_ttft_ms).toBe(420);

    const cheapestFirst = sortComparisonRows(rows, 'cost_per_m_micro_usd', false);
    expect(cheapestFirst[0].model_id).toBe('gpt-5.3-codex');

    const providers = buildProviderComparison(rows, speed, models);
    expect(providers.map((p) => p.provider_id)).toEqual(['claude_code', 'codex']);
    expect(providers[0].cost_share).toBeCloseTo(0.75);
    expect(providers[1].cache_hit_ratio).toBeCloseTo(0.5);
    expect(providers[1].mean_tps).toBe(108.4);
  });

  it('merges models and speed into the 模型对比 page with a comparison view first', async () => {
    vi.spyOn(ipc, 'getModelsPricing').mockResolvedValue(
      modelsReport([modelRow({}), modelRow({ model_id: 'gpt-5.3-codex', provider_id: 'codex', estimated_cost_micro_usd: 3_000_000 })])
    );
    vi.spyOn(ipc, 'getSpeedLatency').mockResolvedValue(makeSpeedLatencyReport());
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(makePreferences());

    renderWithQuery(<ModelsComparePage />);

    expect(screen.getByRole('heading', { name: '模型对比' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: '综合对比' })).toHaveAttribute('aria-selected', 'true');
    const providerTable = await screen.findByTestId('provider-compare-table');
    expect(within(providerTable).getByText('Claude Code')).toBeInTheDocument();
    expect(within(providerTable).getByText('108.4 tok/s')).toBeInTheDocument();
    expect(screen.queryByTestId('models-value-scatter')).not.toBeInTheDocument();

    const modelTable = screen.getByTestId('model-compare-table');
    // The per-1M column follows the selected price basis.
    expect(screen.getByRole('radio', { name: 'API 实付' })).toHaveAttribute('aria-checked', 'true');
    fireEvent.click(screen.getByRole('radio', { name: 'API 标价' }));
    expect(screen.getByRole('radio', { name: 'API 标价' })).toHaveAttribute('aria-checked', 'true');
    fireEvent.click(within(modelTable).getByRole('columnheader', { name: /平均速率/ }));
    const firstModel = within(modelTable).getAllByRole('row')[1];
    expect(within(firstModel).getByText('gpt-5.3-codex')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('tab', { name: '速度' }));
    expect(await screen.findByText('模型提供商维度速度与延迟总览')).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: '速度监测' })).not.toBeInTheDocument();
  });

  it('drives the 统计 page from the shared time range control', async () => {
    const monthlySpy = vi.spyOn(ipc, 'getMonthly').mockResolvedValue(makeMonthlyReport());
    useWorkspaceStore.getState().setCustomDateRange('2026-10-01', '2026-10-15');

    renderWithQuery(<MonthlyPage />);

    expect(await screen.findByRole('heading', { name: '统计' })).toBeInTheDocument();
    expect(screen.getByTestId('time-range-control')).toBeInTheDocument();
    expect(screen.getByTestId('stats-date-span')).toHaveTextContent('2026-10-01 ~ 2026-10-15');
    await waitFor(() => expect(monthlySpy).toHaveBeenCalled());
    const query = monthlySpy.mock.calls[0][0];
    expect(query.start_ms).toBe(new Date(2026, 9, 1).getTime());
    expect(query.end_ms).toBe(new Date(2026, 9, 15, 23, 59, 59, 999).getTime());
    expect(query.year).toBe(2026);
    expect(query.month).toBe(10);
  });

  it('computes the previous comparison window and prints the range on the share receipt', () => {
    const start = 1_000_000;
    expect(previousPeriodRange(start, start + 999, start + 10_000)).toEqual({
      start_ms: start - 1_000,
      end_ms: start - 1,
    });
    // A window running into the future is clamped to now before mirroring.
    expect(previousPeriodRange(start, start + 99_999, start + 499)).toEqual({
      start_ms: start - 500,
      end_ms: start - 1,
    });
    expect(percentChange(150, 100)).toBe(50);
    expect(percentChange(10, 0)).toBeNull();

    const model = buildReceiptModel(makeMonthlyReport(), 'USD', '7.25', new Date(), '2026-10-01 ~ 2026-10-15');
    expect(model.lines).toContainEqual({ kind: 'subtitle', text: 'AI 用量小票 · 2026-10-01 ~ 2026-10-15' });
    expect(model.fileName).toBe('tokenvane-receipt-2026-10-01_2026-10-15.png');
  });
});
