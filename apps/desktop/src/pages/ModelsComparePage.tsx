import { useMemo, useState } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { EChartBox } from '../components/EChartBox';
import { InlineProgressBar } from '../components/InlineProgressBar';
import { TimeRangeControl } from '../components/TimeRangeControl';
import { useChartTheme } from '../lib/chartTheme';
import {
  convertMicroUsd,
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
  computeParetoFrontier,
  isFrontierEligible,
  MIN_FRONTIER_SPEED_SAMPLES,
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
import type { AnalyticsFilter, SpeedBasis } from '../types/ipc';
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

interface ScatterPoint {
  key: string;
  model_id: string;
  provider_id: string;
  /** Cost per 1M tokens in the display currency. */
  price: number;
  tps: number;
  tokens: number;
  samples: number;
  /** Subscription basis requested but the provider has no paid plan, so API 实付 is shown. */
  fallback: boolean;
  speed_basis: SpeedBasis | null;
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
  const chartTheme = useChartTheme();
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
  const sortedModels = useMemo(
    () => sortComparisonRows(modelRows, sortKey, sortDesc),
    [modelRows, sortKey, sortDesc]
  );

  const bestValue = useMemo(() => {
    const priced = modelRows.filter((r) => r.cost_per_m_micro_usd !== null && r.total_tokens > 0);
    const cheapest = sortComparisonRows(priced, 'cost_per_m_micro_usd', false)[0] ?? null;
    const fastest = sortComparisonRows(modelRows, 'mean_tps', true)[0] ?? null;
    const snappiest = sortComparisonRows(modelRows, 'avg_ttft_ms', false)[0] ?? null;
    const top = modelRows[0] ?? null;
    return { cheapest, fastest, snappiest, top };
  }, [modelRows]);

  const scatterPoints = useMemo(() => {
    const points: ScatterPoint[] = [];
    for (const r of modelRows) {
      const price = priceForBasis(r, priceBasis);
      if (!price || r.mean_tps === null) continue;
      points.push({
        key: r.key,
        model_id: r.model_id,
        provider_id: r.provider_id,
        price: convertMicroUsd(price.per_m_micro_usd, currency, usdToCnyRate),
        tps: r.mean_tps,
        tokens: r.total_tokens,
        samples: r.speed_samples,
        fallback: price.basis !== priceBasis,
        speed_basis: r.speed_basis,
      });
    }
    return points;
  }, [modelRows, priceBasis, currency, usdToCnyRate]);

  const scatterOption = useMemo<EChartsOption>(() => {
    const eligible = scatterPoints.filter(isFrontierEligible);
    const { frontier, dominatedBy } = computeParetoFrontier(eligible);
    const frontierSet = new Set(frontier);
    const byKey = new Map(scatterPoints.map((p) => [p.key, p] as const));
    const maxTokens = Math.max(1, ...scatterPoints.map((p) => p.tokens));
    const byProvider = new Map<string, ScatterPoint[]>();
    for (const p of scatterPoints) {
      byProvider.set(p.provider_id, [...(byProvider.get(p.provider_id) ?? []), p]);
    }
    const symbol = currency === 'CNY' ? '¥' : '$';
    const basisLabel = PRICE_BASIS_OPTIONS.find((o) => o.id === priceBasis)?.label ?? '';
    const median = (vals: number[]) => {
      const v = [...vals].sort((a, b) => a - b);
      if (v.length === 0) return 0;
      const mid = Math.floor(v.length / 2);
      return v.length % 2 ? v[mid] : (v[mid - 1] + v[mid]) / 2;
    };
    const prices = eligible.map((p) => p.price);
    const speeds = eligible.map((p) => p.tps);
    type AreaCorner = { xAxis: number; yAxis: number };
    const betterArea: Array<[AreaCorner, AreaCorner]> =
      eligible.length >= 3
        ? [
            [
              { xAxis: median(prices), yAxis: median(speeds) },
              { xAxis: Math.min(...prices), yAxis: Math.max(...speeds) },
            ],
          ]
        : [];
    const frontierLine = frontier
      .map((k) => byKey.get(k))
      .filter((p): p is ScatterPoint => p !== undefined)
      .map((p) => [p.price, p.tps]);

    return {
      backgroundColor: 'transparent',
      animationDuration: 280,
      animationDurationUpdate: 260,
      tooltip: {
        trigger: 'item',
        backgroundColor: chartTheme.tooltipBg,
        borderColor: chartTheme.tooltipBorder,
        textStyle: { color: chartTheme.tooltipText, fontSize: 12 },
        formatter: (params: unknown) => {
          const p = (params as { data?: { point?: ScatterPoint } }).data?.point;
          if (!p) return '';
          const priceLine = p.fallback
            ? `API 实付 ${symbol}${p.price.toFixed(3)} / M`
            : `${basisLabel} ${symbol}${p.price.toFixed(3)} / M`;
          let status = '';
          if (p.samples < MIN_FRONTIER_SPEED_SAMPLES) {
            status = `速度样本积累中（${p.samples}/${MIN_FRONTIER_SPEED_SAMPLES}）`;
          } else if (frontierSet.has(p.key)) {
            status = '帕累托前沿 · 同价位最快';
          } else {
            const by = byKey.get(dominatedBy.get(p.key) ?? '');
            if (by) status = `${by.model_id} 更快且更便宜`;
          }
          const basisNote = p.speed_basis === 'end_to_end' ? '端到端口径 · 含首字延迟' : '';
          return [
            `<b>${p.model_id}</b> · ${providerDisplayName(p.provider_id)}`,
            priceLine,
            `平均速率 ${p.tps.toFixed(1)} tok/s · ${p.samples} 样本`,
            `Token ${formatTokens(p.tokens)}`,
            basisNote,
            status,
          ]
            .filter(Boolean)
            .join('<br/>');
        },
      },
      legend: {
        top: 0,
        right: 0,
        data: [...byProvider.keys()].map(providerDisplayName),
        textStyle: { color: chartTheme.textSecondary, fontSize: 11 },
        itemWidth: 8,
        itemHeight: 8,
      },
      graphic: [
        {
          type: 'text',
          right: 24,
          top: 34,
          silent: true,
          style: { text: '快且便宜 ↗', fill: chartTheme.textMuted, fontSize: 11 },
        },
        {
          type: 'text',
          left: 64,
          bottom: 42,
          silent: true,
          style: { text: '↙ 慢且贵', fill: chartTheme.textMuted, fontSize: 11 },
        },
      ],
      grid: { top: 30, right: 20, bottom: 34, left: 56 },
      xAxis: {
        type: 'log',
        inverse: true,
        name: `每百万 Token (${currency})`,
        nameLocation: 'middle',
        nameGap: 24,
        nameTextStyle: { color: chartTheme.textMuted, fontSize: 11 },
        axisLabel: { color: chartTheme.textMuted, fontSize: 11 },
        axisLine: { lineStyle: { color: chartTheme.axisLineColor } },
        splitLine: { lineStyle: { color: chartTheme.splitLineColor, type: 'dashed' } },
      },
      yAxis: {
        type: 'value',
        scale: true,
        name: 'tok/s',
        nameTextStyle: { color: chartTheme.textMuted, fontSize: 11 },
        axisLabel: { color: chartTheme.textMuted, fontSize: 11 },
        splitLine: { lineStyle: { color: chartTheme.splitLineColor, type: 'dashed' } },
      },
      series: [
        {
          id: 'pareto-frontier',
          name: '帕累托前沿',
          type: 'line' as const,
          silent: true,
          symbol: 'none',
          z: 1,
          lineStyle: { color: chartTheme.textSecondary, width: 1.5 },
          data: frontierLine,
          markArea: {
            silent: true,
            itemStyle: { color: chartTheme.splitLineColor, opacity: 0.35 },
            data: betterArea,
          },
        },
        ...[...byProvider.entries()].map(([providerId, points]) => {
          const color = providerAccentColor(providerId);
          return {
            id: `scatter-${providerId}`,
            name: providerDisplayName(providerId),
            type: 'scatter' as const,
            z: 2,
            itemStyle: { color },
            data: points.map((p) => {
              const endToEnd = p.speed_basis === 'end_to_end';
              const sampled = p.samples >= MIN_FRONTIER_SPEED_SAMPLES;
              const onFrontier = frontierSet.has(p.key);
              return {
                name: p.model_id,
                point: p,
                value: [p.price, p.tps, p.tokens],
                symbol: p.fallback ? 'diamond' : 'circle',
                symbolSize: 8 + Math.sqrt(p.tokens / maxTokens) * 22,
                itemStyle: endToEnd
                  ? {
                      color: 'transparent',
                      borderColor: color,
                      borderWidth: 1.5,
                      borderType: 'dashed' as const,
                      opacity: 0.7,
                    }
                  : sampled
                    ? { color, opacity: onFrontier ? 0.95 : 0.3 }
                    : { color: 'transparent', borderColor: color, borderWidth: 1.5, opacity: 0.7 },
                label: {
                  show: onFrontier,
                  position: 'right' as const,
                  formatter: p.model_id,
                  color: chartTheme.textSecondary,
                  fontSize: 10.5,
                },
              };
            }),
          };
        }),
      ],
    };
  }, [scatterPoints, chartTheme, currency, priceBasis]);

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

      {/* Speed vs price Pareto frontier */}
      {scatterPoints.length > 0 && (
        <section className="pb-5" style={{ borderBottom: '1px solid var(--border-hairline)' }} data-testid="models-value-scatter">
          <div className="flex flex-wrap items-center justify-between gap-2 mb-2">
            <h2 className="text-[13.5px] font-semibold">速度与价格</h2>
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
          <EChartBox option={scatterOption} height={300} />
        </section>
      )}

      {/* Model comparison */}
      <section>
        <h2 className="text-[13.5px] font-semibold mb-3">模型对比</h2>
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
                  <td className="py-2.5 px-3 text-right tabular-nums">{fmtPerM(r.cost_per_m_micro_usd)}</td>
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
