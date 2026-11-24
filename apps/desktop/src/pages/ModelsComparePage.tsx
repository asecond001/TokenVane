import { useMemo, useState } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { InlineProgressBar } from '../components/InlineProgressBar';
import { TimeRangeControl } from '../components/TimeRangeControl';
import {
  formatCost,
  formatDurationMs,
  formatPercent,
  formatTokens,
  formatTps,
  providerAccentColor,
  providerDisplayName,
} from '../lib/format';
import { ipc } from '../lib/ipc';
import { CLOCK_REFRESH_MS } from '../lib/liveRefresh';
import {
  buildModelComparison,
  buildProviderComparison,
  PRICE_BASIS_OPTIONS,
  priceForBasis,
  sortComparisonRows,
  type ComparisonSortKey,
  type PriceBasis,
} from '../lib/modelComparison';
import {
  buildCustomRangeKey,
  computeTimeRange,
  formatResolvedDateSpan,
  selectCustomRange,
  useWorkspaceStore,
} from '../stores/workspaceStore';
import type { AnalyticsFilter } from '../types/ipc';
import { ModelsPricingPage } from './ModelsPricingPage';
import { SpeedLatencyPage } from './SpeedLatencyPage';

type CompareView = 'compare' | 'cost' | 'speed' | 'catalog';

const VIEWS: Array<{ id: CompareView; label: string }> = [
  { id: 'compare', label: '综合对比' },
  { id: 'cost', label: '开销' },
  { id: 'speed', label: '速度' },
  { id: 'catalog', label: '单价目录' },
];

const SORT_COLUMNS: Array<{ id: ComparisonSortKey; label: string }> = [
  { id: 'cost_micro_usd', label: '等效费用' },
  { id: 'total_tokens', label: 'Token' },
  { id: 'cost_per_m_micro_usd', label: '每百万 Token' },
  { id: 'cache_hit_ratio', label: '缓存命中' },
  { id: 'mean_tps', label: '平均速率' },
  { id: 'avg_ttft_ms', label: '首字延迟' },
];

/** Merged 模型对比 page: price, usage and speed comparison across models and providers. */
export function ModelsComparePage() {
  const [view, setView] = useState<CompareView>('compare');

  return (
    <div className="space-y-6">
      <section
        className="flex flex-wrap items-center justify-between gap-3 pb-5"
        style={{ borderBottom: '1px solid var(--border-hairline)' }}
      >
        <div className="flex flex-wrap items-center gap-2.5">
          <h1 className="text-[16px] font-semibold tracking-tight">模型对比</h1>
          <div
            className="seg-group"
            role="tablist"
            style={{ backgroundColor: 'var(--bg-sidebar)', borderColor: 'var(--border-hairline)' }}
          >
            {VIEWS.map((v) => {
              const active = view === v.id;
              return (
                <button
                  key={v.id}
                  type="button"
                  role="tab"
                  aria-selected={active}
                  data-active={active}
                  onClick={() => setView(v.id)}
                  className="seg-pill px-2.5 py-0.5 text-[11.5px] cursor-pointer whitespace-nowrap"
                  style={{
                    color: active ? 'var(--accent-tonal-text)' : 'var(--text-secondary)',
                    fontWeight: active ? 500 : 400,
                  }}
                >
                  {v.label}
                </button>
              );
            })}
          </div>
        </div>
        {view !== 'catalog' && <TimeRangeControl />}
      </section>

      {view === 'compare' && <ComparisonView />}
      {view === 'cost' && <ModelsPricingPage embedded view="efficiency" />}
      {view === 'speed' && <SpeedLatencyPage embedded />}
      {view === 'catalog' && <ModelsPricingPage embedded view="catalog" />}
    </div>
  );
}

function ComparisonView() {
  const {
    timePreset,
    customStartDate,
    customEndDate,
    customPrecise,
    customStartMs,
    customEndMs,
    providerFilter,
    deviceFilter,
    tzOffsetMinutes,
    currency,
    usdToCnyRate,
    getFilter,
  } = useWorkspaceStore();
  const rangeFields = { customStartDate, customEndDate, customPrecise, customStartMs, customEndMs };
  const customRange = selectCustomRange(rangeFields);
  const customRangeKey = buildCustomRangeKey(rangeFields);
  const [sortKey, setSortKey] = useState<ComparisonSortKey>('cost_micro_usd');
  const [sortDesc, setSortDesc] = useState(true);
  const [priceBasis, setPriceBasis] = useState<PriceBasis>('actual');

  const filter = useMemo(
    () => getFilter(),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [timePreset, customRangeKey, providerFilter, deviceFilter]
  );

  // Same query keys as the 开销 / 速度 views so switching views reuses the cache.
  const { data: models, isLoading: modelsLoading } = useQuery({
    queryKey: ['models_pricing', filter],
    queryFn: () => ipc.getModelsPricing(filter),
    placeholderData: keepPreviousData,
  });
  const { data: speed, isLoading: speedLoading } = useQuery({
    queryKey: ['speed_latency', timePreset, customRangeKey, providerFilter, deviceFilter, tzOffsetMinutes],
    queryFn: () => {
      const range = computeTimeRange(timePreset, customRange);
      const f: AnalyticsFilter = {
        start_ms: range.start_ms,
        end_ms: range.end_ms,
        provider_id: providerFilter === 'all' ? null : providerFilter,
        device_id: deviceFilter === 'all' ? null : deviceFilter,
        tz_offset_minutes: tzOffsetMinutes,
        all_time: range.all_time,
      };
      return ipc.getSpeedLatency(f);
    },
    placeholderData: keepPreviousData,
    refetchInterval: CLOCK_REFRESH_MS,
  });

  const modelRows = useMemo(() => buildModelComparison(models, speed), [models, speed]);
  const providerRows = useMemo(
    () => buildProviderComparison(modelRows, speed, models),
    [modelRows, speed, models]
  );
  // Per-1M price under the selected basis; providers without a paid plan fall back to API 实付.
  const basisRows = useMemo(
    () =>
      modelRows.map((r) => {
        const price = priceForBasis(r, priceBasis);
        return {
          ...r,
          cost_per_m_micro_usd: price?.per_m_micro_usd ?? null,
          price_fallback: price !== null && price.basis !== priceBasis,
        };
      }),
    [modelRows, priceBasis]
  );
  const sortedModels = useMemo(
    () => sortComparisonRows(basisRows, sortKey, sortDesc),
    [basisRows, sortKey, sortDesc]
  );

  const bestValue = useMemo(() => {
    const priced = modelRows.filter((r) => r.cost_per_m_micro_usd !== null && r.total_tokens > 0);
    const cheapest = sortComparisonRows(priced, 'cost_per_m_micro_usd', false)[0] ?? null;
    const fastest = sortComparisonRows(modelRows, 'mean_tps', true)[0] ?? null;
    const snappiest = sortComparisonRows(modelRows, 'avg_ttft_ms', false)[0] ?? null;
    const top = modelRows[0] ?? null;
    return { cheapest, fastest, snappiest, top };
  }, [modelRows]);

  const loading = (modelsLoading && !models) || (speedLoading && !speed);
  const fmtPerM = (v: number | null) => (v === null ? '—' : formatCost(v, currency, usdToCnyRate));
  const toggleSort = (key: ComparisonSortKey) => {
    if (key === sortKey) {
      setSortDesc((d) => !d);
    } else {
      setSortKey(key);
      setSortDesc(key !== 'cost_per_m_micro_usd' && key !== 'avg_ttft_ms');
    }
  };

  if (loading) {
    return (
      <div className="py-12 text-center" style={{ color: 'var(--text-muted)' }}>
        汇总模型数据…
      </div>
    );
  }
  if (modelRows.length === 0) {
    return (
      <div className="py-12 text-center text-[12.5px]" style={{ color: 'var(--text-muted)' }}>
        {formatResolvedDateSpan(timePreset, customRange)} 等待模型调用数据
      </div>
    );
  }

  return (
    <div className="space-y-7" data-testid="models-compare-view">
      {/* Highlights */}
      <section className="surface-tonal-band px-5 py-3.5 grid grid-cols-2 lg:grid-cols-4 gap-y-3">
        {(
          [
            { label: '主力模型', row: bestValue.top, value: bestValue.top ? formatCost(bestValue.top.cost_micro_usd, currency, usdToCnyRate) : '—' },
            { label: '单价最低', row: bestValue.cheapest, value: bestValue.cheapest ? `${fmtPerM(bestValue.cheapest.cost_per_m_micro_usd)} / M` : '—' },
            { label: '速率最快', row: bestValue.fastest, value: formatTps(bestValue.fastest?.mean_tps) },
            { label: '首字最快', row: bestValue.snappiest, value: formatDurationMs(bestValue.snappiest?.avg_ttft_ms) },
          ] as const
        ).map((item, idx) => (
          <div
            key={item.label}
            className={idx === 0 ? 'lg:pr-5' : idx === 3 ? 'lg:pl-5' : 'lg:px-5'}
            style={idx === 0 ? undefined : { borderLeft: '1px solid var(--border-hairline)' }}
          >
            <div className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
              {item.label}
            </div>
            <div className="mt-1 text-[17px] font-semibold tabular-nums truncate">{item.value}</div>
            <div className="mt-0.5 text-[11px] truncate font-mono" style={{ color: 'var(--text-muted)' }}>
              {item.row ? item.row.model_id : '—'}
            </div>
          </div>
        ))}
      </section>

      {/* Provider comparison */}
      <section>
        <h2 className="text-[13.5px] font-semibold mb-3">提供商对比</h2>
        <div className="overflow-x-auto">
          <table className="workspace-table w-full border-collapse text-left" data-testid="provider-compare-table">
            <thead>
              <tr
                className="text-[11px] font-medium uppercase tracking-wider"
                style={{ color: 'var(--text-muted)', borderBottom: '1px solid var(--border-hairline)' }}
              >
                <th className="py-2 pr-4">提供商</th>
                <th className="py-2 px-3 w-[22%]">费用占比</th>
                <th className="py-2 px-3 text-right">Token</th>
                <th className="py-2 px-3 text-right">每百万 Token</th>
                <th className="py-2 px-3 text-right">缓存命中</th>
                <th className="py-2 px-3 text-right">平均速率</th>
                <th className="py-2 px-3 text-right">P95 速率</th>
                <th className="py-2 pl-3 pr-5 text-right">首字延迟</th>
              </tr>
            </thead>
            <tbody>
              {providerRows.map((p) => (
                <tr key={p.provider_id} className="row-interactive" style={{ borderBottom: '1px solid var(--border-hairline)' }}>
                  <td className="py-2.5 pr-4 font-medium">
                    <div className="flex items-center gap-2">
                      <span className="inline-block w-2 h-2 rounded-full shrink-0" style={{ backgroundColor: providerAccentColor(p.provider_id) }} />
                      <span>{providerDisplayName(p.provider_id)}</span>
                      <span className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                        {p.model_count} 模型
                      </span>
                    </div>
                  </td>
                  <td className="py-2.5 px-3">
                    <div className="flex items-center gap-2 tabular-nums text-[12px]">
                      <div className="flex-1 min-w-[60px]">
                        <InlineProgressBar percent={p.cost_share * 100} accentColor={providerAccentColor(p.provider_id)} heightPx={4} />
                      </div>
                      <span className="w-[86px] text-right">{formatCost(p.cost_micro_usd, currency, usdToCnyRate)}</span>
                    </div>
                  </td>
                  <td className="py-2.5 px-3 text-right tabular-nums">{formatTokens(p.total_tokens)}</td>
                  <td className="py-2.5 px-3 text-right tabular-nums">{fmtPerM(p.cost_per_m_micro_usd)}</td>
                  <td className="py-2.5 px-3 text-right tabular-nums">{p.cache_hit_ratio === null ? '—' : formatPercent(p.cache_hit_ratio, true)}</td>
                  <td className="py-2.5 px-3 text-right tabular-nums" style={{ color: 'var(--accent-sky)' }}>{formatTps(p.mean_tps)}</td>
                  <td className="py-2.5 px-3 text-right tabular-nums">{formatTps(p.p95_tps)}</td>
                  <td className="py-2.5 pl-3 pr-5 text-right tabular-nums">{formatDurationMs(p.avg_ttft_ms)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>

      {/* Model comparison */}
      <section>
        <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
          <h2 className="text-[13.5px] font-semibold">模型对比</h2>
            <div
              className="seg-group"
              role="radiogroup"
              aria-label="价格口径"
              style={{ backgroundColor: 'var(--bg-sidebar)', borderColor: 'var(--border-hairline)' }}
            >
              {PRICE_BASIS_OPTIONS.map((o) => {
                const active = priceBasis === o.id;
                return (
                  <button
                    key={o.id}
                    type="button"
                    role="radio"
                    aria-checked={active}
                    data-active={active}
                    onClick={() => setPriceBasis(o.id)}
                    className="seg-pill px-2.5 py-0.5 text-[11.5px] cursor-pointer whitespace-nowrap"
                    style={{
                      color: active ? 'var(--accent-tonal-text)' : 'var(--text-secondary)',
                      fontWeight: active ? 500 : 400,
                    }}
                  >
                    {o.label}
                  </button>
                );
              })}
            </div>
        </div>
        <div className="overflow-x-auto">
          <table className="workspace-table w-full border-collapse text-left" data-testid="model-compare-table">
            <thead>
              <tr
                className="text-[11px] font-medium uppercase tracking-wider"
                style={{ color: 'var(--text-muted)', borderBottom: '1px solid var(--border-hairline)' }}
              >
                <th className="py-2 pr-4">模型</th>
                {SORT_COLUMNS.map((col) => (
                  <th
                    key={col.id}
                    onClick={() => toggleSort(col.id)}
                    aria-sort={sortKey === col.id ? (sortDesc ? 'descending' : 'ascending') : 'none'}
                    className="py-2 px-3 text-right cursor-pointer select-none hover:text-[var(--text-primary)]"
                  >
                    {col.label}
                    {sortKey === col.id ? (sortDesc ? ' ↓' : ' ↑') : ''}
                  </th>
                ))}
                <th className="py-2 pl-3 pr-5 text-right">输出单价 / M</th>
              </tr>
            </thead>
            <tbody>
              {sortedModels.map((r) => (
                <tr key={r.key} className="row-interactive" style={{ borderBottom: '1px solid var(--border-hairline)' }}>
                  <td className="py-2.5 pr-4">
                    <div className="flex items-center gap-2 min-w-0">
                      <span className="inline-block w-2 h-2 rounded-full shrink-0" style={{ backgroundColor: providerAccentColor(r.provider_id) }} />
                      <span className="font-mono text-[12px] truncate max-w-[220px]" title={r.model_id}>
                        {r.model_id}
                      </span>
                      <span className="text-[10.5px] shrink-0" style={{ color: 'var(--text-muted)' }}>
                        {providerDisplayName(r.provider_id)}
                      </span>
                    </div>
                  </td>
                  <td className="py-2.5 px-3 text-right tabular-nums font-medium">{formatCost(r.cost_micro_usd, currency, usdToCnyRate)}</td>
                  <td className="py-2.5 px-3 text-right tabular-nums">{formatTokens(r.total_tokens)}</td>
                  <td
                    className="py-2.5 px-3 text-right tabular-nums"
                    style={r.price_fallback ? { color: 'var(--text-muted)' } : undefined}
                    title={r.price_fallback ? 'API 实付' : undefined}
                  >
                    {fmtPerM(r.cost_per_m_micro_usd)}
                  </td>
                  <td className="py-2.5 px-3 text-right tabular-nums">{r.cache_hit_ratio === null ? '—' : formatPercent(r.cache_hit_ratio, true)}</td>
                  <td className="py-2.5 px-3 text-right tabular-nums" style={{ color: 'var(--accent-sky)' }}>{formatTps(r.mean_tps)}</td>
                  <td className="py-2.5 px-3 text-right tabular-nums">{formatDurationMs(r.avg_ttft_ms)}</td>
                  <td className="py-2.5 pl-3 pr-5 text-right tabular-nums" style={{ color: 'var(--text-secondary)' }}>{fmtPerM(r.output_price_per_m_micro_usd)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}
