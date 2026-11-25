import { useCallback, useMemo } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { AnimatedNumber } from '../components/AnimatedNumber';
import { EChartBox } from '../components/EChartBox';
import { InlineProgressBar } from '../components/InlineProgressBar';
import { LiveSpeedBadge } from '../components/LiveSpeedBadge';
import { useChartTheme } from '../lib/chartTheme';
import {
  TOKEN_TYPE_COLORS,
  formatBucketInTz,
  formatCost,
  formatExactInt,
  formatPercent,
  formatRelativeTime,
  formatSessionDisplayName,
  formatTokens,
  formatTps,
  providerAccentColor,
  providerDisplayName,
} from '../lib/format';
import { ipc } from '../lib/ipc';
import {
  computeTimeRange,
  getPresetDateInputs,
  useWorkspaceStore,
} from '../stores/workspaceStore';
import type { AnalyticsFilter } from '../types/ipc';
import { CLOCK_REFRESH_MS } from '../lib/liveRefresh';

function DeltaBadge({ pct, prefix = '较昨日' }: { pct: number | null | undefined; prefix?: string }) {
  if (pct === null || pct === undefined || !Number.isFinite(pct)) {
    return (
      <span className="text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
        {prefix} —
      </span>
    );
  }
  const positive = pct >= 0;
  const sign = positive ? '+' : '';
  return (
    <span
      className="text-[11px] font-medium tabular-nums"
      style={{ color: positive ? 'var(--accent-emerald)' : 'var(--accent-rose)' }}
    >
      {prefix} {sign}
      {pct.toFixed(1)}%
    </span>
  );
}

export function OverviewPage() {
  const {
    providerFilter,
    deviceFilter,
    currency,
    usdToCnyRate,
    tzOffsetMinutes,
    setActivePage,
  } = useWorkspaceStore();
  const chartTheme = useChartTheme();

  const todayDateStr = useMemo(() => getPresetDateInputs('today').start, []);

  const { data, isLoading } = useQuery({
    queryKey: ['overview', 'today', providerFilter, deviceFilter, tzOffsetMinutes],
    queryFn: () => {
      const range = computeTimeRange('today');
      const filter: AnalyticsFilter = {
        start_ms: range.start_ms,
        end_ms: range.end_ms,
        provider_id: providerFilter === 'all' ? null : providerFilter,
        device_id: deviceFilter === 'all' ? null : deviceFilter,
        tz_offset_minutes: tzOffsetMinutes,
        all_time: false,
      };
      return ipc.getOverview(filter);
    },
    placeholderData: keepPreviousData,
    refetchInterval: CLOCK_REFRESH_MS,
  });

  const costFormatter = useCallback(
    (value: number) => formatCost(value, currency, usdToCnyRate),
    [currency, usdToCnyRate]
  );

  const percentFormatter = useCallback((value: number) => formatPercent(value, true), []);

  const pulsePoints = useMemo(
    () =>
      data?.today_hourly_pulse && data.today_hourly_pulse.length > 0
        ? data.today_hourly_pulse
        : (data?.series ?? []),
    [data]
  );

  const hasPulseData = useMemo(
    () =>
      pulsePoints.some(
        (p) =>
          p.input_tokens +
            p.output_tokens +
            p.cache_read_tokens +
            p.cache_write_tokens +
            p.reasoning_tokens >
          0
      ),
    [pulsePoints]
  );

  const pulseChartOption = useMemo<EChartsOption>(() => {
    const xLabels = pulsePoints.map((pt) =>
      formatBucketInTz(pt.bucket_start_ms, 'hour_only', tzOffsetMinutes)
    );

    const seriesDefs = [
      {
        id: 'pulse-input',
        name: 'Input',
        color: TOKEN_TYPE_COLORS.input,
        values: pulsePoints.map((p) => p.input_tokens),
      },
      {
        id: 'pulse-output',
        name: 'Output',
        color: TOKEN_TYPE_COLORS.output,
        values: pulsePoints.map((p) => p.output_tokens + p.reasoning_tokens),
      },
      {
        id: 'pulse-cache-read',
        name: 'Cache Read',
        color: TOKEN_TYPE_COLORS.cache_read,
        values: pulsePoints.map((p) => p.cache_read_tokens),
      },
      {
        id: 'pulse-cache-write',
        name: 'Cache Write',
        color: TOKEN_TYPE_COLORS.cache_write,
        values: pulsePoints.map((p) => p.cache_write_tokens),
      },
    ].filter((entry) => entry.values.some((v) => v > 0));

    return {
      backgroundColor: 'transparent',
      animationDuration: 260,
      tooltip: {
        trigger: 'axis',
        axisPointer: { type: 'shadow' },
        backgroundColor: chartTheme.tooltipBg,
        borderColor: chartTheme.tooltipBorder,
        textStyle: { color: chartTheme.tooltipText, fontSize: 12 },
      },
      legend: {
        top: 0,
        right: 0,
        textStyle: { color: chartTheme.textSecondary, fontSize: 11 },
        itemWidth: 10,
        itemHeight: 6,
      },
      grid: { top: 28, right: 12, bottom: 22, left: 48, containLabel: false },
      xAxis: {
        type: 'category',
        data: xLabels,
        axisLine: { lineStyle: { color: chartTheme.axisLineColor } },
        axisLabel: { color: chartTheme.textMuted, fontSize: 10.5 },
        axisTick: { show: false },
      },
      yAxis: {
        type: 'value',
        splitLine: { lineStyle: { color: chartTheme.splitLineColor, type: 'dashed' } },
        axisLabel: {
          color: chartTheme.textMuted,
          fontSize: 10.5,
          formatter: (val: number) => formatTokens(val),
        },
      },
      series: seriesDefs.map((entry) => ({
        id: entry.id,
        name: entry.name,
        type: 'bar' as const,
        stack: 'today-hourly',
        barMaxWidth: 20,
        barCategoryGap: '26%',
        itemStyle: {
          color: entry.color,
          borderColor: chartTheme.canvasBg,
          borderWidth: 1,
          borderRadius: [2, 2, 0, 0],
        },
        data: entry.values,
      })),
    };
  }, [pulsePoints, tzOffsetMinutes, chartTheme]);

  const liveSessions = data?.active_sessions ?? [];
  const activeCount = liveSessions.filter((s) => s.activity_state === 'active').length;
  const recentCount = liveSessions.length - activeCount;

  const todayRequests = data?.today_requests ?? data?.total_requests ?? 0;
  const todayCacheRatio = data?.today_cache_hit_ratio ?? data?.cache_hit_ratio ?? 0;
  const todayCacheSaved = data?.today_cache_saved_micro_usd ?? 0;
  const todayTokensDelta =
    data?.today_tokens_delta_pct !== undefined
      ? data.today_tokens_delta_pct
      : (data?.tokens_delta_pct ?? null);
  const todayCostDelta =
    data?.today_cost_delta_pct !== undefined
      ? data.today_cost_delta_pct
      : (data?.cost_delta_pct ?? null);

  return (
    <div className="space-y-6">
      {/* Compact Header Strip */}
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2.5">
          <h1 className="text-[16px] font-semibold tracking-tight">今日实时概览</h1>
          <span
            className="px-2 py-0.5 text-[11px] rounded-md tabular-nums"
            style={{
              backgroundColor: 'var(--bg-sidebar)',
              border: '1px solid var(--border-hairline)',
              color: 'var(--text-secondary)',
            }}
          >
            今日实时 · {todayDateStr}
          </span>
        </div>
        <button
          type="button"
          onClick={() => setActivePage('trends')}
          className="btn-interactive px-2.5 py-1 text-[11.5px] rounded-lg cursor-pointer whitespace-nowrap"
          style={{
            backgroundColor: 'var(--bg-elevated)',
            border: '1px solid var(--border-subtle)',
            color: 'var(--text-secondary)',
          }}
        >
          查看趋势分析 →
        </button>
      </div>

      {/* Top Tonal 4-Metric Strip (Today Live Pulse Only) */}
      <section className="surface-tonal-band px-5 py-4 grid grid-cols-2 lg:grid-cols-4 gap-y-4">
        <div className="lg:pr-5">
          <div className="text-[11.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
            今日 API 等效费用 ({currency})
          </div>
          <div className="mt-1.5 flex items-baseline gap-2.5">
            <span className="text-[23px] font-semibold tracking-tight tabular-nums">
              {isLoading && !data ? (
                '…'
              ) : (
                <AnimatedNumber
                  value={data?.today_cost_micro_usd ?? 0}
                  formatter={costFormatter}
                />
              )}
            </span>
            <DeltaBadge pct={todayCostDelta} />
          </div>
          <div className="mt-1 text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
            按本地今日口径折算
          </div>
        </div>

        <div
          className="lg:px-5"
          style={{ borderLeft: '1px solid var(--border-hairline)' }}
        >
          <div className="text-[11.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
            今日 Token 消耗
          </div>
          <div className="mt-1.5 flex items-baseline gap-2.5">
            <span className="text-[23px] font-semibold tracking-tight tabular-nums">
              {isLoading && !data ? (
                '…'
              ) : (
                <AnimatedNumber value={data?.today_tokens ?? 0} formatter={formatTokens} />
              )}
            </span>
            <DeltaBadge pct={todayTokensDelta} />
          </div>
          <div className="mt-1 text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
            {formatExactInt(todayRequests)} 次请求
          </div>
        </div>

        <div
          className="lg:px-5"
          style={{ borderLeft: '1px solid var(--border-hairline)' }}
        >
          <div className="text-[11.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
            今日缓存命中率
          </div>
          <div className="mt-1.5 flex items-baseline gap-2.5">
            <span className="text-[23px] font-semibold tracking-tight tabular-nums">
              {isLoading && !data ? (
                '…'
              ) : (
                <AnimatedNumber value={todayCacheRatio} formatter={percentFormatter} />
              )}
            </span>
          </div>
          <div className="mt-1 text-[11px] tabular-nums" style={{ color: 'var(--accent-emerald)' }}>
            今日节省 {formatCost(todayCacheSaved, currency, usdToCnyRate)}
          </div>
        </div>

        <div
          className="lg:pl-5"
          style={{ borderLeft: '1px solid var(--border-hairline)' }}
        >
          <div className="text-[11.5px] font-medium" style={{ color: 'var(--text-secondary)' }}>
            实时生成速率
          </div>
          <div className="mt-1.5 flex items-baseline gap-2.5">
            <LiveSpeedBadge
              activeSessions={data?.active_sessions}
              liveModelSpeeds={data?.live_model_speeds}
              align="right"
              placement="bottom"
              className="cursor-default"
            >
              {(breakdown) => (
                <span
                  data-testid="overview-live-tps"
                  className="text-[23px] font-semibold tracking-tight tabular-nums"
                  style={{
                    color: breakdown.primaryTps !== null ? 'var(--accent-sky)' : 'var(--text-muted)',
                  }}
                >
                  {breakdown.primaryTps !== null ? formatTps(breakdown.primaryTps) : '空闲'}
                </span>
              )}
            </LiveSpeedBadge>
          </div>
          <div className="mt-1 text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
            均值 {formatTps(data?.avg_output_tps ?? 0)} · 峰值 {formatTps(data?.peak_output_tps ?? 0)}
          </div>
        </div>
      </section>

      {/* Mid 2-Column Live Breakdown: Today Provider/Model Share + 15m Active Sessions */}
      <section
        className="grid grid-cols-1 lg:grid-cols-12 gap-6 pb-6"
        style={{ borderBottom: '1px solid var(--border-hairline)' }}
      >
        {/* Left: Today Token Share Table & Top 3 Models */}
        <div
          className="lg:col-span-5 lg:pr-6 space-y-4 lg:border-r"
          style={{ borderColor: 'var(--border-hairline)' }}
        >
          <div className="space-y-2.5">
            <div className="flex items-center justify-between text-[12px]">
              <span className="font-semibold">今日 Token 占比</span>
              <span className="text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
                {(data?.provider_shares ?? []).length} 个活跃源
              </span>
            </div>

            {(data?.provider_shares ?? []).length > 0 ? (
              <>
                <div
                  className="h-2 w-full rounded-sm overflow-hidden flex"
                  style={{ backgroundColor: 'var(--bg-hover)' }}
                >
                  {(data?.provider_shares ?? []).map((share) => (
                    <div
                      key={share.key}
                      style={{
                        width: `${Math.max(1.5, share.token_share_percent)}%`,
                        backgroundColor: providerAccentColor(share.key),
                      }}
                      title={`${providerDisplayName(share.key)}: ${share.token_share_percent.toFixed(1)}% · ${formatCost(share.cost_micro_usd, currency, usdToCnyRate)}`}
                    />
                  ))}
                </div>

                <table className="w-full border-collapse text-[11.5px] tabular-nums">
                  <thead>
                    <tr
                      className="text-[10.5px]"
                      style={{
                        color: 'var(--text-muted)',
                        borderBottom: '1px solid var(--border-subtle)',
                      }}
                    >
                      <th className="py-1 text-left font-normal">提供商</th>
                      <th className="py-1 px-2 text-right font-normal">Token 占比</th>
                      <th className="py-1 text-right font-normal">API 等效费用</th>
                    </tr>
                  </thead>
                  <tbody>
                    {(data?.provider_shares ?? []).slice(0, 6).map((share) => (
                      <tr key={share.key}>
                        <td className="py-1 pr-2">
                          <span className="inline-flex items-center gap-1.5 min-w-0">
                            <span
                              className="inline-block w-2 h-2 rounded-full shrink-0"
                              style={{ backgroundColor: providerAccentColor(share.key) }}
                            />
                            <span className="truncate" style={{ color: 'var(--text-secondary)' }}>
                              {providerDisplayName(share.key)}
                            </span>
                          </span>
                        </td>
                        <td className="py-1 px-2 text-right font-medium">
                          {share.token_share_percent.toFixed(1)}%
                        </td>
                        <td
                          className="py-1 text-right"
                          style={{ color: 'var(--text-secondary)' }}
                        >
                          {formatCost(share.cost_micro_usd, currency, usdToCnyRate)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </>
            ) : (
              <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                等待今日用量写入
              </div>
            )}
          </div>

          <div
            className="pt-3 space-y-2"
            style={{ borderTop: '1px solid var(--border-subtle)' }}
          >
            <div className="flex items-center justify-between text-[11.5px]">
              <span className="font-medium" style={{ color: 'var(--text-secondary)' }}>
                今日主力模型 Top 3
              </span>
              <span className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
                Token 占比 · 等效费用
              </span>
            </div>
            {(data?.top_model_shares ?? []).slice(0, 3).map((model) => (
              <div key={model.key} className="space-y-1 text-[11px] tabular-nums">
                <div className="flex items-center justify-between gap-2">
                  <span className="font-mono truncate" style={{ color: 'var(--text-primary)' }} title={model.label}>
                    {model.label}
                  </span>
                  <span className="shrink-0" style={{ color: 'var(--text-muted)' }}>
                    {model.token_share_percent.toFixed(1)}% ·{' '}
                    {formatCost(model.cost_micro_usd, currency, usdToCnyRate)}
                  </span>
                </div>
                <InlineProgressBar
                  percent={model.token_share_percent}
                  accentColor="var(--accent-sky)"
                  heightPx={3}
                />
              </div>
            ))}
            {(data?.top_model_shares ?? []).length === 0 && (
              <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                等待模型调用记录
              </div>
            )}
          </div>
        </div>

        {/* Right: Active & Recent Sessions (15m Window) */}
        <div className="lg:col-span-7 flex flex-col min-w-0">
          <div className="flex items-center justify-between gap-3 pb-2.5">
            <div className="flex items-center gap-2">
              <span className="text-[12.5px] font-semibold">活跃与近期会话（15m）</span>
              <span
                className="px-1.5 py-0.5 rounded-md text-[10.5px] tabular-nums"
                style={{
                  backgroundColor: 'rgba(16, 185, 129, 0.12)',
                  color: 'var(--accent-emerald)',
                }}
              >
                {activeCount} 活跃
              </span>
              {recentCount > 0 && (
                <span
                  className="px-1.5 py-0.5 rounded-md text-[10.5px] tabular-nums"
                  style={{
                    backgroundColor: 'var(--bg-hover)',
                    color: 'var(--text-secondary)',
                  }}
                >
                  {recentCount} 近期
                </span>
              )}
            </div>

            <LiveSpeedBadge
              activeSessions={data?.active_sessions}
              liveModelSpeeds={data?.live_model_speeds}
              align="right"
              placement="bottom"
              className="tabular-nums cursor-default"
            >
              {(breakdown) => (
                <span
                  className="text-[11.5px] font-medium"
                  style={{ color: 'var(--accent-sky)' }}
                >
                  实时速率 {formatTps(breakdown.primaryTps)}
                  {breakdown.isMultiModel ? ` (${breakdown.items.length} 模型均值)` : ''}
                </span>
              )}
            </LiveSpeedBadge>
          </div>

          {liveSessions.length > 0 ? (
            <div
              className="divide-y max-h-[244px] overflow-y-auto pr-1"
              style={{ borderTop: '1px solid var(--border-subtle)' }}
            >
              {liveSessions.map((session) => {
                const isLive = session.activity_state === 'active';
                return (
                  <div
                    key={`${session.provider_id}-${session.session_key}`}
                    className="py-2 flex items-center justify-between gap-3 text-[12px]"
                    style={{ borderColor: 'var(--border-subtle)' }}
                  >
                    <div className="min-w-0 flex items-start gap-2.5">
                      <span
                        className={`mt-1.5 inline-block w-2 h-2 rounded-full shrink-0 ${isLive ? 'collector-dot-live' : ''}`}
                        style={{
                          backgroundColor: isLive ? 'var(--accent-emerald)' : 'var(--text-muted)',
                        }}
                      />
                      <div className="min-w-0">
                        <div className="flex items-center gap-2 min-w-0">
                          <span className="font-medium truncate" title={formatSessionDisplayName(session)}>
                            {formatSessionDisplayName(session)}
                          </span>
                          <span
                            className="text-[10px] px-1.5 py-0.2 rounded-md shrink-0"
                            style={{
                              backgroundColor: isLive
                                ? 'rgba(16, 185, 129, 0.14)'
                                : 'var(--bg-hover)',
                              color: isLive
                                ? 'var(--accent-emerald)'
                                : 'var(--text-muted)',
                            }}
                          >
                            {isLive ? '活跃中' : '近期'}
                          </span>
                        </div>
                        <div
                          className="text-[11px] flex items-center gap-2 truncate tabular-nums"
                          style={{ color: 'var(--text-muted)' }}
                        >
                          <span className="truncate" title={session.model_id}>
                            {providerDisplayName(session.provider_id)} · {session.model_id}
                          </span>
                          {session.project_label && (
                            <>
                              <span>·</span>
                              <span className="truncate" title={session.project_label}>{session.project_label}</span>
                            </>
                          )}
                          <span>·</span>
                          <span>{session.request_count} 次请求</span>
                        </div>
                      </div>
                    </div>

                    <div className="shrink-0 text-right tabular-nums">
                      <div className="text-[12px] font-medium">
                        {formatTokens(session.total_tokens)}{' '}
                        <span style={{ color: 'var(--text-muted)' }}>
                          ({formatCost(session.estimated_cost_micro_usd, currency, usdToCnyRate)})
                        </span>
                      </div>
                      <div className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
                        {formatTps(session.latest_output_tps)} ·{' '}
                        {formatRelativeTime(session.last_active_at_ms)}
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          ) : (
            <div
              className="py-10 text-center text-[12px]"
              style={{
                borderTop: '1px solid var(--border-subtle)',
                color: 'var(--text-muted)',
              }}
            >
              等待活跃会话
            </div>
          )}
        </div>
      </section>

      {/* Today 24h Intraday Pulse */}
      <section
        className="pb-6"
        style={{ borderBottom: '1px solid var(--border-hairline)' }}
      >
        <div className="flex items-center justify-between gap-3 mb-2.5">
          <div className="flex items-center gap-2">
            <h2 className="text-[13.5px] font-semibold">今日 24h 分时活跃度</h2>
            <span
              className="px-1.5 py-0.5 text-[10.5px] rounded-md tabular-nums"
              style={{
                backgroundColor: 'var(--bg-sidebar)',
                border: '1px solid var(--border-hairline)',
                color: 'var(--text-muted)',
              }}
            >
              按小时
            </span>
          </div>
          <button
            type="button"
            onClick={() => setActivePage('trends')}
            className="btn-interactive px-2 py-0.5 rounded-md text-[11px] cursor-pointer whitespace-nowrap"
            style={{ color: 'var(--accent-primary)' }}
          >
            跨日趋势对比 →
          </button>
        </div>

        {hasPulseData ? (
          <EChartBox option={pulseChartOption} height={148} />
        ) : (
          <div
            className="h-[140px] flex flex-col items-center justify-center text-[12px] gap-2"
            style={{ color: 'var(--text-muted)' }}
          >
            <span>等待今日分时脉冲</span>
            <button
              type="button"
              onClick={() => setActivePage('sources')}
              className="btn-interactive px-2.5 py-1 rounded-lg text-[11.5px] underline cursor-pointer"
              style={{ color: 'var(--accent-primary)' }}
            >
              前往数据源与合规
            </button>
          </div>
        )}
      </section>

      {/* Bottom Flat Provider Summary Table */}
      <section>
        <div className="flex items-center justify-between mb-3">
          <h2 className="text-[13.5px] font-semibold">模型提供商运行与消耗</h2>
        </div>

        <div className="overflow-x-auto">
          <table className="workspace-table w-full border-collapse text-left">
            <thead>
              <tr
                className="text-[11px] font-medium uppercase tracking-wider"
                style={{
                  color: 'var(--text-muted)',
                  borderBottom: '1px solid var(--border-hairline)',
                }}
              >
                <th className="py-2 pr-4">模型提供商</th>
                <th className="py-2 px-3 text-right">今日用量 / 费用</th>
                <th className="py-2 px-3 text-right">本月用量 / 费用</th>
                <th className="py-2 px-3 text-right">缓存命中率</th>
                <th className="py-2 px-3 w-[220px]">配额窗口</th>
                <th className="py-2 pl-3 pr-5 text-right">最近活跃</th>
              </tr>
            </thead>
            <tbody>
              {(data?.providers ?? []).map((row) => (
                <tr
                  key={row.provider_id}
                  className="row-interactive"
                  style={{ borderBottom: '1px solid var(--border-hairline)' }}
                >
                  <td className="py-3 pr-4">
                    <div className="flex items-center gap-2">
                      <span
                        className="inline-block w-2 h-2 rounded-full shrink-0"
                        style={{ backgroundColor: providerAccentColor(row.provider_id) }}
                      />
                      <span className="font-medium">{providerDisplayName(row.provider_id)}</span>
                      {!row.enabled && (
                        <span
                          className="px-1.5 py-0.5 text-[10px] rounded-md"
                          style={{
                            backgroundColor: 'var(--bg-hover)',
                            color: 'var(--text-muted)',
                          }}
                        >
                          已停用
                        </span>
                      )}
                      {row.observable_steps > 0 && (
                        <span
                          className="px-1.5 py-0.5 text-[10px] rounded-md tabular-nums"
                          style={{
                            backgroundColor: 'var(--bg-hover)',
                            color: 'var(--text-secondary)',
                          }}
                          title="基于本地公开日志记录的可观测响应步数"
                        >
                          +{row.observable_steps} 步追踪
                        </span>
                      )}
                    </div>
                  </td>
                  <td className="py-3 px-3 text-right tabular-nums">
                    <div>{formatTokens(row.today_tokens)}</div>
                    <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                      {formatCost(row.today_cost_micro_usd, currency, usdToCnyRate)}
                    </div>
                  </td>
                  <td className="py-3 px-3 text-right tabular-nums">
                    <div>{formatTokens(row.month_tokens)}</div>
                    <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                      {formatCost(row.month_cost_micro_usd, currency, usdToCnyRate)}
                    </div>
                  </td>
                  <td className="py-3 px-3 text-right tabular-nums">
                    {formatPercent(row.cache_hit_ratio, true)}
                  </td>
                  <td className="py-3 px-3">
                    {row.active_window_used_percent !== null ? (
                      <div className="space-y-1">
                        <div className="flex items-center justify-between text-[11px] tabular-nums">
                          <span style={{ color: 'var(--text-secondary)' }}>
                            {row.active_window_label ?? '滚动窗口'}
                          </span>
                          <span>已用 {row.active_window_used_percent.toFixed(1)}%</span>
                        </div>
                        <InlineProgressBar percent={row.active_window_used_percent} heightPx={4} />
                      </div>
                    ) : (
                      <span className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
                        按实测消耗统计
                      </span>
                    )}
                  </td>
                  <td
                    className="py-3 pl-3 pr-5 text-right text-[12px] tabular-nums"
                    style={{ color: 'var(--text-secondary)' }}
                  >
                    <div>{formatRelativeTime(row.last_sync_at_ms)}</div>
                    {row.last_error && (
                      <div
                        className="text-[10.5px] truncate max-w-[160px]"
                        style={{ color: 'var(--accent-rose)' }}
                        title={row.last_error}
                      >
                        {row.last_error}
                      </div>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}
