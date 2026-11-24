import type { CurrencyCode } from '../types/ipc';

export function formatTokens(tokens: number): string {
  if (!Number.isFinite(tokens)) return '0';
  const abs = Math.abs(tokens);
  if (abs >= 1_000_000_000) {
    return `${(tokens / 1_000_000_000).toFixed(2)}B`;
  }
  if (abs >= 1_000_000) {
    return `${(tokens / 1_000_000).toFixed(2)}M`;
  }
  if (abs >= 10_000) {
    return `${(tokens / 1_000).toFixed(1)}K`;
  }
  return Math.round(tokens).toLocaleString('en-US');
}

export function formatExactInt(n: number): string {
  if (!Number.isFinite(n)) return '0';
  return Math.round(n).toLocaleString('en-US');
}

export function convertMicroUsd(
  microUsd: number,
  currency: CurrencyCode,
  usdToCnyRateStr: string
): number {
  const usd = microUsd / 1_000_000;
  if (currency === 'USD') {
    return usd;
  }
  const rate = Number.parseFloat(usdToCnyRateStr);
  const safeRate = Number.isFinite(rate) && rate > 0 ? rate : 7.25;
  return usd * safeRate;
}

export function formatCost(
  microUsd: number,
  currency: CurrencyCode,
  usdToCnyRateStr: string
): string {
  const amount = convertMicroUsd(microUsd, currency, usdToCnyRateStr);
  const symbol = currency === 'CNY' ? '¥' : '$';
  if (Math.abs(amount) > 0 && Math.abs(amount) < 0.01) {
    return `${symbol}${amount.toFixed(4)}`;
  }
  return `${symbol}${amount.toLocaleString('en-US', {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  })}`;
}

export function formatPercent(ratioOrPercent: number, isRatio = false): string {
  const val = isRatio ? ratioOrPercent * 100 : ratioOrPercent;
  if (!Number.isFinite(val)) return '0.0%';
  return `${val.toFixed(1)}%`;
}

export function formatTps(tps: number | null | undefined): string {
  if (tps === null || tps === undefined || !Number.isFinite(tps) || tps <= 0) {
    return '—';
  }
  return `${tps.toFixed(1)} tok/s`;
}

export function formatDurationMs(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms) || ms <= 0) {
    return '—';
  }
  if (ms < 1000) {
    return `${Math.round(ms)} ms`;
  }
  return `${(ms / 1000).toFixed(2)} s`;
}

export function formatCountdown(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) {
    return '已重置 / 空闲';
  }
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (d > 0) {
    return `${d}d ${h}h`;
  }
  if (h > 0) {
    return `${h}h ${m}m`;
  }
  return `${m}m`;
}

export function formatRelativeTime(tsMs: number | null): string {
  if (!tsMs) return '待同步';
  const diffSec = Math.max(0, Math.floor((Date.now() - tsMs) / 1000));
  if (diffSec < 60) return `${diffSec}s 前`;
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m 前`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h 前`;
  return new Date(tsMs).toLocaleDateString('zh-CN');
}

export function providerDisplayName(providerId: string): string {
  switch (providerId) {
    case 'claude_code':
      return 'Claude Code';
    case 'codex':
      return 'Codex';
    case 'antigravity':
      return 'Antigravity';
    case 'kimi_code':
      return 'Kimi Code';
    case 'workbuddy':
      return 'Workbuddy';
    case 'deepseek_harness':
      return 'DeepSeek Harness';
    case 'qcode':
      return 'QCode';
    case 'minimax_code':
      return 'MiniMax Code';
    case 'cursor':
      return 'Cursor';
    case 'github_copilot':
      return 'GitHub Copilot';
    case 'windsurf':
      return 'Windsurf';
    default:
      if (providerId.startsWith('custom_')) {
        return `自定义 (${providerId.slice(7)})`;
      }
      if (providerId.includes('_')) {
        return providerId
          .split('_')
          .filter(Boolean)
          .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
          .join(' ');
      }
      return providerId;
  }
}

export function providerAccentColor(providerId: string): string {
  switch (providerId) {
    case 'claude_code':
      return '#f59e0b';
    case 'codex':
      return '#10b981';
    case 'antigravity':
      return '#6366f1';
    case 'kimi_code':
      return '#38bdf8';
    case 'workbuddy':
      return '#ec4899';
    case 'deepseek_harness':
      return '#3b82f6';
    case 'qcode':
      return '#2dd4bf';
    case 'minimax_code':
      return '#a855f7';
    case 'cursor':
      return '#22d3ee';
    case 'github_copilot':
      return '#6e7681';
    case 'windsurf':
      return '#06b6d4';
    default:
      return providerId.startsWith('custom_') ? '#8b5cf6' : '#60a5fa';
  }
}

export const TOKEN_TYPE_COLORS: Record<string, string> = {
  input: '#3b82f6',
  output: '#10b981',
  cache_read: '#8b5cf6',
  cache_write: '#f59e0b',
  reasoning: '#ec4899',
};

export function formatSessionDisplayName(session: {
  session_title?: string | null;
  project_label?: string | null;
  session_key: string;
  provider_id: string;
}): string {
  const title = session.session_title?.trim();
  if (title) {
    return title;
  }
  const project = session.project_label?.trim();
  if (project && project.toLowerCase() !== 'workspace') {
    return `${project} · ${session.session_key}`;
  }
  return `${providerDisplayName(session.provider_id)} ${session.session_key}`;
}

export function resolveSubscriptionTierLabel(
  plan: {
    plan_name: string;
    monthly_fee_micro_usd: number;
    auto_source?: string | null;
  },
  currency: CurrencyCode,
  usdToCnyRateStr: string,
  hasOfficialWindows = false
): string {
  if (plan.monthly_fee_micro_usd > 0) {
    return `月费 ${formatCost(plan.monthly_fee_micro_usd, currency, usdToCnyRateStr)}`;
  }
  if (/\bfree\b/i.test(plan.plan_name)) {
    return '免费档位';
  }
  if (plan.auto_source === 'official_api') {
    return '官方同步档位';
  }
  if (plan.auto_source === 'official_cli') {
    return '官方 CLI 同步';
  }
  if (plan.auto_source === 'local_log') {
    return '日志探测档位';
  }
  return hasOfficialWindows ? '官方配额窗口监测' : '实测消耗统计';
}

export function formatBucketInTz(
  tsMs: number,
  granularity: string,
  tzOffsetMinutes: number
): string {
  const shifted = new Date(tsMs + tzOffsetMinutes * 60_000);
  const pad2 = (n: number) => String(n).padStart(2, '0');
  const yyyy = shifted.getUTCFullYear();
  const mm = pad2(shifted.getUTCMonth() + 1);
  const dd = pad2(shifted.getUTCDate());
  const hh = pad2(shifted.getUTCHours());
  const min = pad2(shifted.getUTCMinutes());
  if (granularity === 'hour_only') {
    return `${hh}:${min}`;
  }
  if (granularity === 'fifteen_minutes' || granularity === 'hourly') {
    return `${mm}-${dd} ${hh}:${min}`;
  }
  if (granularity === 'weekly') {
    return `${yyyy}-${mm}-${dd} 周`;
  }
  return `${yyyy}-${mm}-${dd}`;
}
