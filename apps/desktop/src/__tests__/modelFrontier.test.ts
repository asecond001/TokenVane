import { describe, expect, it } from 'vitest';
import {
  buildModelComparison,
  buildProviderComparison,
  priceForBasis,
  type ModelComparisonRow,
} from '../lib/modelComparison';
import type { ModelSpeedStats, ModelStatRow, ModelStatsReport, SpeedLatencyReport } from '../types/ipc';

function statRow(overrides: Partial<ModelStatRow>): ModelStatRow {
  return {
    model_id: 'claude-opus-5-5',
    provider_id: 'claude_code',
    requests: 10,
    observable_steps: 0,
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    cache_write_tokens: 0,
    reasoning_tokens: 0,
    total_tokens: 0,
    cache_saved_micro_usd: 0,
    estimated_cost_micro_usd: 0,
    cost_share_ratio: 0,
    avg_output_tps: 0,
    effective_pricing: null,
    list_cost_micro_usd: 0,
    subscription_cost_micro_usd: null,
    ...overrides,
  };
}

function report(rows: ModelStatRow[]): ModelStatsReport {
  return {
    rows,
    total_cost_micro_usd: rows.reduce((a, r) => a + r.estimated_cost_micro_usd, 0),
    total_cache_saved_micro_usd: 0,
    catalog: [],
    subscription_allocations: [],
  };
}

describe('model comparison data', () => {
  it('counts cache writes as prompt tokens in the cache hit ratio', () => {
    // Claude Code shape: tiny uncached input, cache writes reported separately.
    const models = report([
      statRow({
        input_tokens: 100,
        cache_read_tokens: 980_000,
        cache_write_tokens: 19_900,
        output_tokens: 5_000,
        total_tokens: 1_005_000,
        estimated_cost_micro_usd: 1_000_000,
      }),
    ]);
    const rows = buildModelComparison(models, undefined);
    expect(rows[0].cache_hit_ratio).toBeCloseTo(0.98, 4);
    const providers = buildProviderComparison(rows, undefined, models);
    expect(providers[0].cache_hit_ratio).toBeCloseTo(0.98, 4);
  });

  it('prices rows per basis and falls back to API 实付 without a paid plan', () => {
    const [withPlan, withoutPlan] = buildModelComparison(
      report([
        statRow({
          total_tokens: 2_000_000,
          estimated_cost_micro_usd: 4_000_000,
          list_cost_micro_usd: 10_000_000,
          subscription_cost_micro_usd: 1_000_000,
        }),
        statRow({
          model_id: 'k3.1-pro',
          provider_id: 'kimi_code',
          total_tokens: 1_000_000,
          estimated_cost_micro_usd: 1_000_000,
          list_cost_micro_usd: 3_000_000,
        }),
      ]),
      undefined
    );
    expect(priceForBasis(withPlan, 'list')).toEqual({ per_m_micro_usd: 5_000_000, basis: 'list' });
    expect(priceForBasis(withPlan, 'actual')).toEqual({ per_m_micro_usd: 2_000_000, basis: 'actual' });
    expect(priceForBasis(withPlan, 'subscription')).toEqual({
      per_m_micro_usd: 500_000,
      basis: 'subscription',
    });
    expect(priceForBasis(withoutPlan, 'subscription')).toEqual({
      per_m_micro_usd: 1_000_000,
      basis: 'actual',
    });
    const empty = { ...withoutPlan, total_tokens: 0 } as ModelComparisonRow;
    expect(priceForBasis(empty, 'actual')).toBeNull();
  });

  it('carries the speed caliber of each model', () => {
    const speedRow = (overrides: Partial<ModelSpeedStats>): ModelSpeedStats => ({
      model_id: 'gpt-6-astra',
      provider_id: 'codex',
      sample_count: 120,
      mean_tps: 42,
      p50_tps: 40,
      p95_tps: 70,
      max_tps: 90,
      min_tps: 12,
      latest_tps: 41,
      avg_duration_ms: 9_000,
      avg_ttft_ms: 3_000,
      ...overrides,
    });
    const speed: SpeedLatencyReport = {
      providers: [],
      models: [
        speedRow({ speed_basis: 'decode' }),
        speedRow({ model_id: 'gemini-pro', provider_id: 'antigravity', speed_basis: 'end_to_end' }),
        speedRow({ model_id: 'legacy', provider_id: 'claude_code' }),
      ],
    };
    const rows = buildModelComparison(report([]), speed);
    const basisByModel = Object.fromEntries(rows.map((r) => [r.model_id, r.speed_basis]));
    expect(basisByModel).toEqual({ 'gpt-6-astra': 'decode', 'gemini-pro': 'end_to_end', legacy: null });
  });
});
