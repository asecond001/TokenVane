import { useEffect, useState, type ReactNode } from 'react';
import { formatTps, providerAccentColor, providerDisplayName } from '../lib/format';
import type { ActiveSessionSummary, LiveModelSpeed } from '../types/ipc';

export interface LiveSpeedBreakdown {
  primaryTps: number | null;
  items: LiveModelSpeed[];
  isMultiModel: boolean;
}

/** A session counts as live only while its last request is this recent. */
export const LIVE_ACTIVE_WINDOW_MS = 120_000;

function isSessionLive(session: ActiveSessionSummary, nowMs: number): boolean {
  return (
    session.activity_state === 'active' &&
    nowMs - session.last_active_at_ms <= LIVE_ACTIVE_WINDOW_MS
  );
}

/**
 * Live generation speed derived from *currently active* sessions only. Once no
 * session has produced output within `LIVE_ACTIVE_WINDOW_MS` the readout is
 * empty, rather than freezing on the last observed value.
 */
export function computeLiveSpeedBreakdown(
  activeSessions?: ActiveSessionSummary[] | null,
  liveModelSpeeds?: LiveModelSpeed[] | null,
  nowMs: number = Date.now()
): LiveSpeedBreakdown {
  const sessions = Array.isArray(activeSessions) ? activeSessions : [];
  const liveSessions = sessions.filter((s) => isSessionLive(s, nowMs));
  const liveKeys = new Set(liveSessions.map((s) => `${s.provider_id}::${s.model_id}`));

  if (Array.isArray(liveModelSpeeds) && liveModelSpeeds.length > 0) {
    const valid = liveModelSpeeds.filter(
      (m) =>
        Number.isFinite(m.output_tps) &&
        m.output_tps > 0 &&
        m.activity_state === 'active' &&
        liveKeys.has(`${m.provider_id}::${m.model_id}`)
    );
    if (valid.length > 0) {
      const mean = valid.reduce((sum, item) => sum + item.output_tps, 0) / valid.length;
      return { primaryTps: mean, items: valid, isMultiModel: valid.length > 1 };
    }
  }

  const grouped = new Map<string, { provider_id: string; model_id: string; values: number[] }>();
  for (const sess of liveSessions) {
    const tps = sess.latest_output_tps;
    if (typeof tps !== 'number' || !Number.isFinite(tps) || tps <= 0) continue;
    const key = `${sess.provider_id}::${sess.model_id}`;
    const existing = grouped.get(key);
    if (existing) {
      existing.values.push(tps);
    } else {
      grouped.set(key, { provider_id: sess.provider_id, model_id: sess.model_id, values: [tps] });
    }
  }

  const items: LiveModelSpeed[] = Array.from(grouped.values()).map((entry) => ({
    provider_id: entry.provider_id,
    model_id: entry.model_id,
    output_tps: entry.values.reduce((a, b) => a + b, 0) / entry.values.length,
    activity_state: 'active',
  }));

  if (items.length === 0) {
    return { primaryTps: null, items: [], isMultiModel: false };
  }
  return {
    primaryTps: items.reduce((sum, item) => sum + item.output_tps, 0) / items.length,
    items,
    isMultiModel: items.length > 1,
  };
}

/** Re-renders every `intervalMs` so live readouts fade out on wall-clock time. */
export function useNowTick(intervalMs = 5_000): number {
  const [nowMs, setNowMs] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNowMs(Date.now()), intervalMs);
    return () => window.clearInterval(timer);
  }, [intervalMs]);
  return nowMs;
}

export interface LiveSpeedBadgeProps {
  activeSessions?: ActiveSessionSummary[] | null;
  liveModelSpeeds?: LiveModelSpeed[] | null;
  align?: 'left' | 'right';
  placement?: 'top' | 'bottom';
  className?: string;
  children?: (breakdown: LiveSpeedBreakdown) => ReactNode;
}

export function LiveSpeedBadge({
  activeSessions,
  liveModelSpeeds,
  align = 'right',
  placement = 'bottom',
  className = '',
  children,
}: LiveSpeedBadgeProps) {
  const [hovered, setHovered] = useState(false);
  const nowMs = useNowTick();
  const breakdown = computeLiveSpeedBreakdown(activeSessions, liveModelSpeeds, nowMs);

  const tooltipSummary =
    breakdown.items.length > 0
      ? breakdown.items
          .map(
            (item) =>
              `${providerDisplayName(item.provider_id)} · ${item.model_id}: ${formatTps(item.output_tps)}`
          )
          .join(' | ')
      : breakdown.primaryTps !== null
        ? `实时生成速率: ${formatTps(breakdown.primaryTps)}`
        : '等待活跃会话';

  const hasDetails = breakdown.items.length > 0 || breakdown.primaryTps !== null;

  return (
    <div
      className={`relative inline-block ${className}`}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={() => setHovered(true)}
      onBlur={() => setHovered(false)}
      title={tooltipSummary}
    >
      {children ? (
        children(breakdown)
      ) : (
        <span className="tabular-nums">{formatTps(breakdown.primaryTps)}</span>
      )}

      {hovered && hasDetails && (
        <div
          role="tooltip"
          data-testid="live-speed-tooltip"
          className={`z-50 w-max min-w-[205px] max-w-[min(440px,calc(100vw-24px))] px-2.5 py-2 rounded-lg text-left text-[11px] space-y-1.5 pointer-events-none select-none ${
            placement === 'top' ? 'bottom-full mb-1.5' : 'top-full mt-1.5'
          } ${align === 'right' ? 'right-0' : 'left-0'}`}
          style={{
            position: 'absolute',
            backgroundColor: 'var(--bg-sidebar)',
            border: '1px solid var(--border-strong)',
            color: 'var(--text-primary)',
            boxShadow: '0 8px 24px rgba(0, 0, 0, 0.28)',
          }}
        >
          <div
            className="flex items-center justify-between gap-2 pb-1 text-[10.5px]"
            style={{
              color: 'var(--text-muted)',
              borderBottom: '1px solid var(--border-hairline)',
            }}
          >
            <span>
              {breakdown.isMultiModel
                ? `活跃模型速率 (${breakdown.items.length} 模型均值)`
                : '实时速率来源模型'}
            </span>
            <span
              className="font-semibold tabular-nums"
              style={{ color: 'var(--accent-sky)' }}
            >
              {formatTps(breakdown.primaryTps)}
            </span>
          </div>

          {breakdown.items.length > 0 ? (
            <div className="space-y-1">
              {breakdown.items.map((item) => (
                <div
                  key={`${item.provider_id}::${item.model_id}`}
                  className="flex items-center justify-between gap-3 tabular-nums"
                >
                  <div className="flex items-center gap-1.5 min-w-0">
                    <span
                      className="inline-block w-1.5 h-1.5 rounded-full shrink-0"
                      style={{ backgroundColor: providerAccentColor(item.provider_id) }}
                    />
                    <span className="font-medium break-all" style={{ color: 'var(--text-primary)' }}>
                      {item.model_id}
                    </span>
                    <span
                      className="text-[10px] shrink-0"
                      style={{ color: 'var(--text-muted)' }}
                    >
                      {providerDisplayName(item.provider_id)}
                    </span>
                  </div>
                  <span
                    className="font-semibold shrink-0"
                    style={{ color: 'var(--accent-sky)' }}
                  >
                    {formatTps(item.output_tps)}
                  </span>
                </div>
              ))}
            </div>
          ) : (
            <div className="text-[10.5px]" style={{ color: 'var(--text-secondary)' }}>
              近期生成速率 {formatTps(breakdown.primaryTps)}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
