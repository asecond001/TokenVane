import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ActionNotice } from '../components/ActionNotice';
import { createPortal } from 'react-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Check, ExternalLink, Info, Pencil, Plus, RefreshCw, Trash2, X } from 'lucide-react';
import { InlineProgressBar } from '../components/InlineProgressBar';
import {
  formatCost,
  formatCountdown,
  formatRelativeTime,
  formatTokens,
  providerAccentColor,
  providerDisplayName,
  resolveSubscriptionTierLabel,
} from '../lib/format';
import { ipc } from '../lib/ipc';
import { TRAY_SUMMARY_QUERY_KEY } from '../lib/queryKeys';
import { useWorkspaceStore } from '../stores/workspaceStore';
import type {
  CurrencyCode,
  ObservedRateLimitWindow,
  PaygAccount,
  PaygAccountStatus,
  RemoteSubscriptionSnapshot,
  SubscriptionPlanStatus,
  SubscriptionPlanTemplate,
} from '../types/ipc';
import { CLOCK_REFRESH_MS } from '../lib/liveRefresh';

const CURRENT_TIER_VALUE = '__current';
const CUSTOM_TIER_VALUE = '__custom';

const API_PROVIDERS: Array<{ id: string; label: string }> = [
  { id: 'deepseek_harness', label: 'DeepSeek Harness' },
  { id: 'kimi_code', label: 'Kimi Code' },
  { id: 'minimax_code', label: 'MiniMax Code' },
  { id: 'qcode', label: 'QCode' },
  { id: 'claude_code', label: 'Claude Code' },
  { id: 'codex', label: 'Codex' },
  { id: 'workbuddy', label: 'Workbuddy' },
  { id: 'cursor', label: 'Cursor' },
  { id: 'github_copilot', label: 'GitHub Copilot' },
  { id: 'windsurf', label: 'Windsurf' },
];

const LOGIN_PROVIDERS: Array<{ id: string; label: string }> = [
  { id: 'antigravity', label: 'Antigravity (Google AI Pro / Ultra)' },
  { id: 'codex', label: 'Codex (ChatGPT Plus / Pro)' },
  { id: 'claude_code', label: 'Claude Code (Claude Pro / Max)' },
];

const API_KEY_CAPABLE_PROVIDERS = new Set([
  'deepseek_harness',
  'kimi_code',
  'qcode',
  'minimax_code',
  'claude_code',
  'codex',
]);

const QUOTA_BALANCE_API_PROVIDERS = new Set(['antigravity', 'kimi_code', 'minimax_code', 'qcode']);

const CLI_QUOTA_LOGIN_PROVIDERS = new Set(['antigravity', 'claude_code']);

interface AccountFormDraft {
  mode: 'create' | 'edit';
  connectKind: 'login' | 'api';
  account_id: string;
  provider_id: string;
  account_name: string;
  enabled: boolean;
  is_auto: boolean;
  api_key_input: string;
  manual_balance_input: string;
  initial_manual_balance_micro_usd: number | null;
  existing_anchor_ms: number | null;
  monthly_budget_input: string;
}

function defaultAccountName(providerId: string): string {
  switch (providerId) {
    case 'deepseek_harness':
      return 'DeepSeek 官方账户';
    case 'codex':
      return 'Codex Credits';
    case 'claude_code':
      return 'Claude Console 账户';
    case 'kimi_code':
      return 'Kimi Code 账户';
    case 'qcode':
      return 'QCode 账户';
    case 'minimax_code':
      return 'MiniMax 账户';
    default:
      return `${providerDisplayName(providerId)} 账户`;
  }
}

function formatBalanceAmount(
  micro: number | null | undefined,
  balCurrency: string,
  displayCurrency: CurrencyCode,
  usdToCnyRate: string
): string {
  const upper = balCurrency.toUpperCase();
  if (upper === 'CREDITS_UNLIMITED') {
    return 'Unlimited Credits';
  }
  if (micro === null || micro === undefined || !Number.isFinite(micro)) {
    return '—';
  }
  if (upper === 'CREDITS') {
    return `${(micro / 1_000_000).toLocaleString('en-US', {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    })} Credits`;
  }
  if (upper === 'CNY') {
    const cny = micro / 1_000_000;
    return `¥${cny.toLocaleString('en-US', {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    })}`;
  }
  if (upper === 'USD') {
    return formatCost(micro, displayCurrency, usdToCnyRate);
  }
  return `${(micro / 1_000_000).toFixed(2)} ${balCurrency}`;
}

function getSafeExchangeRate(rateStr: string): number {
  const rate = Number.parseFloat(rateStr);
  return Number.isFinite(rate) && rate > 0 ? rate : 7.25;
}

function microUsdToCurrencyInput(
  microUsd: number,
  currency: CurrencyCode,
  usdToCnyRate: string
): string {
  if (!Number.isFinite(microUsd) || microUsd <= 0) {
    return '0';
  }
  const usd = microUsd / 1_000_000;
  const converted = currency === 'CNY' ? usd * getSafeExchangeRate(usdToCnyRate) : usd;
  return Number(converted.toFixed(2)).toString();
}

function currencyInputToMicroUsd(
  input: string,
  currency: CurrencyCode,
  usdToCnyRate: string
): number | null {
  const trimmed = input.trim();
  if (!trimmed) {
    return 0;
  }
  const parsed = Number(trimmed);
  if (!Number.isFinite(parsed) || parsed < 0) {
    return null;
  }
  const usd = currency === 'CNY' ? parsed / getSafeExchangeRate(usdToCnyRate) : parsed;
  return Math.max(0, Math.round(usd * 1_000_000));
}

function formatObservedWindowLabel(obs: ObservedRateLimitWindow): string {
  const trimmed = obs.window_name.trim();
  if (trimmed.includes(' · ')) {
    return trimmed;
  }
  const lower = trimmed.toLowerCase();
  let durLabel: string;
  if (obs.window_minutes !== null && Number.isFinite(obs.window_minutes) && obs.window_minutes > 0) {
    if (obs.window_minutes % 1440 === 0) {
      durLabel = `${obs.window_minutes / 1440}d 官方限额`;
    } else if (obs.window_minutes % 60 === 0) {
      durLabel = `${obs.window_minutes / 60}h 官方限额`;
    } else {
      durLabel = `${obs.window_minutes}m 官方限额`;
    }
  } else if (lower.includes('primary') || lower.includes('5h')) {
    durLabel = '5h 官方限额';
  } else if (lower.includes('secondary') || lower.includes('7d') || lower.includes('week')) {
    durLabel = '7d 官方限额';
  } else if (lower.includes('30d') || lower.includes('month')) {
    durLabel = '30d 官方限额';
  } else {
    durLabel = `${obs.window_name} 官方限额`;
  }

  if (lower.startsWith('gemini-') || lower.includes('gemini')) {
    return `Gemini 模型池 · ${durLabel}`;
  }
  if (
    lower.startsWith('3p-') ||
    lower.includes('claude') ||
    lower.includes('gpt') ||
    lower.includes('third')
  ) {
    return `Claude / GPT 模型池 · ${durLabel}`;
  }
  return durLabel;
}

function observedWindowSortKey(obs: ObservedRateLimitWindow): number {
  if (obs.window_minutes !== null && Number.isFinite(obs.window_minutes)) {
    return obs.window_minutes;
  }
  const lower = obs.window_name.toLowerCase();
  if (lower.includes('primary') || lower.includes('5h')) return 300;
  if (lower.includes('secondary') || lower.includes('7d')) return 10080;
  if (lower.includes('30d') || lower.includes('month')) return 43200;
  return 999999;
}

function windowSourceBadge(source: string): { label: string; color: string } {
  if (source === 'official_api') {
    return { label: '官方 API 实时', color: 'var(--accent-emerald)' };
  }
  if (source === 'official_cli') {
    return { label: '官方 CLI 实时', color: 'var(--accent-emerald)' };
  }
  return { label: '官方日志快照', color: 'var(--accent-emerald)' };
}

function autoSourceBadge(
  autoSource: string | null | undefined
): { label: string; color: string } | null {
  if (!autoSource) {
    return null;
  }
  if (autoSource === 'official_api') {
    return { label: '自动获取 · 官方 API', color: 'var(--accent-emerald)' };
  }
  if (autoSource === 'official_cli') {
    return { label: '自动获取 · 官方 CLI', color: 'var(--accent-emerald)' };
  }
  if (autoSource === 'official_log_snapshot') {
    return { label: '自动获取 · 官方日志', color: 'var(--accent-emerald)' };
  }
  if (autoSource === 'local_log') {
    return { label: '自动获取 · 本地日志', color: 'var(--accent-sky)' };
  }
  return { label: `自动获取 · ${autoSource}`, color: 'var(--accent-sky)' };
}

function computeLiveResetSeconds(
  win: { resets_at_ms: number | null; resets_in_seconds: number },
  fetchedAtMs: number,
  nowMs: number
): number {
  if (win.resets_at_ms !== null && Number.isFinite(win.resets_at_ms)) {
    return Math.max(0, Math.floor((win.resets_at_ms - nowMs) / 1000));
  }
  const elapsedSecs =
    fetchedAtMs > 0 ? Math.max(0, Math.floor((nowMs - fetchedAtMs) / 1000)) : 0;
  return Math.max(0, win.resets_in_seconds - elapsedSecs);
}

export function SubscriptionsPage() {
  const queryClient = useQueryClient();
  const { currency, usdToCnyRate, tzOffsetMinutes } = useWorkspaceStore();

  const [confirmDeleteQuotaId, setConfirmDeleteQuotaId] = useState<string | null>(null);
  const [customFeeDraft, setCustomFeeDraft] = useState<{ planId: string; value: string } | null>(
    null
  );
  const [accountDraft, setAccountDraft] = useState<AccountFormDraft | null>(null);
  const [accountFormError, setAccountFormError] = useState<string | null>(null);
  const [confirmDeletePaygId, setConfirmDeletePaygId] = useState<string | null>(null);
  const [notice, setNotice] = useState<{ kind: 'success' | 'error'; text: string } | null>(null);
  const clearNotice = useCallback(() => setNotice(null), []);

  const [nowMs, setNowMs] = useState<number>(() => Date.now());
  const prevCurrencyRef = useRef<{ currency: CurrencyCode; usdToCnyRate: string }>({
    currency,
    usdToCnyRate,
  });

  useEffect(() => {
    const timer = window.setInterval(() => {
      setNowMs(Date.now());
    }, 1000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    const prev = prevCurrencyRef.current;
    if (prev.currency === currency && prev.usdToCnyRate === usdToCnyRate) {
      return;
    }
    prevCurrencyRef.current = { currency, usdToCnyRate };
    setAccountDraft((current) => {
      if (!current) {
        return current;
      }
      const manualMicro = current.manual_balance_input.trim()
        ? currencyInputToMicroUsd(current.manual_balance_input, prev.currency, prev.usdToCnyRate)
        : null;
      const budgetMicro = current.monthly_budget_input.trim()
        ? currencyInputToMicroUsd(current.monthly_budget_input, prev.currency, prev.usdToCnyRate)
        : null;
      return {
        ...current,
        manual_balance_input:
          manualMicro !== null
            ? microUsdToCurrencyInput(manualMicro, currency, usdToCnyRate)
            : current.manual_balance_input,
        monthly_budget_input:
          budgetMicro !== null
            ? microUsdToCurrencyInput(budgetMicro, currency, usdToCnyRate)
            : current.monthly_budget_input,
      };
    });
  }, [currency, usdToCnyRate]);

  const {
    data: plans = [],
    isLoading,
    dataUpdatedAt: plansUpdatedAt,
  } = useQuery({
    queryKey: ['subscriptions'],
    queryFn: () => ipc.getSubscriptions(),
    refetchInterval: CLOCK_REFRESH_MS,
  });

  const { data: planTemplates = [] } = useQuery({
    queryKey: ['subscription_templates'],
    queryFn: () => ipc.getSubscriptionTemplates(),
    staleTime: 5 * 60_000,
  });

  const templatesByProvider = useMemo(() => {
    const grouped = new Map<string, SubscriptionPlanTemplate[]>();
    for (const tpl of Array.isArray(planTemplates) ? planTemplates : []) {
      const key = tpl.provider_id.toLowerCase();
      const list = grouped.get(key) ?? [];
      list.push(tpl);
      grouped.set(key, list);
    }
    return grouped;
  }, [planTemplates]);

  const { data: observedRateLimits = [] } = useQuery({
    queryKey: ['observed_rate_limits'],
    queryFn: () => ipc.getObservedRateLimits(),
    refetchInterval: CLOCK_REFRESH_MS,
  });

  const { data: paygAccounts = [] } = useQuery({
    queryKey: ['payg_accounts', tzOffsetMinutes],
    queryFn: () => ipc.getPaygAccounts(tzOffsetMinutes),
    refetchInterval: CLOCK_REFRESH_MS,
  });

  const { data: providerConfigs = [] } = useQuery({
    queryKey: ['provider_configs'],
    queryFn: () => ipc.getProviderConfigs(),
    staleTime: 30_000,
  });

  const { data: remoteSnapshots = [] } = useQuery({
    queryKey: ['remote-subscriptions'],
    queryFn: () => ipc.getRemoteSubscriptionSnapshots(),
  });

  const remoteDeviceGroups = useMemo(() => {
    const safeSnapshots = Array.isArray(remoteSnapshots) ? remoteSnapshots : [];
    const grouped = new Map<
      string,
      { device_id: string; device_display_name: string; items: RemoteSubscriptionSnapshot[] }
    >();
    for (const snap of safeSnapshots) {
      const existing = grouped.get(snap.device_id);
      if (existing) {
        existing.items.push(snap);
      } else {
        grouped.set(snap.device_id, {
          device_id: snap.device_id,
          device_display_name: snap.device_display_name || snap.device_id,
          items: [snap],
        });
      }
    }
    return Array.from(grouped.values());
  }, [remoteSnapshots]);

  const quotaAccounts = useMemo(() => plans.filter((s) => s.plan.is_user_configured), [plans]);

  // Providers already represented by a plan row (enabled or paused): their observed windows
  // belong to that row and never show up again as a separate row.
  const boundQuotaProviders = useMemo(
    () => new Set(quotaAccounts.map((s) => s.plan.provider_id.toLowerCase())),
    [quotaAccounts]
  );

  const unboundObservedGroups = useMemo(() => {
    const grouped = new Map<string, ObservedRateLimitWindow[]>();
    for (const obs of observedRateLimits) {
      if (obs.source === 'local_log_hint' || obs.window_name === 'tier_hint') {
        continue;
      }
      if (boundQuotaProviders.has(obs.provider_id.toLowerCase())) {
        continue;
      }
      const list = grouped.get(obs.provider_id) ?? [];
      list.push(obs);
      grouped.set(obs.provider_id, list);
    }
    return Array.from(grouped.entries()).map(([providerId, windows]) => ({
      provider_id: providerId,
      windows: [...windows].sort(
        (a, b) => observedWindowSortKey(a) - observedWindowSortKey(b)
      ),
    }));
  }, [observedRateLimits, boundQuotaProviders]);

  const invalidateUsageQueries = async () => {
    await queryClient.invalidateQueries({ queryKey: ['subscriptions'] });
    await queryClient.invalidateQueries({ queryKey: ['payg_accounts'] });
    await queryClient.invalidateQueries({ queryKey: ['provider_configs'] });
    await queryClient.invalidateQueries({ queryKey: ['provider_balances'] });
    await queryClient.invalidateQueries({ queryKey: ['provider-balances'] });
    await queryClient.invalidateQueries({ queryKey: ['monthly'] });
    await queryClient.invalidateQueries({ queryKey: ['overview'] });
    await queryClient.invalidateQueries({ queryKey: [TRAY_SUMMARY_QUERY_KEY] });
    await queryClient.invalidateQueries({ queryKey: ['observed_rate_limits'] });
  };

  const toggleQuotaMutation = useMutation({
    mutationFn: (status: SubscriptionPlanStatus) =>
      ipc.setSubscriptionPlanEnabled(status.plan.plan_id, !status.plan.enabled),
    onSuccess: async () => {
      await invalidateUsageQueries();
    },
  });

  const applyTierMutation = useMutation({
    mutationFn: ({ status, tpl }: { status: SubscriptionPlanStatus; tpl: SubscriptionPlanTemplate }) =>
      ipc.upsertSubscriptionPlan({
        ...status.plan,
        plan_name: tpl.plan_name,
        monthly_fee_micro_usd: tpl.monthly_fee_micro_usd,
        is_user_configured: true,
      }),
    onSuccess: async () => {
      await invalidateUsageQueries();
    },
  });

  const applyCustomFeeMutation = useMutation({
    mutationFn: ({ status, feeMicroUsd }: { status: SubscriptionPlanStatus; feeMicroUsd: number }) =>
      ipc.upsertSubscriptionPlan({
        ...status.plan,
        monthly_fee_micro_usd: feeMicroUsd,
        is_user_configured: true,
      }),
    onSuccess: async () => {
      setCustomFeeDraft(null);
      await invalidateUsageQueries();
    },
  });

  const commitCustomFee = (status: SubscriptionPlanStatus) => {
    if (!customFeeDraft || customFeeDraft.planId !== status.plan.plan_id) return;
    const usd = Number.parseFloat(customFeeDraft.value);
    if (!Number.isFinite(usd) || usd < 0) {
      setCustomFeeDraft(null);
      return;
    }
    const feeMicroUsd = Math.round(usd * 1_000_000);
    if (feeMicroUsd === status.plan.monthly_fee_micro_usd) {
      setCustomFeeDraft(null);
      return;
    }
    applyCustomFeeMutation.mutate({ status, feeMicroUsd });
  };

  const deleteQuotaMutation = useMutation({
    mutationFn: (planId: string) => ipc.deleteSubscriptionPlan(planId),
    onSuccess: async () => {
      setConfirmDeleteQuotaId(null);
      await invalidateUsageQueries();
    },
  });

  const refreshQuotaMutation = useMutation({
    mutationFn: async (providerId: string) => {
      if (QUOTA_BALANCE_API_PROVIDERS.has(providerId) || CLI_QUOTA_LOGIN_PROVIDERS.has(providerId)) {
        const outcome = await ipc.refreshProviderBalance(providerId);
        return outcome.from_cache
          ? `${providerDisplayName(providerId)} 冷却中 ${outcome.cooldown_remaining_secs}s`
          : `${providerDisplayName(providerId)} 实时配额已刷新`;
      }
      await ipc.triggerScanNow();
      return `${providerDisplayName(providerId)} 配额已刷新`;
    },
    onSuccess: async (text) => {
      setNotice({ kind: 'success', text });
      await invalidateUsageQueries();
    },
    onError: (err: unknown) => {
      setNotice({
        kind: 'error',
        text: err instanceof Error ? err.message : String(err),
      });
    },
  });

  const launchLoginMutation = useMutation({
    mutationFn: (providerId: string) => ipc.launchProviderCliLogin(providerId),
    onSuccess: (message) => {
      setNotice({ kind: 'success', text: message });
      setAccountFormError(null);
    },
    onError: (err: unknown) => {
      setAccountFormError(err instanceof Error ? err.message : String(err));
    },
  });

  const syncLoginAccountMutation = useMutation({
    mutationFn: async (providerId: string) => {
      if (CLI_QUOTA_LOGIN_PROVIDERS.has(providerId)) {
        const outcome = await ipc.refreshProviderBalance(providerId);
        if (providerId === 'claude_code') {
          await ipc.triggerScanNow();
        }
        return outcome.from_cache
          ? `${providerDisplayName(providerId)} 冷却中 ${outcome.cooldown_remaining_secs}s`
          : `${providerDisplayName(providerId)} 配额已同步`;
      }
      const report = await ipc.triggerScanNow();
      return `${providerDisplayName(providerId)} 已同步 ${report.files_scanned} 个文件`;
    },
    onSuccess: async (text) => {
      setAccountDraft(null);
      setAccountFormError(null);
      setNotice({ kind: 'success', text });
      await invalidateUsageQueries();
    },
    onError: (err: unknown) => {
      setAccountFormError(err instanceof Error ? err.message : String(err));
    },
  });

  const saveAccountMutation = useMutation({
    mutationFn: async (payload: {
      account: PaygAccount;
      apiKey?: string;
      shouldUpsertPayg: boolean;
    }) => {
      if (payload.shouldUpsertPayg) {
        await ipc.upsertPaygAccount(payload.account);
      }
      const trimmedKey = payload.apiKey?.trim();
      if (trimmedKey && API_KEY_CAPABLE_PROVIDERS.has(payload.account.provider_id)) {
        await ipc.setProviderApiKey(payload.account.provider_id, trimmedKey);
      }
      return payload;
    },
    onSuccess: async (payload) => {
      setAccountDraft(null);
      setAccountFormError(null);
      setNotice({
        kind: 'success',
        text: payload.apiKey?.trim()
          ? `已接入「${payload.account.account_name}」`
          : `已保存账户「${payload.account.account_name}」`,
      });
      await invalidateUsageQueries();
    },
    onError: (err: unknown) => {
      setAccountFormError(err instanceof Error ? err.message : String(err));
    },
  });

  const deletePaygMutation = useMutation({
    mutationFn: (accountId: string) => ipc.deletePaygAccount(accountId),
    onSuccess: async () => {
      setConfirmDeletePaygId(null);
      await invalidateUsageQueries();
    },
  });

  const refreshPaygBalanceMutation = useMutation({
    mutationFn: (providerId: string) => ipc.refreshProviderBalance(providerId),
    onSuccess: async (outcome) => {
      await invalidateUsageQueries();
      setNotice({
        kind: 'success',
        text: outcome.from_cache
          ? `${providerDisplayName(outcome.snapshot.provider_id)} 冷却中 ${outcome.cooldown_remaining_secs}s`
          : `${providerDisplayName(outcome.snapshot.provider_id)} 余额已刷新`,
      });
    },
    onError: (err: unknown) => {
      setNotice({
        kind: 'error',
        text: err instanceof Error ? err.message : String(err),
      });
    },
  });

  const openAddAccountDrawer = (connectKind: 'login' | 'api' = 'login') => {
    const providerId = connectKind === 'login' ? 'antigravity' : 'deepseek_harness';
    setAccountFormError(null);
    setAccountDraft({
      mode: 'create',
      connectKind,
      account_id: `payg_${providerId}_${Date.now()}`,
      provider_id: providerId,
      account_name: defaultAccountName(providerId),
      enabled: true,
      is_auto: false,
      api_key_input: '',
      manual_balance_input: '',
      initial_manual_balance_micro_usd: null,
      existing_anchor_ms: null,
      monthly_budget_input: '',
    });
  };

  const openEditAccountDrawer = (status: PaygAccountStatus) => {
    const acc = status.account;
    setAccountFormError(null);
    setAccountDraft({
      mode: 'edit',
      connectKind: 'api',
      account_id: acc.account_id,
      provider_id: acc.provider_id,
      account_name: acc.account_name,
      enabled: acc.enabled,
      is_auto: acc.is_auto,
      api_key_input: '',
      manual_balance_input:
        acc.manual_balance_micro_usd !== null
          ? microUsdToCurrencyInput(acc.manual_balance_micro_usd, currency, usdToCnyRate)
          : '',
      initial_manual_balance_micro_usd: acc.manual_balance_micro_usd,
      existing_anchor_ms: acc.balance_anchor_ms,
      monthly_budget_input:
        acc.monthly_budget_micro_usd !== null
          ? microUsdToCurrencyInput(acc.monthly_budget_micro_usd, currency, usdToCnyRate)
          : '',
    });
  };

  const togglePaygEnabled = (status: PaygAccountStatus) => {
    saveAccountMutation.mutate({
      shouldUpsertPayg: true,
      account: {
        ...status.account,
        enabled: !status.account.enabled,
        updated_at_ms: Date.now(),
      },
    });
  };

  const handleSaveApiAccount = () => {
    if (!accountDraft) return;
    const trimmedName = accountDraft.account_name.trim();
    if (!trimmedName) {
      setAccountFormError('请输入账户名称');
      return;
    }

    const trimmedKey = accountDraft.api_key_input.trim();
    const hasManualInput = Boolean(accountDraft.manual_balance_input.trim());
    const hasBudgetInput = Boolean(accountDraft.monthly_budget_input.trim());

    if (accountDraft.mode === 'create' && !trimmedKey && !hasManualInput && !hasBudgetInput) {
      setAccountFormError('请填写 API Key、起始余额或月度上限');
      return;
    }

    let manualBalanceMicro: number | null = null;
    if (hasManualInput) {
      const parsed = currencyInputToMicroUsd(
        accountDraft.manual_balance_input,
        currency,
        usdToCnyRate
      );
      if (parsed === null || parsed < 0) {
        setAccountFormError('起始余额需 ≥ 0');
        return;
      }
      manualBalanceMicro = parsed;
    }

    let monthlyBudgetMicro: number | null = null;
    if (hasBudgetInput) {
      const parsed = currencyInputToMicroUsd(
        accountDraft.monthly_budget_input,
        currency,
        usdToCnyRate
      );
      if (parsed === null || parsed <= 0) {
        setAccountFormError('月度支出上限需大于 0');
        return;
      }
      monthlyBudgetMicro = parsed;
    }

    const now = Date.now();
    const balanceAnchorMs =
      manualBalanceMicro === null
        ? null
        : accountDraft.mode === 'edit' &&
            accountDraft.initial_manual_balance_micro_usd === manualBalanceMicro &&
            accountDraft.existing_anchor_ms !== null
          ? accountDraft.existing_anchor_ms
          : now;

    const shouldUpsertPayg =
      accountDraft.mode === 'edit' ||
      hasManualInput ||
      hasBudgetInput ||
      !API_KEY_CAPABLE_PROVIDERS.has(accountDraft.provider_id);

    setAccountFormError(null);
    saveAccountMutation.mutate({
      shouldUpsertPayg,
      account: {
        account_id: accountDraft.account_id,
        provider_id: accountDraft.provider_id,
        account_name: trimmedName,
        enabled: accountDraft.enabled,
        manual_balance_micro_usd: manualBalanceMicro,
        balance_anchor_ms: balanceAnchorMs,
        monthly_budget_micro_usd: monthlyBudgetMicro,
        is_auto: accountDraft.is_auto,
        updated_at_ms: now,
      },
      apiKey: trimmedKey || undefined,
    });
  };

  const renderQuotaRowActions = (row: { planId: string; providerId: string; name: string }) => (
    <div className="flex items-center gap-1.5 shrink-0">
      {confirmDeleteQuotaId === row.planId ? (
        <div className="flex items-center gap-1">
          <button
            type="button"
            disabled={deleteQuotaMutation.isPending}
            onClick={() => deleteQuotaMutation.mutate(row.planId)}
            className="btn-danger inline-flex items-center gap-1 px-2 py-1 rounded-lg text-[11px] font-medium cursor-pointer disabled:opacity-50"
            style={{
              backgroundColor: 'var(--accent-rose)',
              color: '#ffffff',
            }}
          >
            <Check className="w-3 h-3" />
            <span>确认</span>
          </button>
          <button
            type="button"
            onClick={() => setConfirmDeleteQuotaId(null)}
            className="btn-interactive bg-[var(--bg-elevated)] px-2 py-1 rounded-lg text-[11px] cursor-pointer"
            style={{
              border: '1px solid var(--border-subtle)',
              color: 'var(--text-secondary)',
            }}
          >
            取消
          </button>
        </div>
      ) : (
        <>
          <button
            type="button"
            disabled={refreshQuotaMutation.isPending}
            onClick={() => refreshQuotaMutation.mutate(row.providerId)}
            className="btn-interactive bg-[var(--bg-elevated)] p-1.5 rounded-lg cursor-pointer disabled:opacity-50"
            style={{
              border: '1px solid var(--border-subtle)',
              color: 'var(--text-secondary)',
            }}
            title={`刷新 ${row.name}`}
            aria-label={`刷新 ${row.name}`}
          >
            <RefreshCw
              className={`w-3.5 h-3.5 ${refreshQuotaMutation.isPending ? 'animate-spin' : ''}`}
            />
          </button>
          <button
            type="button"
            onClick={() => setConfirmDeleteQuotaId(row.planId)}
            className="btn-danger bg-[var(--bg-elevated)] p-1.5 rounded-lg cursor-pointer"
            style={{
              border: '1px solid var(--border-subtle)',
              color: 'var(--accent-rose)',
            }}
            title="移除该配额项"
            aria-label={`删除 ${row.name}`}
          >
            <Trash2 className="w-3.5 h-3.5" />
          </button>
        </>
      )}
    </div>
  );

  const hasCredentialForProvider = (providerId: string): boolean =>
    providerConfigs.some((cfg) => cfg.provider_id === providerId && cfg.has_credential);

  const hasAnyQuotaSection = quotaAccounts.length > 0 || unboundObservedGroups.length > 0;
  const hasAnyContent =
    hasAnyQuotaSection || paygAccounts.length > 0 || remoteDeviceGroups.length > 0;

  return (
    <div className="space-y-7">
      {/* Header */}
      <section
        className="flex flex-wrap items-center justify-between gap-4 pb-5"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <h1 className="text-[16px] font-semibold tracking-tight">用量</h1>

        <button
          type="button"
          onClick={() => openAddAccountDrawer('login')}
          className="btn-primary inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium cursor-pointer"
          style={{
            backgroundColor: 'var(--accent-primary)',
            color: '#ffffff',
          }}
        >
          <Plus className="w-3.5 h-3.5" />
          <span>添加账户</span>
        </button>
      </section>

      <ActionNotice text={notice?.text ?? null} kind={notice?.kind} onClose={clearNotice} />

      {isLoading ? (
        <div className="py-12 text-center" style={{ color: 'var(--text-muted)' }}>
          加载中…
        </div>
      ) : !hasAnyContent ? (
        <div className="surface-tonal-band py-6 px-5 flex items-center gap-2.5 text-[13px]">
          <Info className="w-4 h-4 shrink-0" style={{ color: 'var(--accent-primary)' }} />
          <span>添加账户以同步配额与余额</span>
        </div>
      ) : (
        <>
          {/* Compact Account Hub Summary Strip */}
          <section className="surface-tonal-band px-4 py-3 grid grid-cols-2 sm:grid-cols-4 gap-3 text-[12px] tabular-nums">
            <div>
              <div className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
                配额监测窗口
              </div>
              <div className="mt-0.5 text-[16px] font-semibold">
                {quotaAccounts.reduce((acc, s) => acc + s.windows.length, 0) +
                  unboundObservedGroups.reduce((acc, g) => acc + g.windows.length, 0)}
              </div>
            </div>
            <div style={{ borderLeft: '1px solid var(--border-hairline)', paddingLeft: '1rem' }}>
              <div className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
                按量余额账户
              </div>
              <div className="mt-0.5 text-[16px] font-semibold">{paygAccounts.length}</div>
            </div>
            <div style={{ borderLeft: '1px solid var(--border-hairline)', paddingLeft: '1rem' }}>
              <div className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
                同步设备
              </div>
              <div className="mt-0.5 text-[16px] font-semibold">{remoteDeviceGroups.length}</div>
            </div>
            <div style={{ borderLeft: '1px solid var(--border-hairline)', paddingLeft: '1rem' }}>
              <div className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
                最高占用
              </div>
              <div
                className="mt-0.5 text-[16px] font-semibold"
                style={{ color: 'var(--accent-primary)' }}
              >
                {(() => {
                  const pcts = [
                    ...quotaAccounts.flatMap((s) => s.windows.map((w) => w.used_percent)),
                    ...unboundObservedGroups.flatMap((g) => g.windows.map((o) => o.used_percent)),
                  ].filter((v) => Number.isFinite(v));
                  return pcts.length > 0 ? `${Math.max(...pcts).toFixed(1)}%` : '—';
                })()}
              </div>
            </div>
          </section>

          {/* Section 1: Authoritative Quota Windows (CLI / Official API / Official Log Snapshot) */}
          {hasAnyQuotaSection && (
            <section className="space-y-2">
              <div className="flex items-center justify-between">
                <h2
                  className="text-[11px] font-semibold uppercase tracking-wider"
                  style={{ color: 'var(--text-muted)' }}
                >
                  配额窗口
                </h2>
                <span className="text-[11.5px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
                  {quotaAccounts.length + unboundObservedGroups.length} 项数据源
                </span>
              </div>

              <div style={{ borderTop: '1px solid var(--border-subtle)' }}>
                {quotaAccounts.map((status) => {
                  const { plan, windows } = status;
                                    const autoBadge = autoSourceBadge(plan.auto_source);
                  const tierOptions = templatesByProvider.get(plan.provider_id.toLowerCase()) ?? [];
                  const selectedTier = tierOptions.find(
                    (tpl) =>
                      tpl.plan_name === plan.plan_name &&
                      tpl.monthly_fee_micro_usd === plan.monthly_fee_micro_usd
                  );

                  return (
                    <div
                      key={plan.plan_id}
                      className="row-interactive py-5 px-4 grid grid-cols-12 gap-5 items-center"
                      style={{
                        borderBottom: '1px solid var(--border-hairline)',
                        opacity: plan.enabled ? 1 : 0.58,
                      }}
                    >
                      {/* Left Column: Provider + Identity */}
                      <div className="col-span-12 lg:col-span-3 space-y-1.5">
                        <div className="flex items-center gap-2">
                          <span
                            className="inline-block w-2 h-2 rounded-full shrink-0"
                            style={{ backgroundColor: providerAccentColor(plan.provider_id) }}
                          />
                          <span
                            className="text-[11px] uppercase tracking-wider"
                            style={{ color: 'var(--text-muted)' }}
                          >
                            {providerDisplayName(plan.provider_id)}
                          </span>
                        </div>
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="text-[14px] font-semibold">{plan.plan_name}</span>
                          {autoBadge && (
                            <span
                              className="px-1.5 py-0.5 text-[10px] rounded-md shrink-0"
                              style={{
                                backgroundColor: 'var(--bg-elevated)',
                                border: '1px solid var(--border-hairline)',
                                color: autoBadge.color,
                              }}
                            >
                              {autoBadge.label}
                            </span>
                          )}
                        </div>
                        <div
                          className="flex flex-wrap items-center gap-2 text-[11.5px] tabular-nums"
                          style={{ color: 'var(--text-secondary)' }}
                        >
                          <span>
                            {resolveSubscriptionTierLabel(plan, currency, usdToCnyRate, true)}
                          </span>
                          <select
                              aria-label={`${plan.plan_name} 订阅档位`}
                              value={
                                customFeeDraft?.planId === plan.plan_id
                                  ? CUSTOM_TIER_VALUE
                                  : (selectedTier?.template_id ?? CURRENT_TIER_VALUE)
                              }
                              disabled={applyTierMutation.isPending || applyCustomFeeMutation.isPending}
                              onChange={(e) => {
                                if (e.target.value === CUSTOM_TIER_VALUE) {
                                  setCustomFeeDraft({
                                    planId: plan.plan_id,
                                    value: String(plan.monthly_fee_micro_usd / 1_000_000),
                                  });
                                  return;
                                }
                                setCustomFeeDraft(null);
                                const tpl = tierOptions.find(
                                  (t) => t.template_id === e.target.value
                                );
                                if (tpl) {
                                  applyTierMutation.mutate({ status, tpl });
                                }
                              }}
                              className="px-1.5 py-0.5 text-[11px] rounded-lg outline-none cursor-pointer tabular-nums disabled:opacity-50"
                              style={{
                                backgroundColor: 'var(--bg-input)',
                                border: '1px solid var(--border-subtle)',
                                color: 'var(--text-primary)',
                              }}
                            >
                              {!selectedTier && (
                                <option value={CURRENT_TIER_VALUE} disabled>
                                  {plan.monthly_fee_micro_usd > 0
                                    ? `${plan.plan_name} · ${formatCost(plan.monthly_fee_micro_usd, currency, usdToCnyRate)}`
                                    : plan.plan_name}
                                </option>
                              )}
                              {tierOptions.map((tpl) => (
                                <option key={tpl.template_id} value={tpl.template_id}>
                                  {tpl.monthly_fee_micro_usd > 0
                                    ? `${tpl.plan_name} · ${formatCost(tpl.monthly_fee_micro_usd, currency, usdToCnyRate)}`
                                    : tpl.plan_name}
                                </option>
                              ))}
                              <option value={CUSTOM_TIER_VALUE}>自定义月费…</option>
                            </select>
                          {customFeeDraft?.planId === plan.plan_id && (
                            <label className="inline-flex items-center gap-1">
                              <span>USD</span>
                              <input
                                type="number"
                                min={0}
                                step={0.01}
                                autoFocus
                                aria-label={`${plan.plan_name} 月费 USD`}
                                value={customFeeDraft.value}
                                onChange={(e) =>
                                  setCustomFeeDraft({ planId: plan.plan_id, value: e.target.value })
                                }
                                onKeyDown={(e) => {
                                  if (e.key === 'Enter') commitCustomFee(status);
                                  if (e.key === 'Escape') setCustomFeeDraft(null);
                                }}
                                onBlur={() => commitCustomFee(status)}
                                className="w-20 px-1.5 py-0.5 text-[11px] rounded-lg outline-none tabular-nums"
                                style={{
                                  backgroundColor: 'var(--bg-input)',
                                  border: '1px solid var(--border-subtle)',
                                  color: 'var(--text-primary)',
                                }}
                              />
                            </label>
                          )}
                          <button
                            type="button"
                            onClick={() => toggleQuotaMutation.mutate(status)}
                            disabled={toggleQuotaMutation.isPending}
                            className="btn-interactive bg-[var(--bg-elevated)] px-2 py-0.5 rounded-lg text-[11px] cursor-pointer disabled:opacity-50"
                            style={{
                              border: '1px solid var(--border-hairline)',
                              color: plan.enabled ? 'var(--accent-emerald)' : 'var(--text-muted)',
                            }}
                          >
                            {plan.enabled ? '已启用' : '已停用'}
                          </button>
                        </div>
                      </div>

                      {/* Center Column: Official Windows + Factual Spend Summary */}
                      <div className="col-span-12 lg:col-span-6 space-y-3">
                        {windows.map((w) => {
                          const isPercentOnlyWindow =
                            w.source === 'official_cli' ||
                            (w.unit === 'tokens' && w.limit_value === 100);
                          const usedStr =
                            w.unit === 'micro_usd'
                              ? formatCost(w.used_value, currency, usdToCnyRate)
                              : formatTokens(w.used_value);
                          const limitStr =
                            w.unit === 'micro_usd'
                              ? formatCost(w.limit_value, currency, usdToCnyRate)
                              : formatTokens(w.limit_value);
                          const srcBadge = windowSourceBadge(w.source);
                          return (
                            <div key={w.window_id} className="space-y-1">
                              <div className="flex items-center justify-between gap-2 text-[12px] tabular-nums">
                                <div className="flex items-center gap-2 min-w-0">
                                  <span className="font-medium truncate">{w.label}</span>
                                  <span
                                    className="px-1.5 py-0.5 text-[10px] rounded-md shrink-0"
                                    style={{
                                      backgroundColor: 'var(--bg-elevated)',
                                      border: '1px solid var(--border-hairline)',
                                      color: srcBadge.color,
                                    }}
                                  >
                                    {srcBadge.label}
                                  </span>
                                </div>
                                <div className="shrink-0">
                                  {!isPercentOnlyWindow && (
                                    <span className="mr-2" style={{ color: 'var(--text-secondary)' }}>
                                      {usedStr} / {limitStr}
                                    </span>
                                  )}
                                  <span className="font-semibold">
                                    已用 {w.used_percent.toFixed(1)}%
                                  </span>
                                </div>
                              </div>
                              <InlineProgressBar
                                percent={w.used_percent}
                                alertLevel={w.alert_level}
                                heightPx={6}
                              />
                            </div>
                          );
                        })}

                        <div
                          className="pt-1 flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] tabular-nums"
                          style={{ color: 'var(--text-muted)' }}
                        >
                          <span>
                            今日等效 {formatCost(status.today_cost_micro_usd, currency, usdToCnyRate)}
                          </span>
                          <span>
                            近 7d {formatCost(status.week_cost_micro_usd, currency, usdToCnyRate)}
                          </span>
                          <span>
                            本月等效价值{' '}
                            {formatCost(status.month_cost_micro_usd, currency, usdToCnyRate)}（
                            {formatTokens(status.month_tokens)} Tokens）
                          </span>
                        </div>
                      </div>

                      {/* Right Column: Reset Countdown & Actions */}
                      <div className="col-span-12 lg:col-span-3 flex items-center justify-between gap-3">
                        <div className="space-y-1 text-[11.5px] tabular-nums">
                          {windows.map((w) => (
                            <div
                              key={w.window_id}
                              data-testid="quota-reset-line"
                              className="whitespace-nowrap"
                              style={{ color: 'var(--text-muted)' }}
                            >
                              {windows.length > 1 ? `${w.window_id} · ` : ''}
                              <strong style={{ color: 'var(--text-primary)', fontWeight: 600 }}>
                                {formatCountdown(computeLiveResetSeconds(w, plansUpdatedAt, nowMs))}
                              </strong>{' '}
                              后重置
                            </div>
                          ))}
                        </div>

                        {renderQuotaRowActions({
                          planId: plan.plan_id,
                          providerId: plan.provider_id,
                          name: plan.plan_name,
                        })}
                      </div>
                    </div>
                  );
                })}

                {unboundObservedGroups.map((group) => (
                  <div
                    key={group.provider_id}
                    className="row-interactive py-5 px-4 grid grid-cols-12 gap-5 items-center"
                    style={{ borderBottom: '1px solid var(--border-hairline)' }}
                  >
                    <div className="col-span-12 lg:col-span-3 space-y-1.5">
                      <div className="flex items-center gap-2">
                        <span
                          className="inline-block w-2 h-2 rounded-full shrink-0"
                          style={{ backgroundColor: providerAccentColor(group.provider_id) }}
                        />
                        <span className="text-[14px] font-semibold">
                          {providerDisplayName(group.provider_id)}
                        </span>
                      </div>
                      <div className="flex items-center gap-2">
                        <span
                          className="px-1.5 py-0.5 rounded-md text-[10.5px]"
                          style={{
                            backgroundColor: 'var(--bg-elevated)',
                            border: '1px solid var(--border-hairline)',
                            color: 'var(--accent-emerald)',
                          }}
                        >
                          {group.windows[0]?.plan_hint
                            ? `识别档位 · ${group.windows[0].plan_hint}`
                            : '官方实时限额'}
                        </span>
                      </div>
                    </div>

                    <div className="col-span-12 lg:col-span-6 space-y-3">
                      {group.windows.map((obs) => {
                        const pct = Number.isFinite(obs.used_percent)
                          ? Math.max(0, Math.min(100, obs.used_percent))
                          : 0;
                        const label = formatObservedWindowLabel(obs);
                        return (
                          <div key={`${obs.provider_id}-${obs.window_name}`} className="space-y-1">
                            <div className="flex items-center justify-between text-[12px] tabular-nums">
                              <span className="font-medium">{label}</span>
                              <span className="font-semibold">已用 {pct.toFixed(1)}%</span>
                            </div>
                            <InlineProgressBar percent={pct} heightPx={6} />
                          </div>
                        );
                      })}
                    </div>

                    <div className="col-span-12 lg:col-span-3 flex items-center justify-between gap-3">
                    <div className="space-y-1 text-[11.5px] tabular-nums">
                      {group.windows.map((obs) => {
                        const resetSecs =
                          obs.resets_at_ms !== null
                            ? Math.max(0, Math.floor((obs.resets_at_ms - nowMs) / 1000))
                            : null;
                        return (
                          <div
                            key={`${obs.provider_id}-${obs.window_name}-reset`}
                            data-testid="quota-reset-line"
                            className="whitespace-nowrap"
                            style={{ color: 'var(--text-muted)' }}
                          >
                            {group.windows.length > 1
                              ? `${formatObservedWindowLabel(obs).replace(' 官方限额', '')} · `
                              : ''}
                            {resetSecs !== null ? (
                              <>
                                <strong style={{ color: 'var(--text-primary)', fontWeight: 600 }}>
                                  {formatCountdown(resetSecs)}
                                </strong>{' '}
                                后重置
                              </>
                            ) : (
                              '待刷新'
                            )}
                          </div>
                        );
                      })}
                    </div>
                    {renderQuotaRowActions({
                      planId: `auto_${group.provider_id}`,
                      providerId: group.provider_id,
                      name: providerDisplayName(group.provider_id),
                    })}
                    </div>
                  </div>
                ))}
              </div>
            </section>
          )}

          {/* Section 2: Official Balances, Credits & Pay-As-You-Go Accounts */}
          {paygAccounts.length > 0 && (
            <section className="space-y-2.5">
              <div className="flex items-center justify-between">
                <h2
                  className="text-[11px] font-semibold uppercase tracking-wider"
                  style={{ color: 'var(--text-muted)' }}
                >
                  按量账户
                </h2>
                <span className="text-[11.5px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
                  {paygAccounts.length} 项账户
                </span>
              </div>

              <div style={{ borderTop: '1px solid var(--border-subtle)' }}>
                {paygAccounts.map((status) => {
                  const acc = status.account;
                  const offBal = status.official_balance;
                  const confirmingDelete = confirmDeletePaygId === acc.account_id;
                  const supportsBalanceRefresh =
                    QUOTA_BALANCE_API_PROVIDERS.has(acc.provider_id) ||
                    (acc.provider_id === 'deepseek_harness' &&
                      hasCredentialForProvider(acc.provider_id));
                  const budgetMicro = acc.monthly_budget_micro_usd;
                  const budgetPct =
                    status.budget_used_percent !== null && Number.isFinite(status.budget_used_percent)
                      ? Math.max(0, status.budget_used_percent)
                      : null;

                  return (
                    <div
                      key={acc.account_id}
                      className="row-interactive py-4 px-4 grid grid-cols-12 gap-5 items-center"
                      style={{
                        borderBottom: '1px solid var(--border-hairline)',
                        opacity: acc.enabled ? 1 : 0.58,
                      }}
                    >
                      {/* Left: Provider & Account Identity */}
                      <div className="col-span-12 lg:col-span-3 space-y-1.5">
                        <div className="flex items-center gap-2">
                          <span
                            className="inline-block w-2 h-2 rounded-full shrink-0"
                            style={{ backgroundColor: providerAccentColor(acc.provider_id) }}
                          />
                          <span
                            className="text-[11px] uppercase tracking-wider"
                            style={{ color: 'var(--text-muted)' }}
                          >
                            {providerDisplayName(acc.provider_id)}
                          </span>
                        </div>
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="text-[14px] font-semibold">{acc.account_name}</span>
                          <span
                            className="px-1.5 py-0.5 text-[10px] rounded-md shrink-0"
                            style={{
                              backgroundColor: 'var(--bg-elevated)',
                              border: '1px solid var(--border-hairline)',
                              color: offBal
                                ? 'var(--accent-emerald)'
                                : acc.is_auto
                                  ? 'var(--accent-sky)'
                                  : 'var(--text-secondary)',
                            }}
                          >
                            {offBal
                              ? acc.provider_id === 'codex'
                                ? '本地日志 Credits'
                                : '官方 API 余额'
                              : '按量预算追踪'}
                          </span>
                        </div>
                        <div className="flex items-center gap-2">
                          <button
                            type="button"
                            onClick={() => togglePaygEnabled(status)}
                            disabled={saveAccountMutation.isPending}
                            className="btn-interactive bg-[var(--bg-elevated)] px-2 py-0.5 rounded-lg text-[11px] cursor-pointer disabled:opacity-50"
                            style={{
                              border: '1px solid var(--border-hairline)',
                              color: acc.enabled ? 'var(--accent-emerald)' : 'var(--text-muted)',
                            }}
                          >
                            {acc.enabled ? '已启用' : '已停用'}
                          </button>
                        </div>
                      </div>

                      {/* Center: Balance / Credits & Spend */}
                      <div className="col-span-12 lg:col-span-6 space-y-2.5 tabular-nums">
                        {offBal && (
                          <div className="flex flex-wrap items-baseline justify-between gap-2">
                            <div className="flex flex-wrap items-baseline gap-2.5">
                              <span className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
                                {offBal.currency.startsWith('CREDITS') ? '可用 Credits' : '账户总余额'}
                              </span>
                              <span className="text-[16px] font-semibold">
                                {formatBalanceAmount(
                                  offBal.total_balance_micro,
                                  offBal.currency,
                                  currency,
                                  usdToCnyRate
                                )}
                              </span>
                            </div>
                            {(offBal.granted_balance_micro !== null ||
                              offBal.topped_up_balance_micro !== null) && (
                              <div
                                className="flex flex-wrap items-center gap-2 text-[11px]"
                                style={{ color: 'var(--text-secondary)' }}
                              >
                                {offBal.granted_balance_micro !== null && (
                                  <span>
                                    赠金{' '}
                                    {formatBalanceAmount(
                                      offBal.granted_balance_micro,
                                      offBal.currency,
                                      currency,
                                      usdToCnyRate
                                    )}
                                  </span>
                                )}
                                {offBal.topped_up_balance_micro !== null && (
                                  <span>
                                    · 充值{' '}
                                    {formatBalanceAmount(
                                      offBal.topped_up_balance_micro,
                                      offBal.currency,
                                      currency,
                                      usdToCnyRate
                                    )}
                                  </span>
                                )}
                              </div>
                            )}
                          </div>
                        )}

                        {status.remaining_manual_balance_micro_usd !== null && (
                          <div className="flex items-baseline justify-between gap-2 text-[12px]">
                            <span style={{ color: 'var(--text-secondary)' }}>预充剩余</span>
                            <span
                              className="font-semibold"
                              style={{
                                color:
                                  status.remaining_manual_balance_micro_usd <= 0
                                    ? 'var(--accent-rose)'
                                    : 'var(--text-primary)',
                              }}
                            >
                              {formatCost(
                                status.remaining_manual_balance_micro_usd,
                                currency,
                                usdToCnyRate
                              )}
                            </span>
                          </div>
                        )}

                        <div
                          className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[11.5px]"
                          style={{ color: 'var(--text-secondary)' }}
                        >
                          <span>
                            今日按量{' '}
                            <strong style={{ color: 'var(--text-primary)' }}>
                              {formatCost(status.today_cost_micro_usd, currency, usdToCnyRate)}
                            </strong>
                          </span>
                          <span>
                            本月累计{' '}
                            <strong style={{ color: 'var(--text-primary)' }}>
                              {formatCost(status.month_cost_micro_usd, currency, usdToCnyRate)}
                            </strong>
                          </span>
                        </div>

                        {budgetMicro !== null && budgetMicro > 0 && budgetPct !== null && (
                          <div className="space-y-1">
                            <div className="flex items-center justify-between text-[11.5px]">
                              <span style={{ color: 'var(--text-muted)' }}>本月按量预算</span>
                              <span>
                                {formatCost(status.month_cost_micro_usd, currency, usdToCnyRate)} /{' '}
                                {formatCost(budgetMicro, currency, usdToCnyRate)} ({budgetPct.toFixed(1)}%)
                              </span>
                            </div>
                            <InlineProgressBar percent={budgetPct} heightPx={5} />
                          </div>
                        )}
                      </div>

                      {/* Right: Timestamp, Refresh Balance, Edit & Delete */}
                      <div className="col-span-12 lg:col-span-3 flex items-center justify-between gap-3">
                        <div className="space-y-1.5 text-[11.5px] tabular-nums">
                          <div style={{ color: 'var(--text-muted)' }}>
                            {offBal
                              ? `快照于 ${formatRelativeTime(offBal.fetched_at_ms)}`
                              : `更新于 ${formatRelativeTime(acc.updated_at_ms)}`}
                          </div>
                          {supportsBalanceRefresh && hasCredentialForProvider(acc.provider_id) && (
                            <button
                              type="button"
                              disabled={refreshPaygBalanceMutation.isPending}
                              onClick={() => refreshPaygBalanceMutation.mutate(acc.provider_id)}
                              className="btn-tonal inline-flex items-center gap-1 px-2 py-0.5 rounded-lg text-[11px] cursor-pointer disabled:opacity-50"
                            >
                              <RefreshCw
                                className={`w-3 h-3 ${
                                  refreshPaygBalanceMutation.isPending ? 'animate-spin' : ''
                                }`}
                              />
                              <span>刷新余额</span>
                            </button>
                          )}
                        </div>

                        <div className="flex items-center gap-1.5 shrink-0">
                          {confirmingDelete ? (
                            <div className="flex items-center gap-1">
                              <button
                                type="button"
                                disabled={deletePaygMutation.isPending}
                                onClick={() => deletePaygMutation.mutate(acc.account_id)}
                                className="btn-danger inline-flex items-center gap-1 px-2 py-1 rounded-lg text-[11px] font-medium cursor-pointer disabled:opacity-50"
                                style={{
                                  backgroundColor: 'var(--accent-rose)',
                                  color: '#ffffff',
                                }}
                              >
                                <Check className="w-3 h-3" />
                                <span>确认</span>
                              </button>
                              <button
                                type="button"
                                onClick={() => setConfirmDeletePaygId(null)}
                                className="btn-interactive bg-[var(--bg-elevated)] px-2 py-1 rounded-lg text-[11px] cursor-pointer"
                                style={{
                                  border: '1px solid var(--border-subtle)',
                                  color: 'var(--text-secondary)',
                                }}
                              >
                                取消
                              </button>
                            </div>
                          ) : (
                            <>
                              <button
                                type="button"
                                onClick={() => openEditAccountDrawer(status)}
                                className="btn-interactive bg-[var(--bg-elevated)] p-1.5 rounded-lg cursor-pointer"
                                style={{
                                  border: '1px solid var(--border-subtle)',
                                  color: 'var(--text-secondary)',
                                }}
                                title="编辑账户"
                                aria-label={`编辑 ${acc.account_name}`}
                              >
                                <Pencil className="w-3.5 h-3.5" />
                              </button>
                              <button
                                type="button"
                                onClick={() => setConfirmDeletePaygId(acc.account_id)}
                                className="btn-danger bg-[var(--bg-elevated)] p-1.5 rounded-lg cursor-pointer"
                                style={{
                                  border: '1px solid var(--border-subtle)',
                                  color: 'var(--accent-rose)',
                                }}
                                title="删除账户"
                                aria-label={`删除 ${acc.account_name}`}
                              >
                                <Trash2 className="w-3.5 h-3.5" />
                              </button>
                            </>
                          )}
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            </section>
          )}

          {/* Section 3: Cross-Device Synced Remote Quota Snapshots (Read-only) */}
          {remoteDeviceGroups.length > 0 && (
            <section className="space-y-2">
              <h2
                className="text-[11px] font-semibold uppercase tracking-wider"
                style={{ color: 'var(--text-muted)' }}
              >
                其他设备配额
              </h2>

              <div style={{ borderTop: '1px solid var(--border-subtle)' }}>
                {remoteDeviceGroups.map((devGroup) => (
                  <div
                    key={devGroup.device_id}
                    className="py-5 px-4 grid grid-cols-12 gap-5 items-start"
                    style={{ borderBottom: '1px solid var(--border-hairline)' }}
                  >
                    <div className="col-span-12 lg:col-span-3 space-y-1">
                      <div className="text-[14px] font-semibold">{devGroup.device_display_name}</div>
                      <div className="flex flex-wrap items-center gap-1.5">
                        <span
                          className="px-1.5 py-0.5 rounded-md text-[10.5px]"
                          style={{
                            backgroundColor: 'var(--bg-elevated)',
                            border: '1px solid var(--border-hairline)',
                            color: 'var(--accent-sky)',
                          }}
                        >
                          只读远端快照
                        </span>
                        <span
                          className="text-[10.5px] font-mono"
                          style={{ color: 'var(--text-muted)' }}
                        >
                          {devGroup.device_id.slice(0, 8)}
                        </span>
                      </div>
                    </div>

                    <div className="col-span-12 lg:col-span-9 space-y-3">
                      {devGroup.items.map((snap) => {
                        const pct = Number.isFinite(snap.used_percent)
                          ? Math.max(0, Math.min(100, snap.used_percent))
                          : 0;
                        const resetSecs =
                          snap.resets_at_ms !== null
                            ? Math.max(0, Math.floor((snap.resets_at_ms - nowMs) / 1000))
                            : null;
                        return (
                          <div
                            key={`${snap.device_id}-${snap.plan_id}-${snap.window_id}`}
                            className="space-y-1"
                          >
                            <div className="flex flex-wrap items-center justify-between gap-2 text-[12px] tabular-nums">
                              <div className="flex items-center gap-2 min-w-0">
                                <span
                                  className="inline-block w-2 h-2 rounded-full shrink-0"
                                  style={{ backgroundColor: providerAccentColor(snap.provider_id) }}
                                />
                                <span className="font-medium">
                                  {providerDisplayName(snap.provider_id)} · {snap.plan_name} ·{' '}
                                  {snap.window_label}
                                </span>
                              </div>
                              <div className="flex items-center gap-3">
                                <span style={{ color: 'var(--text-muted)' }}>
                                  {resetSecs !== null ? `重置 ${formatCountdown(resetSecs)} · ` : ''}
                                  剩余 {Math.max(0, 100 - pct).toFixed(1)}% · 同步于{' '}
                                  {formatRelativeTime(snap.updated_at_ms)}
                                </span>
                                <span className="font-semibold">已用 {pct.toFixed(1)}%</span>
                              </div>
                            </div>
                            <InlineProgressBar percent={pct} heightPx={6} />
                          </div>
                        );
                      })}
                    </div>
                  </div>
                ))}
              </div>
            </section>
          )}
        </>
      )}

      {/* Unified Add / Edit Account Drawer */}
      {accountDraft &&
        createPortal(
          <div
            className="drawer-backdrop-enter fixed inset-0 z-50 flex justify-end backdrop-blur-sm"
            style={{ backgroundColor: 'rgba(6, 8, 14, 0.42)' }}
            onClick={(e) => {
              if (e.target === e.currentTarget) {
                setAccountDraft(null);
              }
            }}
          >
            <div
              className="drawer-panel-enter liquid-drawer w-full max-w-[460px] h-full p-5 overflow-y-auto space-y-5"
              style={{
                borderLeft: '1px solid var(--border-strong)',
                color: 'var(--text-primary)',
              }}
            >
              <div
                className="flex items-center justify-between pb-3"
                style={{ borderBottom: '1px solid var(--border-subtle)' }}
              >
                <h2 className="text-[14.5px] font-semibold">
                  {accountDraft.mode === 'create' ? '添加账户' : '编辑账户'}
                </h2>
                <button
                  type="button"
                  onClick={() => setAccountDraft(null)}
                  className="btn-interactive p-1.5 rounded-lg cursor-pointer"
                  style={{ color: 'var(--text-secondary)' }}
                  aria-label="关闭抽屉"
                >
                  <X className="w-4 h-4" />
                </button>
              </div>

              {accountDraft.mode === 'create' && (
                <div className="seg-group w-full">
                  <button
                    type="button"
                    data-active={accountDraft.connectKind === 'login' ? 'true' : undefined}
                    onClick={() => {
                      setAccountFormError(null);
                      setAccountDraft((curr) =>
                        curr
                          ? {
                              ...curr,
                              connectKind: 'login',
                              provider_id: 'antigravity',
                            }
                          : curr
                      );
                    }}
                    className="seg-pill flex-1 py-1.5 text-[12px] cursor-pointer"
                    style={{
                      color:
                        accountDraft.connectKind === 'login'
                          ? 'var(--text-primary)'
                          : 'var(--text-secondary)',
                      fontWeight: accountDraft.connectKind === 'login' ? 500 : 400,
                    }}
                  >
                    提供商账号登录
                  </button>
                  <button
                    type="button"
                    data-active={accountDraft.connectKind === 'api' ? 'true' : undefined}
                    onClick={() => {
                      setAccountFormError(null);
                      setAccountDraft((curr) =>
                        curr
                          ? {
                              ...curr,
                              connectKind: 'api',
                              provider_id: 'deepseek_harness',
                              account_name: defaultAccountName('deepseek_harness'),
                            }
                          : curr
                      );
                    }}
                    className="seg-pill flex-1 py-1.5 text-[12px] cursor-pointer"
                    style={{
                      color:
                        accountDraft.connectKind === 'api'
                          ? 'var(--text-primary)'
                          : 'var(--text-secondary)',
                      fontWeight: accountDraft.connectKind === 'api' ? 500 : 400,
                    }}
                  >
                    API 密钥接入
                  </button>
                </div>
              )}

              {accountDraft.mode === 'create' && accountDraft.connectKind === 'login' ? (
                <div className="space-y-4">
                  <div>
                    <label
                      htmlFor="login-provider"
                      className="block text-[11.5px] mb-1"
                      style={{ color: 'var(--text-secondary)' }}
                    >
                      登录提供商
                    </label>
                    <select
                      id="login-provider"
                      aria-label="登录提供商"
                      value={accountDraft.provider_id}
                      onChange={(e) =>
                        setAccountDraft({
                          ...accountDraft,
                          provider_id: e.target.value,
                        })
                      }
                      className="w-full px-2.5 py-1.5 text-[12px] rounded-lg outline-none cursor-pointer"
                      style={{
                        backgroundColor: 'var(--bg-input)',
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-primary)',
                      }}
                    >
                      {LOGIN_PROVIDERS.map((p) => (
                        <option key={p.id} value={p.id}>
                          {p.label}
                        </option>
                      ))}
                    </select>
                  </div>

                  {accountFormError && (
                    <div
                      className="px-3 py-2 rounded-lg text-[11.5px]"
                      style={{
                        backgroundColor: 'var(--bg-elevated)',
                        border: '1px solid var(--accent-rose)',
                        color: 'var(--accent-rose)',
                      }}
                    >
                      {accountFormError}
                    </div>
                  )}

                  <div
                    className="pt-3 flex flex-wrap items-center gap-2"
                    style={{ borderTop: '1px solid var(--border-subtle)' }}
                  >
                    <button
                      type="button"
                      disabled={syncLoginAccountMutation.isPending}
                      onClick={() => syncLoginAccountMutation.mutate(accountDraft.provider_id)}
                      className="btn-primary inline-flex items-center gap-1.5 px-3.5 py-1.5 rounded-lg text-[12px] font-medium cursor-pointer disabled:opacity-50"
                      style={{
                        backgroundColor: 'var(--accent-primary)',
                        color: '#ffffff',
                      }}
                    >
                      <RefreshCw
                        className={`w-3.5 h-3.5 ${
                          syncLoginAccountMutation.isPending ? 'animate-spin' : ''
                        }`}
                      />
                      <span>
                        {syncLoginAccountMutation.isPending ? '同步中…' : '同步账户用量'}
                      </span>
                    </button>

                    <button
                      type="button"
                      disabled={launchLoginMutation.isPending}
                      onClick={() => launchLoginMutation.mutate(accountDraft.provider_id)}
                      className="btn-interactive bg-[var(--bg-elevated)] inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] cursor-pointer disabled:opacity-50"
                      style={{
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-primary)',
                      }}
                    >
                      <ExternalLink className="w-3.5 h-3.5" />
                      <span>启动终端登录</span>
                    </button>

                    <button
                      type="button"
                      onClick={() => setAccountDraft(null)}
                      className="btn-interactive bg-[var(--bg-elevated)] px-3 py-1.5 rounded-lg text-[12px] cursor-pointer ml-auto"
                      style={{
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-secondary)',
                      }}
                    >
                      取消
                    </button>
                  </div>
                </div>
              ) : (
                <div className="space-y-4">
                  <div>
                    <label
                      htmlFor="account-provider"
                      className="block text-[11.5px] mb-1"
                      style={{ color: 'var(--text-secondary)' }}
                    >
                      模型提供商
                    </label>
                    <select
                      id="account-provider"
                      aria-label="API 提供商"
                      value={accountDraft.provider_id}
                      onChange={(e) => {
                        const nextPid = e.target.value;
                        setAccountDraft({
                          ...accountDraft,
                          provider_id: nextPid,
                          account_name:
                            accountDraft.mode === 'create'
                              ? defaultAccountName(nextPid)
                              : accountDraft.account_name,
                        });
                      }}
                      className="w-full px-2.5 py-1.5 text-[12px] rounded-lg outline-none cursor-pointer"
                      style={{
                        backgroundColor: 'var(--bg-input)',
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-primary)',
                      }}
                    >
                      {API_PROVIDERS.map((p) => (
                        <option key={p.id} value={p.id}>
                          {p.label}
                        </option>
                      ))}
                    </select>
                  </div>

                  <div>
                    <label
                      htmlFor="account-name"
                      className="block text-[11.5px] mb-1"
                      style={{ color: 'var(--text-secondary)' }}
                    >
                      账户名称
                    </label>
                    <input
                      id="account-name"
                      type="text"
                      placeholder="如 DeepSeek 官方"
                      value={accountDraft.account_name}
                      onChange={(e) =>
                        setAccountDraft({ ...accountDraft, account_name: e.target.value })
                      }
                      className="w-full px-2.5 py-1.5 text-[12px] rounded-lg outline-none"
                      style={{
                        backgroundColor: 'var(--bg-input)',
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-primary)',
                      }}
                    />
                  </div>

                  {API_KEY_CAPABLE_PROVIDERS.has(accountDraft.provider_id) && (
                    <div>
                      <label
                        htmlFor="account-api-key"
                        className="block text-[11.5px] mb-1"
                        style={{ color: 'var(--text-secondary)' }}
                      >
                        官方 API Key
                      </label>
                      <input
                        id="account-api-key"
                        type="password"
                        autoComplete="off"
                        placeholder={
                          hasCredentialForProvider(accountDraft.provider_id)
                            ? '输入新值可替换'
                            : 'sk-...'
                        }
                        value={accountDraft.api_key_input}
                        onChange={(e) =>
                          setAccountDraft({ ...accountDraft, api_key_input: e.target.value })
                        }
                        className="w-full px-2.5 py-1.5 text-[12px] rounded-lg outline-none font-mono"
                        style={{
                          backgroundColor: 'var(--bg-input)',
                          border: '1px solid var(--border-subtle)',
                          color: 'var(--text-primary)',
                        }}
                      />
                    </div>
                  )}

                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                    <div>
                      <label
                        htmlFor="account-manual-balance"
                        className="block text-[11.5px] mb-1"
                        style={{ color: 'var(--text-secondary)' }}
                      >
                        起始余额 ({currency})
                      </label>
                      <input
                        id="account-manual-balance"
                        type="text"
                        inputMode="decimal"
                        placeholder="0.00"
                        value={accountDraft.manual_balance_input}
                        onChange={(e) =>
                          setAccountDraft({
                            ...accountDraft,
                            manual_balance_input: e.target.value,
                          })
                        }
                        className="w-full px-2.5 py-1.5 text-[12px] rounded-lg tabular-nums outline-none"
                        style={{
                          backgroundColor: 'var(--bg-input)',
                          border: '1px solid var(--border-subtle)',
                          color: 'var(--text-primary)',
                        }}
                      />
                    </div>

                    <div>
                      <label
                        htmlFor="account-monthly-budget"
                        className="block text-[11.5px] mb-1"
                        style={{ color: 'var(--text-secondary)' }}
                      >
                        月度上限 ({currency})
                      </label>
                      <input
                        id="account-monthly-budget"
                        type="text"
                        inputMode="decimal"
                        placeholder="50"
                        value={accountDraft.monthly_budget_input}
                        onChange={(e) =>
                          setAccountDraft({
                            ...accountDraft,
                            monthly_budget_input: e.target.value,
                          })
                        }
                        className="w-full px-2.5 py-1.5 text-[12px] rounded-lg tabular-nums outline-none"
                        style={{
                          backgroundColor: 'var(--bg-input)',
                          border: '1px solid var(--border-subtle)',
                          color: 'var(--text-primary)',
                        }}
                      />
                    </div>
                  </div>

                  {accountFormError && (
                    <div
                      className="px-3 py-2 rounded-lg text-[11.5px]"
                      style={{
                        backgroundColor: 'var(--bg-elevated)',
                        border: '1px solid var(--accent-rose)',
                        color: 'var(--accent-rose)',
                      }}
                    >
                      {accountFormError}
                    </div>
                  )}

                  <div
                    className="pt-3 flex items-center gap-2"
                    style={{ borderTop: '1px solid var(--border-subtle)' }}
                  >
                    <button
                      type="button"
                      disabled={saveAccountMutation.isPending}
                      onClick={handleSaveApiAccount}
                      className="btn-primary px-3.5 py-1.5 rounded-lg text-[12px] font-medium cursor-pointer disabled:opacity-50"
                      style={{
                        backgroundColor: 'var(--accent-primary)',
                        color: '#ffffff',
                      }}
                    >
                      {saveAccountMutation.isPending ? '保存中…' : '保存账户'}
                    </button>
                    <button
                      type="button"
                      onClick={() => setAccountDraft(null)}
                      className="btn-interactive bg-[var(--bg-elevated)] px-3 py-1.5 rounded-lg text-[12px] cursor-pointer"
                      style={{
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-secondary)',
                      }}
                    >
                      取消
                    </button>
                  </div>
                </div>
              )}
            </div>
          </div>,
          document.body
        )}
    </div>
  );
}
