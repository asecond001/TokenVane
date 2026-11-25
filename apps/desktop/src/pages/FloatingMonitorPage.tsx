import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  ChevronDown,
  ChevronUp,
  ExternalLink,
  Maximize2,
  Minimize2,
  SlidersHorizontal,
  X,
} from 'lucide-react';
import { AnimatedNumber } from '../components/AnimatedNumber';
import { InlineProgressBar } from '../components/InlineProgressBar';
import { LiveSpeedBadge } from '../components/LiveSpeedBadge';
import {
  FLOATING_MODULES,
  computeFloatingDimensions,
  floatingModuleMeta,
  floatingPanelColumns,
  isModuleEnabled,
  moveFloatingModule,
  toggleFloatingModule,
  useFloatingConfig,
  type FloatingModuleId,
  type FloatingWindowConfig,
} from '../lib/floatingConfig';
import {
  formatCost,
  formatCountdown,
  formatExactInt,
  formatPercent,
  formatSessionDisplayName,
  formatTokens,
  formatTps,
  providerAccentColor,
  providerDisplayName,
} from '../lib/format';
import { ipc, isTauriRuntime } from '../lib/ipc';
import { TRAY_SUMMARY_QUERY_KEY } from '../lib/queryKeys';
import { syncDocumentAndNativeTheme, useWorkspaceStore } from '../stores/workspaceStore';
import type { AnalyticsFilter, ThemeMode, TraySummary } from '../types/ipc';
import { LIVE_CLOCK_REFRESH_MS } from '../lib/liveRefresh';

function normalizeThemeMode(raw: string | undefined | null): ThemeMode {
  return raw === 'light' || raw === 'dark' || raw === 'system' ? raw : 'dark';
}

/** Today-overview refresh for the heavier modules (pulse / models / cache). */
const OVERVIEW_REFRESH_MS = 30_000;
const HEADER_HEIGHT = 34;
const STAT_ROW_HEIGHT = 52;
const PANEL_TITLE_HEIGHT = 20;
const LIST_ROW_HEIGHT = 26;

export interface FloatingMonitorPageProps {
  livePulseSeq?: number;
}

type SubWindowRow = {
  key: string;
  providerId: string;
  win: TraySummary['subscriptions'][number]['windows'][number];
};

function flattenSubscriptionWindows(summary: TraySummary | undefined): SubWindowRow[] {
  const subs = summary?.subscriptions ?? [];
  const primary: SubWindowRow[] = [];
  const secondary: SubWindowRow[] = [];
  for (const sub of subs) {
    sub.windows.forEach((w, idx) => {
      const row = { key: `${sub.plan.plan_id}-${w.window_id}`, providerId: sub.plan.provider_id, win: w };
      (idx === 0 ? primary : secondary).push(row);
    });
  }
  return [...primary, ...secondary];
}

function todayFilter(tzOffsetMinutes: number): AnalyticsFilter {
  const now = new Date();
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  return {
    start_ms: start,
    end_ms: now.getTime(),
    provider_id: null,
    device_id: null,
    tz_offset_minutes: tzOffsetMinutes,
    all_time: false,
  };
}

/**
 * Always-on-top floating monitor. Content is composed from user-ordered
 * modules: metric "stats" in a strip at the top and "panels" in a grid that
 * gains columns and list rows as the window grows.
 */
export function FloatingMonitorPage({ livePulseSeq = 0 }: FloatingMonitorPageProps) {
  const [floatingConfig, updateFloatingConfig] = useFloatingConfig();
  const [customizing, setCustomizing] = useState(false);
  const [recentDeltaTokens, setRecentDeltaTokens] = useState<number>(0);
  const [burstKey, setBurstKey] = useState(0);
  const prevTodayTokensRef = useRef<number | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const [viewport, setViewport] = useState<{ width: number; height: number }>(() => ({
    width: typeof window !== 'undefined' && window.innerWidth > 0 ? window.innerWidth : 304,
    height: typeof window !== 'undefined' && window.innerHeight > 0 ? window.innerHeight : 248,
  }));

  const {
    tzOffsetMinutes,
    currency: storeCurrency,
    usdToCnyRate: storeRate,
    resolvedTheme,
    setTheme,
    setCurrency,
    setUsdToCnyRate,
  } = useWorkspaceStore();

  useEffect(() => {
    if (typeof document === 'undefined') return;
    document.documentElement.dataset.floatingView = 'true';
    return () => {
      delete document.documentElement.dataset.floatingView;
    };
  }, []);

  useEffect(() => {
    if (typeof window === 'undefined') return;
    const syncSize = () => {
      const w = rootRef.current?.clientWidth || window.innerWidth || 304;
      const h = rootRef.current?.clientHeight || window.innerHeight || 248;
      setViewport((prev) => (prev.width === w && prev.height === h ? prev : { width: w, height: h }));
    };
    syncSize();
    window.addEventListener('resize', syncSize);
    let observer: ResizeObserver | null = null;
    if (typeof ResizeObserver !== 'undefined' && rootRef.current) {
      observer = new ResizeObserver(() => syncSize());
      observer.observe(rootRef.current);
    }
    return () => {
      window.removeEventListener('resize', syncSize);
      observer?.disconnect();
    };
  }, []);

  const { data: prefs } = useQuery({
    queryKey: ['preferences'],
    queryFn: () => ipc.getPreferences(),
  });

  useEffect(() => {
    if (!prefs) return;
    const nextTheme = normalizeThemeMode(prefs.theme);
    setTheme(nextTheme);
    syncDocumentAndNativeTheme(nextTheme);
    setCurrency(prefs.currency);
    setUsdToCnyRate(prefs.usd_to_cny_rate);
  }, [prefs, setTheme, setCurrency, setUsdToCnyRate]);

  const { data: summary } = useQuery({
    queryKey: [TRAY_SUMMARY_QUERY_KEY, tzOffsetMinutes],
    queryFn: () => ipc.getTraySummary(tzOffsetMinutes),
    refetchInterval: LIVE_CLOCK_REFRESH_MS,
  });

  const needsOverview =
    !floatingConfig.compact &&
    floatingConfig.modules.some((id) => floatingModuleMeta(id).needsOverview);
  const { data: overview } = useQuery({
    queryKey: ['floating-overview', tzOffsetMinutes],
    queryFn: () => ipc.getOverview(todayFilter(tzOffsetMinutes)),
    enabled: needsOverview,
    refetchInterval: OVERVIEW_REFRESH_MS,
  });

  const todayTokens = summary?.today_tokens ?? 0;
  useEffect(() => {
    if (prevTodayTokensRef.current === null) {
      prevTodayTokensRef.current = todayTokens;
      return;
    }
    const delta = todayTokens - prevTodayTokensRef.current;
    prevTodayTokensRef.current = todayTokens;
    if (delta > 0) {
      setRecentDeltaTokens(delta);
      setBurstKey((k) => k + 1);
    }
  }, [todayTokens, livePulseSeq]);

  useEffect(() => {
    if (recentDeltaTokens <= 0) return;
    const timer = setTimeout(() => setRecentDeltaTokens(0), 2_600);
    return () => clearTimeout(timer);
  }, [recentDeltaTokens, burstKey]);

  const isCompactViewport = floatingConfig.compact || viewport.height <= 68;

  const resizeFor = (nextCfg: FloatingWindowConfig) => {
    if (!nextCfg.compact) {
      const dims = computeFloatingDimensions(nextCfg, viewport.width);
      void ipc.setFloatingWindowCompact(false, dims.width, dims.height);
    }
  };

  const handleToggleCompact = () => {
    const nextCompact = !floatingConfig.compact;
    setCustomizing(false);
    const nextCfg = updateFloatingConfig({ compact: nextCompact });
    if (nextCompact) {
      void ipc.setFloatingWindowCompact(true);
    } else {
      resizeFor(nextCfg);
    }
  };

  const handleToggleModule = (id: FloatingModuleId) => {
    resizeFor(updateFloatingConfig({ modules: toggleFloatingModule(floatingConfig.modules, id) }));
  };

  const handleMoveModule = (id: FloatingModuleId, delta: -1 | 1) => {
    updateFloatingConfig({ modules: moveFloatingModule(floatingConfig.modules, id, delta) });
  };

  const handleClose = () => {
    void ipc.toggleFloatingWindow();
  };

  const handleDragStart = (e: React.MouseEvent) => {
    if (e.button !== 0 || !isTauriRuntime()) return;
    void getCurrentWindow().startDragging();
  };

  const handleResizeDragStart = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (e.button !== 0 || !isTauriRuntime()) return;
    const win = getCurrentWindow() as unknown as {
      startResizeDragging?: (direction: string) => Promise<void>;
    };
    if (typeof win.startResizeDragging === 'function') {
      void win.startResizeDragging('SouthEast');
    }
  };

  const currency = prefs?.currency ?? summary?.currency ?? storeCurrency;
  const rate = prefs?.usd_to_cny_rate ?? summary?.usd_to_cny_rate ?? storeRate;
  const costFormatter = (v: number) => formatCost(v, currency, rate);

  const subWindows = useMemo(() => flattenSubscriptionWindows(summary), [summary]);

  const isLight = resolvedTheme === 'light';
  const alpha = floatingConfig.opacity;
  const lowAlpha = Math.max(0.5, alpha - 0.1);
  const surfaceGradient = isLight
    ? `linear-gradient(155deg, rgba(255, 255, 255, ${alpha}) 0%, rgba(242, 245, 252, ${lowAlpha}) 100%)`
    : `linear-gradient(155deg, rgba(26, 30, 44, ${alpha}) 0%, rgba(13, 16, 25, ${lowAlpha}) 100%)`;
  const borderColor = isLight ? 'rgba(15, 23, 42, 0.14)' : 'rgba(255, 255, 255, 0.14)';
  const specularRim = isLight
    ? 'inset 0 1px 0 rgba(255, 255, 255, 0.92), 0 12px 32px -10px rgba(15, 23, 42, 0.22)'
    : 'inset 0 1px 0 rgba(255, 255, 255, 0.18), 0 14px 36px -10px rgba(0, 0, 0, 0.65)';

  const burstBadge = (className: string) =>
    recentDeltaTokens > 0 ? (
      <span
        key={burstKey}
        className={`hud-burst-badge font-medium tabular-nums shrink-0 ${className}`}
        style={{ color: 'var(--accent-emerald)' }}
      >
        +{formatTokens(recentDeltaTokens)}
      </span>
    ) : null;

  if (isCompactViewport) {
    return (
      <div
        ref={rootRef}
        data-theme={resolvedTheme}
        className="h-screen w-screen flex items-center justify-between gap-2 px-3.5 select-none overflow-visible rounded-full"
        style={{
          backgroundImage: surfaceGradient,
          backdropFilter: 'blur(24px) saturate(180%)',
          WebkitBackdropFilter: 'blur(24px) saturate(180%)',
          color: 'var(--text-primary)',
          border: `1px solid ${borderColor}`,
          boxShadow: specularRim,
        }}
      >
        <div
          data-tauri-drag-region
          onMouseDown={handleDragStart}
          className="flex items-center gap-2 min-w-0 flex-1 h-full cursor-move"
          title="拖动悬浮窗"
        >
          <span
            className={`inline-block w-2 h-2 rounded-full shrink-0 ${
              recentDeltaTokens > 0 ? 'live-pulse-dot' : 'collector-dot-live'
            }`}
            style={{ backgroundColor: 'var(--accent-emerald)' }}
          />
          {isModuleEnabled(floatingConfig, 'tokens') && (
            <span className="text-[13px] font-semibold tabular-nums truncate">
              <AnimatedNumber value={todayTokens} formatter={formatTokens} />
            </span>
          )}
          {isModuleEnabled(floatingConfig, 'cost') && viewport.width >= 190 && (
            <span className="text-[11px] tabular-nums shrink-0" style={{ color: 'var(--text-secondary)' }}>
              <AnimatedNumber value={summary?.today_cost_micro_usd ?? 0} formatter={costFormatter} />
            </span>
          )}
          {isModuleEnabled(floatingConfig, 'speed') && viewport.width >= 235 && (
            <LiveSpeedBadge
              activeSessions={summary?.active_sessions}
              liveModelSpeeds={summary?.live_model_speeds}
              align="left"
              placement="bottom"
              className="shrink-0"
            >
              {(breakdown) => (
                <span className="text-[10.5px] tabular-nums cursor-default" style={{ color: 'var(--accent-sky)' }}>
                  {formatTps(breakdown.primaryTps)}
                </span>
              )}
            </LiveSpeedBadge>
          )}
          {burstBadge('text-[10px]')}
        </div>
        <div className="flex items-center gap-0.5 shrink-0">
          <IconButton label="展开悬浮窗" onClick={handleToggleCompact}>
            <Maximize2 className="w-3 h-3" />
          </IconButton>
          <IconButton label="关闭悬浮窗" onClick={handleClose}>
            <X className="w-3 h-3" />
          </IconButton>
        </div>
      </div>
    );
  }

  // ---- Expanded, modular layout ----
  const statIds = floatingConfig.modules.filter((id) => floatingModuleMeta(id).kind === 'stat');
  const panelIds = floatingConfig.modules.filter((id) => floatingModuleMeta(id).kind === 'panel');
  const isNarrow = viewport.width < 255;
  const isShort = viewport.height < 185;
  const statsPerRow = Math.max(1, Math.min(statIds.length, viewport.width >= 470 ? 4 : viewport.width >= 300 ? 3 : 2));
  const statRows = statIds.length > 0 ? Math.ceil(statIds.length / statsPerRow) : 0;
  const columns = Math.min(Math.max(1, panelIds.length), floatingPanelColumns(viewport.width));
  const panelRows = panelIds.length > 0 ? Math.ceil(panelIds.length / columns) : 0;
  const panelAreaHeight = viewport.height - HEADER_HEIGHT - statRows * STAT_ROW_HEIGHT - 20;
  const perPanelHeight = panelRows > 0 ? panelAreaHeight / panelRows : 0;
  const listRows = Math.max(1, Math.min(10, Math.floor((perPanelHeight - PANEL_TITLE_HEIGHT) / LIST_ROW_HEIGHT)));
  const heroFont = viewport.width >= 470 && viewport.height >= 300 ? 'text-[26px]' : isNarrow || isShort ? 'text-[17px]' : 'text-[20px]';

  const renderStat = (id: FloatingModuleId): ReactNode => {
    switch (id) {
      case 'tokens':
        return (
          <StatCell key={id} label="今日 Token">
            <div className="flex items-center gap-1.5 min-w-0">
              <span className={`${heroFont} font-semibold tracking-tight tabular-nums truncate`}>
                <AnimatedNumber value={todayTokens} formatter={formatTokens} />
              </span>
              {burstBadge('text-[10.5px] px-1 py-0.5 rounded-md')}
            </div>
          </StatCell>
        );
      case 'cost':
        return (
          <StatCell key={id} label="今日等效">
            <span className="text-[15px] font-semibold tabular-nums">
              <AnimatedNumber value={summary?.today_cost_micro_usd ?? 0} formatter={costFormatter} />
            </span>
          </StatCell>
        );
      case 'speed':
        return (
          <LiveSpeedBadge
            key={id}
            activeSessions={summary?.active_sessions}
            liveModelSpeeds={summary?.live_model_speeds}
            align="right"
            placement="bottom"
            className="min-w-0 tabular-nums cursor-default"
          >
            {(breakdown) => (
              <div className="min-w-0">
                <div className="text-[10.5px] truncate" style={{ color: 'var(--text-muted)' }}>
                  {breakdown.isMultiModel ? `实时生成速率 (${breakdown.items.length} 模型均值)` : '实时生成速率'}
                </div>
                <div
                  className="text-[15px] font-semibold"
                  style={{ color: breakdown.primaryTps !== null ? 'var(--accent-sky)' : 'var(--text-muted)' }}
                >
                  {breakdown.primaryTps !== null ? formatTps(breakdown.primaryTps) : '空闲'}
                </div>
              </div>
            )}
          </LiveSpeedBadge>
        );
      case 'requests':
        return (
          <StatCell key={id} label="今日请求">
            <span className="text-[15px] font-semibold tabular-nums">
              <AnimatedNumber value={summary?.today_requests ?? 0} formatter={formatExactInt} />
            </span>
          </StatCell>
        );
      case 'cache':
        return (
          <StatCell key={id} label="缓存命中">
            <span className="text-[15px] font-semibold tabular-nums" style={{ color: 'var(--accent-emerald)' }}>
              {overview?.today_cache_hit_ratio !== undefined
                ? formatPercent(overview.today_cache_hit_ratio, true)
                : '—'}
            </span>
          </StatCell>
        );
      default:
        return null;
    }
  };

  const renderPanel = (id: FloatingModuleId): ReactNode => {
    switch (id) {
      case 'sessions': {
        const sessions = (summary?.active_sessions ?? []).slice(0, listRows);
        return (
          <Panel key={id} title="活跃会话">
            {sessions.length > 0 ? (
              sessions.map((sess) => (
                <button
                  key={`${sess.provider_id}-${sess.session_key}`}
                  type="button"
                  onClick={() => void ipc.openMainWorkspace('overview', sess.provider_id)}
                  className="row-interactive w-full text-left px-1 py-0.5 rounded-lg flex items-center justify-between gap-2 cursor-pointer"
                >
                  <div className="min-w-0 flex-1">
                    <div className="text-[11px] font-medium truncate" title={formatSessionDisplayName(sess)}>{formatSessionDisplayName(sess)}</div>
                    {!isShort && (
                      <div
                        className="text-[10px] truncate"
                        style={{ color: 'var(--text-muted)' }}
                        title={`${providerDisplayName(sess.provider_id)} · ${sess.model_id}`}
                      >
                        {providerDisplayName(sess.provider_id)} · {sess.model_id}
                      </div>
                    )}
                  </div>
                  <span className="text-[10.5px] tabular-nums shrink-0" style={{ color: 'var(--text-secondary)' }}>
                    {formatTokens(sess.total_tokens)}
                  </span>
                </button>
              ))
            ) : (
              <PanelHint>等待活跃会话</PanelHint>
            )}
          </Panel>
        );
      }
      case 'quotas': {
        const rows = subWindows.slice(0, listRows);
        return (
          <Panel key={id} title="配额窗口">
            {rows.length > 0 ? (
              rows.map(({ key, providerId, win }) => (
                <div key={key} className="space-y-0.5">
                  <div className="flex items-center justify-between gap-2 text-[10.5px] tabular-nums">
                    <span className="truncate" style={{ color: 'var(--text-secondary)' }}>
                      {providerDisplayName(providerId)} · {win.label}
                    </span>
                    <span className="shrink-0" style={{ color: 'var(--text-primary)' }}>
                      已用 {win.used_percent.toFixed(0)}% · {formatCountdown(win.resets_in_seconds)}
                    </span>
                  </div>
                  <InlineProgressBar percent={win.used_percent} alertLevel={win.alert_level} heightPx={3} />
                </div>
              ))
            ) : (
              <PanelHint>等待官方配额同步</PanelHint>
            )}
          </Panel>
        );
      }
      case 'pulse': {
        const points = overview?.today_hourly_pulse ?? [];
        const values = points.map(
          (p) => p.input_tokens + p.output_tokens + p.cache_read_tokens + p.cache_write_tokens + p.reasoning_tokens
        );
        const max = Math.max(1, ...values);
        const barHeight = Math.max(18, Math.min(120, perPanelHeight - PANEL_TITLE_HEIGHT - 16));
        const nowHour = new Date().getHours();
        return (
          <Panel key={id} title="今日脉搏">
            {values.some((v) => v > 0) ? (
              <div data-testid="floating-pulse" className="flex items-end gap-[2px]" style={{ height: barHeight }}>
                {values.map((v, idx) => {
                  const hour = new Date(points[idx].bucket_start_ms).getHours();
                  return (
                    <div
                      key={points[idx].bucket_start_ms}
                      title={`${String(hour).padStart(2, '0')}:00 · ${formatTokens(v)}`}
                      className="flex-1 rounded-[2px]"
                      style={{
                        height: `${Math.max(v > 0 ? 6 : 2, (v / max) * 100)}%`,
                        backgroundColor: hour === nowHour ? 'var(--accent-emerald)' : 'var(--accent-sky)',
                        opacity: v > 0 ? 0.85 : 0.25,
                      }}
                    />
                  );
                })}
              </div>
            ) : (
              <PanelHint>等待今日首个请求</PanelHint>
            )}
          </Panel>
        );
      }
      case 'providers': {
        const shares = (summary?.provider_shares ?? []).filter((s) => s.tokens > 0).slice(0, listRows);
        return (
          <Panel key={id} title="提供商占比">
            {shares.length > 0 ? (
              shares.map((s) => (
                <ShareRow
                  key={s.key}
                  label={providerDisplayName(s.key)}
                  value={formatTokens(s.tokens)}
                  percent={s.token_share_percent}
                  color={providerAccentColor(s.key)}
                />
              ))
            ) : (
              <PanelHint>等待今日用量</PanelHint>
            )}
          </Panel>
        );
      }
      case 'models': {
        const shares = (overview?.top_model_shares ?? []).filter((s) => s.tokens > 0).slice(0, listRows);
        return (
          <Panel key={id} title="主力模型">
            {shares.length > 0 ? (
              shares.map((s) => (
                <ShareRow
                  key={s.key}
                  label={s.label || s.key}
                  value={formatCost(s.cost_micro_usd, currency, rate)}
                  percent={s.token_share_percent}
                  color="var(--accent-sky)"
                  mono
                />
              ))
            ) : (
              <PanelHint>等待今日用量</PanelHint>
            )}
          </Panel>
        );
      }
      default:
        return null;
    }
  };

  return (
    <div
      ref={rootRef}
      data-theme={resolvedTheme}
      className={`relative h-screen w-screen flex flex-col ${isShort || isNarrow ? 'p-2.5' : 'p-3'} select-none overflow-hidden rounded-2xl`}
      style={{
        backgroundImage: surfaceGradient,
        backdropFilter: 'blur(24px) saturate(180%)',
        WebkitBackdropFilter: 'blur(24px) saturate(180%)',
        color: 'var(--text-primary)',
        border: `1px solid ${borderColor}`,
        boxShadow: specularRim,
      }}
    >
      {/* Top Drag Header */}
      <div className="flex items-center justify-between pb-1.5 shrink-0" style={{ borderBottom: '1px solid var(--border-hairline)' }}>
        <div
          data-tauri-drag-region
          onMouseDown={handleDragStart}
          className="flex items-center gap-1.5 flex-1 min-w-0 cursor-move"
          title="拖动悬浮窗"
        >
          <span
            className={`inline-block w-2 h-2 rounded-full shrink-0 ${
              recentDeltaTokens > 0 ? 'live-pulse-dot' : 'collector-dot-live'
            }`}
            style={{ backgroundColor: 'var(--accent-emerald)' }}
          />
          <span className="text-[11.5px] font-semibold tracking-tight truncate">TokenVane</span>
        </div>
        <div className="flex items-center gap-0.5 shrink-0">
          <IconButton
            label="自定义悬浮窗"
            title="自定义悬浮窗模块"
            active={customizing}
            onClick={() => setCustomizing((prev) => !prev)}
          >
            <SlidersHorizontal className="w-3 h-3" />
          </IconButton>
          <IconButton label="切换精简胶囊模式" onClick={handleToggleCompact}>
            <Minimize2 className="w-3 h-3" />
          </IconButton>
          <IconButton label="打开工作台" title="打开主工作台" onClick={() => void ipc.openMainWorkspace('overview')}>
            <ExternalLink className="w-3 h-3" />
          </IconButton>
          <IconButton label="关闭悬浮窗" onClick={handleClose}>
            <X className="w-3 h-3" />
          </IconButton>
        </div>
      </div>

      {customizing ? (
        <div className="flex-1 py-2 space-y-2.5 overflow-y-auto text-[11px]" data-testid="floating-customize">
          <div className="space-y-1">
            <div style={{ color: 'var(--text-muted)' }}>模块</div>
            <ul className="space-y-0.5">
              {[
                ...floatingConfig.modules.map((id) => floatingModuleMeta(id)),
                ...FLOATING_MODULES.filter((m) => !floatingConfig.modules.includes(m.id)),
              ].map((meta) => {
                const enabled = floatingConfig.modules.includes(meta.id);
                const idx = floatingConfig.modules.indexOf(meta.id);
                return (
                  <li key={meta.id} className="flex items-center justify-between gap-2">
                    <label className="flex items-center gap-1.5 cursor-pointer min-w-0">
                      <input
                        type="checkbox"
                        checked={enabled}
                        onChange={() => handleToggleModule(meta.id)}
                        className="accent-[var(--accent-primary)]"
                      />
                      <span className="truncate" style={{ color: enabled ? 'var(--text-primary)' : 'var(--text-muted)' }}>
                        {meta.label}
                      </span>
                    </label>
                    {enabled && (
                      <span className="flex items-center gap-0.5 shrink-0">
                        <IconButton
                          label={`上移 ${meta.label}`}
                          disabled={idx <= 0}
                          onClick={() => handleMoveModule(meta.id, -1)}
                        >
                          <ChevronUp className="w-3 h-3" />
                        </IconButton>
                        <IconButton
                          label={`下移 ${meta.label}`}
                          disabled={idx >= floatingConfig.modules.length - 1}
                          onClick={() => handleMoveModule(meta.id, 1)}
                        >
                          <ChevronDown className="w-3 h-3" />
                        </IconButton>
                      </span>
                    )}
                  </li>
                );
              })}
            </ul>
          </div>

          <label className="block space-y-1">
            <div className="flex items-center justify-between">
              <span style={{ color: 'var(--text-muted)' }}>背景不透明度</span>
              <span className="tabular-nums">{Math.round(floatingConfig.opacity * 100)}%</span>
            </div>
            <input
              type="range"
              aria-label="悬浮窗不透明度"
              min={0.55}
              max={1}
              step={0.01}
              value={floatingConfig.opacity}
              onChange={(e) => {
                const val = Number.parseFloat(e.target.value);
                if (Number.isFinite(val)) {
                  updateFloatingConfig({ opacity: val });
                }
              }}
              className="w-full cursor-pointer accent-[var(--accent-primary)]"
            />
          </label>
        </div>
      ) : (
        <div className="flex-1 flex flex-col min-h-0">
          {statIds.length > 0 && (
            <div
              data-testid="floating-stats"
              className="grid gap-x-3 gap-y-1.5 py-1.5 shrink-0"
              style={{ gridTemplateColumns: `repeat(${statsPerRow}, minmax(0, 1fr))` }}
            >
              {statIds.map(renderStat)}
            </div>
          )}
          {panelIds.length > 0 && (
            <div
              data-testid="floating-panels"
              className="grid gap-x-3 flex-1 min-h-0"
              style={{
                gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
                gridAutoRows: 'minmax(0, 1fr)',
              }}
            >
              {panelIds.map(renderPanel)}
            </div>
          )}
        </div>
      )}

      {/* Corner Resize Grip */}
      <div
        onMouseDown={handleResizeDragStart}
        aria-label="调整悬浮窗大小"
        title="拖拽调整悬浮窗大小"
        className="absolute right-0.5 bottom-0.5 w-3 h-3 cursor-se-resize opacity-45 hover:opacity-90"
        style={{
          borderRight: '2px solid var(--text-muted)',
          borderBottom: '2px solid var(--text-muted)',
        }}
      />
    </div>
  );
}

function IconButton({
  label,
  title,
  active = false,
  disabled = false,
  onClick,
  children,
}: {
  label: string;
  title?: string;
  active?: boolean;
  disabled?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={title ?? label}
      data-active={active ? 'true' : undefined}
      disabled={disabled}
      onClick={onClick}
      className="btn-interactive p-1 rounded-lg cursor-pointer disabled:opacity-30 disabled:cursor-default"
      style={{ color: active ? 'var(--accent-primary)' : 'var(--text-secondary)' }}
    >
      {children}
    </button>
  );
}

function StatCell({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="min-w-0">
      <div className="text-[10.5px] truncate" style={{ color: 'var(--text-muted)' }}>
        {label}
      </div>
      {children}
    </div>
  );
}

function Panel({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section
      className="min-h-0 overflow-hidden pt-1.5 pb-1 space-y-1"
      style={{ borderTop: '1px solid var(--border-hairline)' }}
    >
      <div className="text-[10px] font-medium uppercase tracking-wider" style={{ color: 'var(--text-muted)' }}>
        {title}
      </div>
      {children}
    </section>
  );
}

function PanelHint({ children }: { children: ReactNode }) {
  return (
    <div className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
      {children}
    </div>
  );
}

function ShareRow({
  label,
  value,
  percent,
  color,
  mono = false,
}: {
  label: string;
  value: string;
  percent: number;
  color: string;
  mono?: boolean;
}) {
  return (
    <div className="space-y-0.5">
      <div className="flex items-center justify-between gap-2 text-[10.5px] tabular-nums">
        <span
          className={`truncate ${mono ? 'font-mono' : ''}`}
          style={{ color: 'var(--text-secondary)' }}
          title={label}
        >
          {label}
        </span>
        <span className="shrink-0" style={{ color: 'var(--text-primary)' }}>
          {percent.toFixed(0)}% · {value}
        </span>
      </div>
      <InlineProgressBar percent={percent} accentColor={color} heightPx={3} />
    </div>
  );
}
