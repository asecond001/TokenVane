import type {
  ModelStatRow,
  ModelStatsReport,
  SpeedBasis,
  SpeedLatencyReport,
} from '../types/ipc';

/** One model (per provider) with price and speed joined side by side. */
export interface ModelComparisonRow {
  key: string;
  model_id: string;
  provider_id: string;
  requests: number;
  total_tokens: number;
  output_tokens: number;
  cost_micro_usd: number;
  cache_saved_micro_usd: number;
  /** Blended API-equivalent cost per 1M tokens, in micro-USD. `null` without tokens. */
  cost_per_m_micro_usd: number | null;
  /** API list-price cost with prompt caching ignored (all prompt tokens at the input rate). */
  list_cost_micro_usd: number;
  /** Prorated subscription fee allocated to this model; `null` without a paid plan. */
  subscription_cost_micro_usd: number | null;
  /** List price per 1M output tokens from the effective pricing entry. */
  output_price_per_m_micro_usd: number | null;
  cache_hit_ratio: number | null;
  mean_tps: number | null;
  p95_tps: number | null;
  avg_ttft_ms: number | null;
  avg_duration_ms: number | null;
  speed_samples: number;
  /** Caliber of `mean_tps` / `p95_tps`; `null` when the speed report has no row for the model. */
  speed_basis: SpeedBasis | null;
}

/** Provider-level roll-up of the same metrics. */
export interface ProviderComparisonRow {
  provider_id: string;
  model_count: number;
  requests: number;
  total_tokens: number;
  cost_micro_usd: number;
  cost_share: number;
  cache_saved_micro_usd: number;
  cost_per_m_micro_usd: number | null;
  cache_hit_ratio: number | null;
  mean_tps: number | null;
  p95_tps: number | null;
  avg_ttft_ms: number | null;
  speed_samples: number;
}

function costPerMillion(costMicro: number, tokens: number): number | null {
  return tokens > 0 ? (costMicro / tokens) * 1_000_000 : null;
}

/**
 * Share of prompt tokens served from cache. Cache writes count as prompt tokens:
 * Anthropic reports `input_tokens` as the uncached remainder only, so leaving
 * `cache_creation_input_tokens` out pins Claude Code at ~100%.
 */
function cacheHitRatio(
  row: Pick<ModelStatRow, 'input_tokens' | 'cache_read_tokens' | 'cache_write_tokens'>
): number | null {
  const denom = row.input_tokens + row.cache_read_tokens + row.cache_write_tokens;
  return denom > 0 ? row.cache_read_tokens / denom : null;
}

function positiveOrNull(v: number | null | undefined): number | null {
  return typeof v === 'number' && Number.isFinite(v) && v > 0 ? v : null;
}

export function buildModelComparison(
  models: ModelStatsReport | undefined,
  speed: SpeedLatencyReport | undefined
): ModelComparisonRow[] {
  const speedByKey = new Map<string, SpeedLatencyReport['models'][number]>(
    (speed?.models ?? []).map((m) => [`${m.provider_id}::${m.model_id}`, m] as const)
  );
  const rows = new Map<string, ModelComparisonRow>();

  for (const m of models?.rows ?? []) {
    const key = `${m.provider_id}::${m.model_id}`;
    const s = speedByKey.get(key);
    rows.set(key, {
      key,
      model_id: m.model_id,
      provider_id: m.provider_id,
      requests: m.requests,
      total_tokens: m.total_tokens,
      output_tokens: m.output_tokens + m.reasoning_tokens,
      cost_micro_usd: m.estimated_cost_micro_usd,
      cache_saved_micro_usd: m.cache_saved_micro_usd,
      cost_per_m_micro_usd: costPerMillion(m.estimated_cost_micro_usd, m.total_tokens),
      list_cost_micro_usd: m.list_cost_micro_usd,
      subscription_cost_micro_usd: m.subscription_cost_micro_usd,
      output_price_per_m_micro_usd: positiveOrNull(m.effective_pricing?.output_per_m_micro_usd),
      cache_hit_ratio: cacheHitRatio(m),
      mean_tps: positiveOrNull(s?.mean_tps ?? m.avg_output_tps),
      p95_tps: positiveOrNull(s?.p95_tps),
      avg_ttft_ms: positiveOrNull(s?.avg_ttft_ms),
      avg_duration_ms: positiveOrNull(s?.avg_duration_ms),
      speed_samples: s?.sample_count ?? 0,
      speed_basis: s?.speed_basis ?? null,
    });
  }

  // Models with speed samples but no billable rows in range still get a row.
  for (const [key, s] of speedByKey) {
    if (rows.has(key)) continue;
    rows.set(key, {
      key,
      model_id: s.model_id,
      provider_id: s.provider_id,
      requests: 0,
      total_tokens: 0,
      output_tokens: 0,
      cost_micro_usd: 0,
      cache_saved_micro_usd: 0,
      cost_per_m_micro_usd: null,
      list_cost_micro_usd: 0,
      subscription_cost_micro_usd: null,
      output_price_per_m_micro_usd: null,
      cache_hit_ratio: null,
      mean_tps: positiveOrNull(s.mean_tps),
      p95_tps: positiveOrNull(s.p95_tps),
      avg_ttft_ms: positiveOrNull(s.avg_ttft_ms),
      avg_duration_ms: positiveOrNull(s.avg_duration_ms),
      speed_samples: s.sample_count,
      speed_basis: s.speed_basis ?? null,
    });
  }

  return [...rows.values()].sort(
    (a, b) => b.cost_micro_usd - a.cost_micro_usd || b.total_tokens - a.total_tokens
  );
}

export function buildProviderComparison(
  rows: ModelComparisonRow[],
  speed: SpeedLatencyReport | undefined,
  models: ModelStatsReport | undefined
): ProviderComparisonRow[] {
  const speedByProvider = new Map((speed?.providers ?? []).map((p) => [p.provider_id, p] as const));
  const totalCost = rows.reduce((acc, r) => acc + r.cost_micro_usd, 0);
  const cacheByProvider = new Map<
    string,
    { input_tokens: number; cache_read_tokens: number; cache_write_tokens: number }
  >();
  for (const m of models?.rows ?? []) {
    const e = cacheByProvider.get(m.provider_id) ?? {
      input_tokens: 0,
      cache_read_tokens: 0,
      cache_write_tokens: 0,
    };
    e.input_tokens += m.input_tokens;
    e.cache_read_tokens += m.cache_read_tokens;
    e.cache_write_tokens += m.cache_write_tokens;
    cacheByProvider.set(m.provider_id, e);
  }

  const grouped = new Map<string, ProviderComparisonRow>();
  for (const r of rows) {
    const g = grouped.get(r.provider_id) ?? {
      provider_id: r.provider_id,
      model_count: 0,
      requests: 0,
      total_tokens: 0,
      cost_micro_usd: 0,
      cost_share: 0,
      cache_saved_micro_usd: 0,
      cost_per_m_micro_usd: null,
      cache_hit_ratio: null,
      mean_tps: null,
      p95_tps: null,
      avg_ttft_ms: null,
      speed_samples: 0,
    };
    g.model_count += 1;
    g.requests += r.requests;
    g.total_tokens += r.total_tokens;
    g.cost_micro_usd += r.cost_micro_usd;
    g.cache_saved_micro_usd += r.cache_saved_micro_usd;
    grouped.set(r.provider_id, g);
  }

  for (const g of grouped.values()) {
    const s = speedByProvider.get(g.provider_id);
    const c = cacheByProvider.get(g.provider_id);
    g.cost_share = totalCost > 0 ? g.cost_micro_usd / totalCost : 0;
    g.cost_per_m_micro_usd = costPerMillion(g.cost_micro_usd, g.total_tokens);
    g.cache_hit_ratio = c ? cacheHitRatio(c) : null;
    g.mean_tps = positiveOrNull(s?.mean_tps);
    g.p95_tps = positiveOrNull(s?.p95_tps);
    g.avg_ttft_ms = positiveOrNull(s?.avg_ttft_ms);
    g.speed_samples = s?.sample_count ?? 0;
  }

  return [...grouped.values()].sort(
    (a, b) => b.cost_micro_usd - a.cost_micro_usd || b.total_tokens - a.total_tokens
  );
}

export type ComparisonSortKey =
  | 'cost_micro_usd'
  | 'total_tokens'
  | 'cost_per_m_micro_usd'
  | 'mean_tps'
  | 'avg_ttft_ms'
  | 'cache_hit_ratio';

/** Sorts with `null` values always last, regardless of direction. */
export function sortComparisonRows<T extends Record<ComparisonSortKey, number | null>>(
  rows: T[],
  key: ComparisonSortKey,
  desc: boolean
): T[] {
  return [...rows].sort((a, b) => {
    const av = a[key];
    const bv = b[key];
    if (av === null && bv === null) return 0;
    if (av === null) return 1;
    if (bv === null) return -1;
    return desc ? bv - av : av - bv;
  });
}

/** Price basis for the price × speed chart. */
export type PriceBasis = 'list' | 'actual' | 'subscription';

export const PRICE_BASIS_OPTIONS: Array<{ id: PriceBasis; label: string }> = [
  { id: 'list', label: 'API 标价' },
  { id: 'actual', label: 'API 实付' },
  { id: 'subscription', label: '订阅摊销' },
];

/** Minimum speed samples before a model takes part in the frontier. */
export const MIN_FRONTIER_SPEED_SAMPLES = 20;

/**
 * Whether a point's speed is settled enough to take part in the frontier. Every speed
 * caliber counts, end-to-end included: it is the speed the user actually experiences.
 */
export function isFrontierEligible(point: { samples: number }): boolean {
  return point.samples >= MIN_FRONTIER_SPEED_SAMPLES;
}

export interface BasisPrice {
  /** Cost per 1M tokens in micro-USD under the requested basis. */
  per_m_micro_usd: number;
  /** Basis actually used (subscription falls back to `actual` without a paid plan). */
  basis: PriceBasis;
}

/** Cost per 1M tokens under `basis`; `null` when the row has no tokens or no price. */
export function priceForBasis(row: ModelComparisonRow, basis: PriceBasis): BasisPrice | null {
  if (row.total_tokens <= 0) return null;
  let cost: number;
  let used: PriceBasis = basis;
  if (basis === 'list') {
    cost = row.list_cost_micro_usd;
  } else if (basis === 'subscription' && row.subscription_cost_micro_usd !== null) {
    cost = row.subscription_cost_micro_usd;
  } else {
    cost = row.cost_micro_usd;
    used = 'actual';
  }
  if (!(cost > 0)) return null;
  return { per_m_micro_usd: (cost / row.total_tokens) * 1_000_000, basis: used };
}

export interface FrontierPoint {
  key: string;
  price: number;
  tps: number;
}

export interface FrontierResult {
  /** Keys on the Pareto frontier, cheapest first. */
  frontier: string[];
  /** Dominated key → the frontier key that beats it on both price and speed. */
  dominatedBy: Map<string, string>;
}

/**
 * Pareto frontier for "cheaper is better, faster is better": a point is
 * dominated when another is no more expensive and no slower, and strictly
 * better on at least one axis.
 */
export function computeParetoFrontier(points: FrontierPoint[]): FrontierResult {
  const sorted = [...points].sort((a, b) => a.price - b.price || b.tps - a.tps);
  const frontier: FrontierPoint[] = [];
  let bestTps = -Infinity;
  for (const p of sorted) {
    const last = frontier[frontier.length - 1];
    const tiesLast = last !== undefined && last.price === p.price && last.tps === p.tps;
    if (p.tps > bestTps || tiesLast) {
      frontier.push(p);
      bestTps = p.tps;
    }
  }
  const frontierKeys = new Set(frontier.map((p) => p.key));
  const dominatedBy = new Map<string, string>();
  for (const p of points) {
    if (frontierKeys.has(p.key)) continue;
    // Cheapest frontier point that is at least as cheap and at least as fast.
    const by = frontier.find((f) => f.price <= p.price && f.tps >= p.tps);
    if (by) dominatedBy.set(p.key, by.key);
  }
  return { frontier: frontier.map((p) => p.key), dominatedBy };
}
