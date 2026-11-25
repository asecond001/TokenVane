import { useMemo, useState } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import {
  createColumnHelper,
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  useReactTable,
  type SortingState,
} from '@tanstack/react-table';
import type { EChartsOption } from 'echarts';
import { AnimatedNumber } from '../components/AnimatedNumber';
import { EChartBox } from '../components/EChartBox';
import { TimeRangeControl } from '../components/TimeRangeControl';
import { useChartTheme } from '../lib/chartTheme';
import {
  formatDurationMs,
  formatExactInt,
  formatTps,
  providerAccentColor,
  providerDisplayName,
} from '../lib/format';
import { ipc } from '../lib/ipc';
import {
  buildCustomRangeKey,
  computeTimeRange,
  selectCustomRange,
  useWorkspaceStore,
} from '../stores/workspaceStore';
import type { AnalyticsFilter, ModelSpeedStats } from '../types/ipc';
import { CLOCK_REFRESH_MS } from '../lib/liveRefresh';

type SpeedGroupMode = 'model' | 'provider';
type SpeedChartMetric = 'tps' | 'latency';

interface SpeedChartItem {
  label: string;
  providerId: string;
  p50: number;
  mean: number;
  p95: number;
  sampleCount: number;
  avgDurationMs: number;
  avgTtftMs: number | null;
}

const colHelper = createColumnHelper<ModelSpeedStats>();

function hexToRgba(hex: string, alpha: number): string {
  const normalized = hex.replace('#', '');
  if (normalized.length !== 6) {
    return hex;
  }
  const r = Number.parseInt(normalized.slice(0, 2), 16);
  const g = Number.parseInt(normalized.slice(2, 4), 16);
  const b = Number.parseInt(normalized.slice(4, 6), 16);
  if ([r, g, b].some(Number.isNaN)) {
    return hex;
  }
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

function escapeHtml(text: string): string {
  return text
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

export interface SpeedLatencyPageProps {
  /** Rendered inside the merged 模型对比 page: header and range control are owned by the host. */
  embedded?: boolean;
}

export function SpeedLatencyPage({ embedded = false }: SpeedLatencyPageProps = {}) {
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
  } = useWorkspaceStore();
  const rangeFields = { customStartDate, customEndDate, customPrecise, customStartMs, customEndMs };
  const customRange = selectCustomRange(rangeFields);
  const customRangeKey = buildCustomRangeKey(rangeFields);
  const chartTheme = useChartTheme();
  const [groupMode, setGroupMode] = useState<SpeedGroupMode>('model');
  const [chartMetric, setChartMetric] = useState<SpeedChartMetric>('tps');
  const [sorting, setSorting] = useState<SortingState>([{ id: 'sample_count', desc: true }]);

  const { data, isLoading } = useQuery({
    queryKey: [
      'speed_latency',
      timePreset,
      customRangeKey,
      providerFilter,
      deviceFilter,
      tzOffsetMinutes,
    ],
    queryFn: () => {
      const range = computeTimeRange(timePreset, customRange);
      const filter: AnalyticsFilter = {
        start_ms: range.start_ms,
        end_ms: range.end_ms,
        provider_id: providerFilter === 'all' ? null : providerFilter,
        device_id: deviceFilter === 'all' ? null : deviceFilter,
        tz_offset_minutes: tzOffsetMinutes,
        all_time: range.all_time,
      };
      return ipc.getSpeedLatency(filter);
    },
    placeholderData: keepPreviousData,
    refetchInterval: CLOCK_REFRESH_MS,
  });

  const providers = data?.providers ?? [];
  const models = data?.models ?? [];

  const summaryStats = useMemo(() => {
    const sourceRows = providers.length > 0 ? providers : models;
    let totalSamples = 0;
    let weightedTpsSum = 0;
    let weightedDurationSum = 0;
    let ttftSamples = 0;
    let weightedTtftSum = 0;
    for (const row of sourceRows) {
      totalSamples += row.sample_count;
      weightedTpsSum += row.mean_tps * row.sample_count;
      weightedDurationSum += row.avg_duration_ms * row.sample_count;
      if (row.avg_ttft_ms !== null) {
        ttftSamples += row.sample_count;
        weightedTtftSum += row.avg_ttft_ms * row.sample_count;
      }
    }
    const fastestModel =
      models.length > 0
        ? [...models].sort((a, b) => b.p95_tps - a.p95_tps)[0]
        : null;
    return {
      totalSamples,
      weightedMeanTps: totalSamples > 0 ? weightedTpsSum / totalSamples : 0,
      weightedDurationMs: totalSamples > 0 ? weightedDurationSum / totalSamples : 0,
      weightedTtftMs: ttftSamples > 0 ? weightedTtftSum / ttftSamples : null,
      fastestModel,
    };
  }, [providers, models]);

  const groupedModelsForChart = useMemo<ModelSpeedStats[]>(() => {
    if (models.length === 0) return [];
    const maxSamplesByModel = new Map<string, number>();
    for (const row of models) {
      const prev = maxSamplesByModel.get(row.model_id) ?? 0;
      if (row.sample_count > prev) {
        maxSamplesByModel.set(row.model_id, row.sample_count);
      }
    }
    return [...models]
      .sort((a, b) => {
        const aMax = maxSamplesByModel.get(a.model_id) ?? 0;
        const bMax = maxSamplesByModel.get(b.model_id) ?? 0;
        if (bMax !== aMax) return bMax - aMax;
        if (a.model_id !== b.model_id) return a.model_id.localeCompare(b.model_id);
        return b.mean_tps - a.mean_tps;
      })
      .slice(0, 12);
  }, [models]);

  const chartItems = useMemo<SpeedChartItem[]>(() => {
    if (groupMode === 'provider') {
      return providers.map((p) => ({
        label: providerDisplayName(p.provider_id),
        providerId: p.provider_id,
        p50: p.p50_tps,
        mean: p.mean_tps,
        p95: p.p95_tps,
        sampleCount: p.sample_count,
        avgDurationMs: p.avg_duration_ms,
        avgTtftMs: p.avg_ttft_ms,
      }));
    }
    return groupedModelsForChart.map((m) => ({
      label: `${m.model_id} · ${providerDisplayName(m.provider_id)}`,
      providerId: m.provider_id,
      p50: m.p50_tps,
      mean: m.mean_tps,
      p95: m.p95_tps,
      sampleCount: m.sample_count,
      avgDurationMs: m.avg_duration_ms,
      avgTtftMs: m.avg_ttft_ms,
    }));
  }, [groupMode, providers, groupedModelsForChart]);

  const speedComparisonOption = useMemo<EChartsOption>(() => {
    if (chartMetric === 'latency') {
      return {
        backgroundColor: 'transparent',
        animationDuration: 280,
        animationDurationUpdate: 260,
        tooltip: {
          trigger: 'axis',
          axisPointer: { type: 'shadow' },
          backgroundColor: chartTheme.tooltipBg,
          borderColor: chartTheme.tooltipBorder,
          textStyle: { color: chartTheme.tooltipText, fontSize: 12 },
          formatter: (params: unknown) => {
            const list = Array.isArray(params)
              ? (params as Array<{ dataIndex?: number }>)
              : [];
            const idx = list[0]?.dataIndex;
            if (idx === undefined) return '';
            const item = chartItems[idx];
            if (!item) return '';
            return [
              `<b>${escapeHtml(item.label)}</b>`,
              `平均总耗时: <b>${formatDurationMs(item.avgDurationMs)}</b>`,
              `平均首字延迟: <b>${formatDurationMs(item.avgTtftMs)}</b>`,
              `样本数: ${formatExactInt(item.sampleCount)}`,
            ].join('<br/>');
          },
        },
        legend: {
          top: 0,
          right: 0,
          textStyle: { color: chartTheme.textSecondary, fontSize: 11 },
          itemWidth: 10,
          itemHeight: 6,
        },
        grid: { top: 34, right: 24, bottom: 26, left: 16, containLabel: true },
        xAxis: {
          type: 'value',
          name: 'ms',
          nameTextStyle: { color: chartTheme.textMuted, fontSize: 11 },
          splitLine: { lineStyle: { color: chartTheme.splitLineColor, type: 'dashed' } },
          axisLabel: {
            color: chartTheme.textMuted,
            fontSize: 11,
            formatter: (val: number) => formatDurationMs(val),
          },
        },
        yAxis: {
          type: 'category',
          inverse: true,
          data: chartItems.map((item) => item.label),
          axisLine: { lineStyle: { color: chartTheme.axisLineColor } },
          axisTick: { show: false },
          axisLabel: {
            color: chartTheme.textSecondary,
            fontSize: 11,
            width: 220,
            overflow: 'truncate',
          },
        },
        series: [
          {
            id: `latency-ttft-${groupMode}`,
            name: '平均首字延迟',
            type: 'bar',
            barWidth: 7,
            barGap: '28%',
            universalTransition: true,
            itemStyle: {
              color: '#0ea5e9',
              borderRadius: [0, 3, 3, 0],
            },
            data: chartItems.map((item) => Math.round(item.avgTtftMs ?? 0)),
          },
          {
            id: `latency-duration-${groupMode}`,
            name: '平均总耗时',
            type: 'bar',
            barWidth: 7,
            universalTransition: true,
            itemStyle: {
              color: '#f59e0b',
              borderRadius: [0, 3, 3, 0],
            },
            data: chartItems.map((item) => Math.round(item.avgDurationMs)),
          },
        ],
      };
    }

    return {
      backgroundColor: 'transparent',
      animationDuration: 280,
      animationDurationUpdate: 260,
      tooltip: {
        trigger: 'axis',
        axisPointer: { type: 'shadow' },
        backgroundColor: chartTheme.tooltipBg,
        borderColor: chartTheme.tooltipBorder,
        textStyle: { color: chartTheme.tooltipText, fontSize: 12 },
        formatter: (params: unknown) => {
          const list = Array.isArray(params)
            ? (params as Array<{ dataIndex?: number }>)
            : [];
          const idx = list[0]?.dataIndex;
          if (idx === undefined) return '';
          const item = chartItems[idx];
          if (!item) return '';
          const lines = [
            `<b>${escapeHtml(item.label)}</b>`,
            `P50: <b>${formatTps(item.p50)}</b>`,
            `均值: <b>${formatTps(item.mean)}</b>`,
            `P95: <b>${formatTps(item.p95)}</b>`,
            `样本数: ${formatExactInt(item.sampleCount)} · 平均耗时: ${formatDurationMs(item.avgDurationMs)}`,
          ];
          if (item.avgTtftMs !== null) {
            lines.push(`平均首字延迟: ${formatDurationMs(item.avgTtftMs)}`);
          }
          return lines.join('<br/>');
        },
      },
      legend: {
        top: 0,
        right: 0,
        textStyle: { color: chartTheme.textSecondary, fontSize: 11 },
        itemWidth: 10,
        itemHeight: 6,
      },
      grid: { top: 34, right: 24, bottom: 26, left: 16, containLabel: true },
      xAxis: {
        type: 'value',
        name: 'tok/s',
        nameTextStyle: { color: chartTheme.textMuted, fontSize: 11 },
        splitLine: { lineStyle: { color: chartTheme.splitLineColor, type: 'dashed' } },
        axisLabel: { color: chartTheme.textMuted, fontSize: 11 },
      },
      yAxis: {
        type: 'category',
        inverse: true,
        data: chartItems.map((item) => item.label),
        axisLine: { lineStyle: { color: chartTheme.axisLineColor } },
        axisTick: { show: false },
        axisLabel: {
          color: chartTheme.textSecondary,
          fontSize: 11,
          width: 220,
          overflow: 'truncate',
        },
      },
      series: [
        {
          id: `speed-p50-${groupMode}`,
          name: 'P50',
          type: 'bar',
          barWidth: 6,
          barGap: '30%',
          universalTransition: true,
          itemStyle: {
            color: chartTheme.textMuted,
            borderRadius: [0, 3, 3, 0],
          },
          data: chartItems.map((item) => ({
            value: Number(item.p50.toFixed(1)),
            itemStyle: {
              color: hexToRgba(providerAccentColor(item.providerId), 0.45),
              borderRadius: [0, 3, 3, 0],
            },
          })),
        },
        {
          id: `speed-mean-${groupMode}`,
          name: '均值',
          type: 'bar',
          barWidth: 6,
          universalTransition: true,
          itemStyle: {
            color: chartTheme.textPrimary,
            borderRadius: [0, 3, 3, 0],
          },
          data: chartItems.map((item) => ({
            value: Number(item.mean.toFixed(1)),
            itemStyle: {
              color: providerAccentColor(item.providerId),
              borderRadius: [0, 3, 3, 0],
            },
          })),
        },
        {
          id: `speed-p95-${groupMode}`,
          name: 'P95',
          type: 'bar',
          barWidth: 6,
          universalTransition: true,
          itemStyle: {
            color: chartTheme.textSecondary,
            borderRadius: [0, 3, 3, 0],
          },
          data: chartItems.map((item) => ({
            value: Number(item.p95.toFixed(1)),
            itemStyle: {
              color: hexToRgba(providerAccentColor(item.providerId), 0.72),
              borderColor: providerAccentColor(item.providerId),
              borderWidth: 1,
              borderRadius: [0, 3, 3, 0],
            },
          })),
        },
      ],
    };
  }, [chartItems, chartTheme, groupMode, chartMetric]);

  const columns = useMemo(
    () => [
      colHelper.accessor('model_id', {
        header: '模型',
        cell: (info) => <span className="font-medium select-text">{info.getValue()}</span>,
      }),
      colHelper.accessor('provider_id', {
        header: '模型提供商',
        cell: (info) => {
          const pid = info.getValue();
          return (
            <div className="flex items-center gap-2" style={{ color: 'var(--text-secondary)' }}>
              <span
                className="inline-block w-2 h-2 rounded-full shrink-0"
                style={{ backgroundColor: providerAccentColor(pid) }}
              />
              <span>{providerDisplayName(pid)}</span>
            </div>
          );
        },
      }),
      colHelper.accessor('sample_count', {
        header: '样本数',
        cell: (info) => (
          <span className="tabular-nums">{formatExactInt(info.getValue())}</span>
        ),
      }),
      colHelper.accessor('mean_tps', {
        header: '均值',
        cell: (info) => (
          <span className="font-medium tabular-nums">{formatTps(info.getValue())}</span>
        ),
      }),
      colHelper.accessor('p50_tps', {
        header: 'P50',
        cell: (info) => <span className="tabular-nums">{formatTps(info.getValue())}</span>,
      }),
      colHelper.accessor('p95_tps', {
        header: 'P95',
        cell: (info) => <span className="tabular-nums">{formatTps(info.getValue())}</span>,
      }),
      colHelper.accessor('max_tps', {
        header: '峰值',
        cell: (info) => <span className="tabular-nums">{formatTps(info.getValue())}</span>,
      }),
      colHelper.accessor('latest_tps', {
        header: '最新',
        cell: (info) => (
          <span className="tabular-nums" style={{ color: 'var(--accent-emerald)' }}>
            {formatTps(info.getValue())}
          </span>
        ),
      }),
      colHelper.accessor('avg_duration_ms', {
        header: '平均耗时',
        cell: (info) => (
          <span className="tabular-nums" style={{ color: 'var(--text-secondary)' }}>
            {formatDurationMs(info.getValue())}
          </span>
        ),
      }),
      colHelper.accessor('avg_ttft_ms', {
        header: '首字延迟',
        sortUndefined: 'last',
        cell: (info) => (
          <span className="tabular-nums" style={{ color: 'var(--text-secondary)' }}>
            {formatDurationMs(info.getValue())}
          </span>
        ),
      }),
    ],
    []
  );

  const table = useReactTable({
    data: models,
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });

  const chartHeight =
    groupMode === 'provider'
      ? Math.max(180, chartItems.length * 56 + 60)
      : Math.max(220, chartItems.length * 42 + 60);

  return (
    <div className="space-y-7">
      {!embedded && (
        <section
          className="flex flex-wrap items-center justify-between gap-3 pb-5"
          style={{ borderBottom: '1px solid var(--border-hairline)' }}
        >
          <div>
            <h1 className="text-[16px] font-semibold tracking-tight">速度监测</h1>
          </div>
          <TimeRangeControl />
        </section>
      )}

      {isLoading && !data ? (
        <div className="py-12 text-center" style={{ color: 'var(--text-muted)' }}>
          计算速度分布与延迟统计…
        </div>
      ) : providers.length === 0 && models.length === 0 ? (
        <div className="py-12 text-center text-[12.5px]" style={{ color: 'var(--text-muted)' }}>
          当前时间窗等待有效耗时样本
        </div>
      ) : (
        <>
          {/* Module 0: 4-Metric Global Speed & Latency Summary Strip */}
          <section className="surface-tonal-band px-5 py-4 grid grid-cols-2 lg:grid-cols-4 gap-y-3">
            <div className="lg:pr-5">
              <div className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
                全局加权均值
              </div>
              <div className="mt-1 text-[22px] font-semibold tracking-tight tabular-nums">
                <AnimatedNumber value={summaryStats.weightedMeanTps} formatter={formatTps} />
              </div>
              <div className="mt-0.5 text-[11px]" style={{ color: 'var(--text-muted)' }}>
                按有效样本数加权
              </div>
            </div>

            <div className="lg:px-5" style={{ borderLeft: '1px solid var(--border-hairline)' }}>
              <div className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
                最快模型 P95
              </div>
              <div
                className="mt-1 text-[22px] font-semibold tracking-tight tabular-nums"
                style={{ color: 'var(--accent-sky)' }}
              >
                {summaryStats.fastestModel
                  ? formatTps(summaryStats.fastestModel.p95_tps)
                  : '—'}
              </div>
              <div
                className="mt-0.5 text-[11px] truncate"
                style={{ color: 'var(--text-muted)' }}
                title={summaryStats.fastestModel?.model_id}
              >
                {summaryStats.fastestModel ? summaryStats.fastestModel.model_id : '等待模型样本'}
              </div>
            </div>

            <div className="lg:px-5" style={{ borderLeft: '1px solid var(--border-hairline)' }}>
              <div className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
                全局平均首字延迟
              </div>
              <div className="mt-1 text-[22px] font-semibold tracking-tight tabular-nums">
                {formatDurationMs(summaryStats.weightedTtftMs)}
              </div>
              <div
                className="mt-0.5 text-[11px] tabular-nums"
                style={{ color: 'var(--text-muted)' }}
              >
                平均总耗时 {formatDurationMs(summaryStats.weightedDurationMs)}
              </div>
            </div>

            <div className="lg:pl-5" style={{ borderLeft: '1px solid var(--border-hairline)' }}>
              <div className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
                有效测速样本
              </div>
              <div className="mt-1 text-[22px] font-semibold tracking-tight tabular-nums">
                {formatExactInt(summaryStats.totalSamples)}
              </div>
              <div
                className="mt-0.5 text-[11px] tabular-nums"
                style={{ color: 'var(--text-muted)' }}
              >
                {models.length} 组模型 · {providers.length} 个源
              </div>
            </div>
          </section>

          {/* Module 1: Provider Speed & Latency Overview Strip */}
          <section
            className="pb-6"
            style={{ borderBottom: '1px solid var(--border-hairline)' }}
          >
            <div className="flex items-center justify-between mb-3">
              <div className="flex items-center gap-2">
                <h2 className="text-[13.5px] font-semibold">模型提供商维度速度与延迟总览</h2>
                {data?.from_hourly_rollups && (
                  <span
                    className="px-1.5 py-0.5 text-[10.5px] rounded-md"
                    style={{
                      color: 'var(--text-muted)',
                      border: '1px solid var(--border-hairline)',
                    }}
                  >
                    同步聚合口径
                  </span>
                )}
              </div>
            </div>

            <div className="grid grid-cols-1 md:grid-flow-col md:auto-cols-fr gap-y-4">
              {providers.map((p, idx) => (
                <div
                  key={p.provider_id}
                  className={idx === 0 ? 'md:pr-5' : 'md:px-5'}
                  style={
                    idx > 0
                      ? { borderLeft: '1px solid var(--border-hairline)' }
                      : undefined
                  }
                >
                  <div className="flex items-center justify-between gap-2">
                    <div className="flex items-center gap-2">
                      <span
                        className="inline-block w-2 h-2 rounded-full shrink-0"
                        style={{ backgroundColor: providerAccentColor(p.provider_id) }}
                      />
                      <span
                        className="text-[12px] font-medium"
                        style={{ color: 'var(--text-secondary)' }}
                      >
                        {providerDisplayName(p.provider_id)}
                      </span>
                    </div>
                    <span className="text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
                      {formatExactInt(p.sample_count)} 样本
                    </span>
                  </div>

                  <div className="mt-1.5 flex items-baseline gap-2">
                    <span className="text-[22px] font-semibold tracking-tight tabular-nums">
                      <AnimatedNumber value={p.mean_tps} formatter={formatTps} />
                    </span>
                    <span className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                      均值
                    </span>
                  </div>

                  <div
                    className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] tabular-nums"
                    style={{ color: 'var(--text-secondary)' }}
                  >
                    <span>P50 {formatTps(p.p50_tps)}</span>
                    <span>P95 {formatTps(p.p95_tps)}</span>
                    <span>耗时 {formatDurationMs(p.avg_duration_ms)}</span>
                    {p.avg_ttft_ms !== null && (
                      <span>首字 {formatDurationMs(p.avg_ttft_ms)}</span>
                    )}
                  </div>
                </div>
              ))}
            </div>
          </section>

          {/* Module 2: Model × Provider Grouped Speed Comparison Chart */}
          <section
            className="pb-6"
            style={{ borderBottom: '1px solid var(--border-hairline)' }}
          >
            <div className="flex flex-wrap items-center justify-between gap-3 mb-3">
              <div>
                <h2 className="text-[13.5px] font-semibold">模型 × 模型提供商速率对比</h2>
              </div>

              <div className="flex flex-wrap items-center gap-2.5">
                {/* Provider Color Legend */}
                <div className="hidden sm:flex items-center gap-2.5 text-[11px]">
                  {providers.map((p) => (
                    <span
                      key={p.provider_id}
                      className="inline-flex items-center gap-1.5"
                      style={{ color: 'var(--text-secondary)' }}
                    >
                      <span
                        className="inline-block w-2 h-2 rounded-full"
                        style={{ backgroundColor: providerAccentColor(p.provider_id) }}
                      />
                      {providerDisplayName(p.provider_id)}
                    </span>
                  ))}
                </div>

                {/* Chart Metric Switch */}
                <div
                  className="seg-group"
                  style={{
                    backgroundColor: 'var(--bg-sidebar)',
                    borderColor: 'var(--border-hairline)',
                  }}
                >
                  {(
                    [
                      { id: 'tps', label: '吞吐速率 (tok/s)' },
                      { id: 'latency', label: '响应耗时与首字延迟 (ms)' },
                    ] as const
                  ).map((option) => {
                    const active = chartMetric === option.id;
                    return (
                      <button
                        key={option.id}
                        type="button"
                        data-active={active}
                        onClick={() => setChartMetric(option.id)}
                        className="seg-pill px-2.5 py-0.5 text-[11.5px] cursor-pointer whitespace-nowrap"
                        style={{
                          color: active ? 'var(--accent-tonal-text)' : 'var(--text-secondary)',
                          fontWeight: active ? 500 : 400,
                        }}
                      >
                        {option.label}
                      </button>
                    );
                  })}
                </div>

                {/* Group Mode Segmented Switch */}
                <div
                  className="seg-group"
                  style={{
                    backgroundColor: 'var(--bg-sidebar)',
                    borderColor: 'var(--border-hairline)',
                  }}
                >
                  {(
                    [
                      { id: 'model', label: '按模型分组' },
                      { id: 'provider', label: '按模型提供商分组' },
                    ] as const
                  ).map((option) => {
                    const active = groupMode === option.id;
                    return (
                      <button
                        key={option.id}
                        type="button"
                        data-active={active}
                        onClick={() => setGroupMode(option.id)}
                        className="seg-pill px-2.5 py-0.5 text-[11.5px] cursor-pointer"
                        style={{
                          color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
                          fontWeight: active ? 500 : 400,
                        }}
                      >
                        {option.label}
                      </button>
                    );
                  })}
                </div>
              </div>
            </div>

            <EChartBox option={speedComparisonOption} height={chartHeight} />
          </section>

          {/* Module 3: Model × Provider High-Density Sortable Table */}
          <section>
            <div className="flex items-center justify-between mb-3">
              <h2 className="text-[13.5px] font-semibold">模型 × 模型提供商速度与耗时明细表</h2>
              <span className="text-[11.5px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
                共 {models.length} 组
              </span>
            </div>

            <div className="overflow-x-auto">
              <table className="workspace-table w-full border-collapse text-left">
                <thead>
                  {table.getHeaderGroups().map((hg) => (
                    <tr
                      key={hg.id}
                      className="text-[11px] font-medium uppercase tracking-wider"
                      style={{
                        color: 'var(--text-muted)',
                        borderBottom: '1px solid var(--border-hairline)',
                      }}
                    >
                      {hg.headers.map((header) => (
                        <th
                          key={header.id}
                          onClick={header.column.getToggleSortingHandler()}
                          className={`py-2 px-2 cursor-pointer select-none ${
                            header.id === 'model_id' || header.id === 'provider_id'
                              ? 'text-left'
                              : 'text-right'
                          }`}
                        >
                          {flexRender(header.column.columnDef.header, header.getContext())}
                          {header.column.getIsSorted() === 'asc'
                            ? ' ↑'
                            : header.column.getIsSorted() === 'desc'
                              ? ' ↓'
                              : ''}
                        </th>
                      ))}
                    </tr>
                  ))}
                </thead>
                <tbody>
                  {table.getRowModel().rows.map((row) => (
                    <tr
                      key={row.id}
                      className="row-interactive"
                      style={{ borderBottom: '1px solid var(--border-hairline)' }}
                    >
                      {row.getVisibleCells().map((cell) => (
                        <td
                          key={cell.id}
                          className={`py-2.5 px-2 ${
                            cell.column.id === 'model_id' || cell.column.id === 'provider_id'
                              ? 'text-left'
                              : 'text-right'
                          }`}
                        >
                          {flexRender(cell.column.columnDef.cell, cell.getContext())}
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>
        </>
      )}
    </div>
  );
}
