import { useEffect } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ExternalLink, PictureInPicture2, RefreshCw } from 'lucide-react';
import { InlineProgressBar } from '../components/InlineProgressBar';
import { LiveSpeedBadge } from '../components/LiveSpeedBadge';
import {
  formatCost,
  formatCountdown,
  formatRelativeTime,
  formatSessionDisplayName,
  formatTokens,
  formatTps,
  providerAccentColor,
  providerDisplayName,
} from '../lib/format';
import { ipc } from '../lib/ipc';
import { TRAY_SUMMARY_QUERY_KEY } from '../lib/queryKeys';
import { syncDocumentAndNativeTheme, useWorkspaceStore } from '../stores/workspaceStore';
import type { ThemeMode } from '../types/ipc';
import { CLOCK_REFRESH_MS } from '../lib/liveRefresh';

function normalizeThemeMode(raw: string | undefined | null): ThemeMode {
  return raw === 'light' || raw === 'dark' || raw === 'system' ? raw : 'dark';
}

export function TrayPopupPage() {
  const queryClient = useQueryClient();
  const {
    tzOffsetMinutes,
    currency: storeCurrency,
    usdToCnyRate: storeRate,
    setTheme,
    setCurrency,
    setUsdToCnyRate,
  } = useWorkspaceStore();

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
    refetchInterval: CLOCK_REFRESH_MS,
  });

  const scanMutation = useMutation({
    mutationFn: () => ipc.triggerScanNow(),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [TRAY_SUMMARY_QUERY_KEY] });
    },
  });

  const currency = prefs?.currency ?? summary?.currency ?? storeCurrency;
  const rate = prefs?.usd_to_cny_rate ?? summary?.usd_to_cny_rate ?? storeRate;

  return (
    <div
      className="h-screen w-screen flex flex-col justify-between p-3.5 select-none"
      style={{
        backgroundColor: 'var(--bg-canvas)',
        backgroundImage: 'var(--ambient-mesh)',
        color: 'var(--text-primary)',
        border: '1px solid var(--border-strong)',
        boxShadow: 'var(--border-specular)',
      }}
    >
      <div className="space-y-3 overflow-y-auto pr-0.5">
        {/* Compact Header */}
        <div
          className="flex items-center justify-between pb-2"
          style={{ borderBottom: '1px solid var(--border-subtle)' }}
        >
          <div className="flex items-center gap-2">
            <span
              className="inline-block w-2 h-2 rounded-full collector-dot-live"
              style={{ backgroundColor: 'var(--accent-emerald)' }}
            />
            <span className="text-[12.5px] font-semibold">TokenVane 实时摘要</span>
          </div>
          <div className="flex items-center gap-2">
            <LiveSpeedBadge
              activeSessions={summary?.active_sessions}
              liveModelSpeeds={summary?.live_model_speeds}
              align="right"
              placement="bottom"
              className="cursor-default"
            >
              {(breakdown) => (
                <span
                  className="text-[11px] font-medium tabular-nums"
                  style={{ color: 'var(--accent-sky)' }}
                >
                  {formatTps(breakdown.primaryTps)}
                </span>
              )}
            </LiveSpeedBadge>
            <button
              type="button"
              aria-label="切换桌面悬浮窗"
              title="开启 / 关闭实时桌面悬浮窗"
              onClick={() => void ipc.toggleFloatingWindow()}
              className="btn-interactive p-1 rounded-lg cursor-pointer"
              style={{ color: 'var(--text-secondary)' }}
            >
              <PictureInPicture2 className="w-3.5 h-3.5" />
            </button>
          </div>
        </div>

        {/* Today Metrics Grid + Provider Share Strip (Flat, Hairline Divided, No Shadow Cards) */}
        <div
          className="space-y-2.5 pb-3 tabular-nums"
          style={{ borderBottom: '1px solid var(--border-subtle)' }}
        >
          <div className="grid grid-cols-2 gap-3">
            <div>
              <div className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
                今日等效金额
              </div>
              <div className="text-[18px] font-semibold tracking-tight">
                {formatCost(summary?.today_cost_micro_usd ?? 0, currency, rate)}
              </div>
            </div>
            <div>
              <div className="text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
                今日总 Token
              </div>
              <div className="text-[18px] font-semibold tracking-tight">
                {formatTokens(summary?.today_tokens ?? 0)}
              </div>
            </div>
            <div className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
              今日请求：{summary?.today_requests ?? 0} 次
            </div>
            <div className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
              启用数据源：{summary?.active_providers_count ?? 0} 个
            </div>
          </div>

          {(summary?.provider_shares ?? []).length > 0 && (
            <div className="space-y-1.5 pt-0.5">
              <div
                className="h-1.5 w-full rounded-sm overflow-hidden flex"
                style={{ backgroundColor: 'var(--bg-hover)' }}
              >
                {(summary?.provider_shares ?? []).map((share) => (
                  <div
                    key={share.key}
                    style={{
                      width: `${Math.max(2, share.token_share_percent)}%`,
                      backgroundColor: providerAccentColor(share.key),
                    }}
                    title={`${providerDisplayName(share.key)} ${share.token_share_percent.toFixed(1)}%`}
                  />
                ))}
              </div>
              <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[10.5px]">
                {(summary?.provider_shares ?? []).slice(0, 4).map((share) => (
                  <span key={share.key} className="inline-flex items-center gap-1">
                    <span
                      className="inline-block w-1.5 h-1.5 rounded-full"
                      style={{ backgroundColor: providerAccentColor(share.key) }}
                    />
                    <span style={{ color: 'var(--text-secondary)' }}>
                      {providerDisplayName(share.key)}
                    </span>
                    <span style={{ color: 'var(--text-muted)' }}>
                      {share.token_share_percent.toFixed(0)}%
                    </span>
                  </span>
                ))}
              </div>
            </div>
          )}
        </div>

        {/* Active / Recent Sessions (Top 3, click opens Main Workspace) */}
        <div
          className="space-y-1.5 pb-2.5"
          style={{ borderBottom: '1px solid var(--border-subtle)' }}
        >
          <div
            className="text-[10.5px] font-semibold uppercase tracking-wider"
            style={{ color: 'var(--text-muted)' }}
          >
            活跃会话（15m）
          </div>
          {(summary?.active_sessions ?? []).slice(0, 3).map((session) => {
            const isLive = session.activity_state === 'active';
            return (
              <button
                key={`${session.provider_id}-${session.session_key}`}
                type="button"
                onClick={() => {
                  void ipc.openMainWorkspace('overview', session.provider_id);
                }}
                className="row-interactive w-full text-left px-1.5 py-1 rounded-lg flex items-center justify-between gap-2 cursor-pointer"
                title="点击打开主工作台查看该模型提供商实时用量"
              >
                <div className="min-w-0 flex items-center gap-1.5">
                  <span
                    className={`inline-block w-1.5 h-1.5 rounded-full shrink-0 ${isLive ? 'collector-dot-live' : ''}`}
                    style={{
                      backgroundColor: isLive ? 'var(--accent-emerald)' : 'var(--text-muted)',
                    }}
                  />
                  <div className="min-w-0">
                    <div className="text-[11px] font-medium truncate" title={formatSessionDisplayName(session)}>
                      {formatSessionDisplayName(session)}
                    </div>
                    <div className="text-[10px] truncate" style={{ color: 'var(--text-muted)' }} title={session.model_id}>
                      {providerDisplayName(session.provider_id)} · {session.model_id} ·{' '}
                      {formatRelativeTime(session.last_active_at_ms)}
                    </div>
                  </div>
                </div>
                <div className="shrink-0 text-right tabular-nums">
                  <div className="text-[11px] font-medium">
                    {formatTokens(session.total_tokens)}
                  </div>
                  <div className="text-[10px]" style={{ color: 'var(--text-muted)' }}>
                    {formatCost(session.estimated_cost_micro_usd, currency, rate)}
                  </div>
                </div>
              </button>
            );
          })}
          {(summary?.active_sessions ?? []).length === 0 && (
            <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
              等待活跃会话
            </div>
          )}
        </div>

        {/* Active Subscription Windows */}
        <div className="space-y-2.5">
          <div
            className="text-[10.5px] font-semibold uppercase tracking-wider"
            style={{ color: 'var(--text-muted)' }}
          >
            配额窗口
          </div>
          {(() => {
            const officialSubs = (summary?.subscriptions ?? [])
              .filter((sub) => sub.windows.length > 0)
              .slice(0, 3);
            if (officialSubs.length === 0) {
              return (
                <div className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                  等待官方配额同步
                </div>
              );
            }
            return officialSubs.map((sub) => {
              const win = sub.windows[0];
              if (!win) return null;
              return (
                <div key={sub.plan.plan_id} className="space-y-1">
                  <div className="flex items-center justify-between text-[11.5px] tabular-nums">
                    <span className="truncate max-w-[190px]" style={{ color: 'var(--text-primary)' }}>
                      {providerDisplayName(sub.plan.provider_id)} · {win.label}
                    </span>
                    <span style={{ color: 'var(--text-secondary)' }}>
                      已用 {win.used_percent.toFixed(1)}% · {formatCountdown(win.resets_in_seconds)}
                    </span>
                  </div>
                  <InlineProgressBar
                    percent={win.used_percent}
                    alertLevel={win.alert_level}
                    heightPx={4}
                  />
                </div>
              );
            });
          })()}
        </div>
      </div>

      {/* Bottom Actions */}
      <div
        className="pt-2.5 flex items-center justify-between gap-2"
        style={{ borderTop: '1px solid var(--border-subtle)' }}
      >
        <button
          type="button"
          onClick={() => scanMutation.mutate()}
          disabled={scanMutation.isPending}
          className="btn-interactive flex-1 inline-flex items-center justify-center gap-1.5 py-1.5 rounded-lg text-[11.5px] cursor-pointer disabled:opacity-50"
          style={{
            backgroundColor: 'var(--bg-elevated)',
            color: 'var(--text-primary)',
            border: '1px solid var(--border-subtle)',
          }}
        >
          <RefreshCw className={`w-3 h-3 ${scanMutation.isPending ? 'animate-spin' : ''}`} />
          <span>立即扫描</span>
        </button>

        <button
          type="button"
          onClick={() => {
            void ipc.openMainWorkspace();
          }}
          className="btn-primary flex-1 inline-flex items-center justify-center gap-1.5 py-1.5 rounded-lg text-[11.5px] font-medium cursor-pointer"
          style={{
            backgroundColor: 'var(--accent-primary)',
            color: '#ffffff',
          }}
        >
          <ExternalLink className="w-3 h-3" />
          <span>打开工作台</span>
        </button>
      </div>
    </div>
  );
}
