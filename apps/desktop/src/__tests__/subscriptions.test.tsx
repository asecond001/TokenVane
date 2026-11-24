import { act, cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TitleBar } from '../components/TitleBar';
import { ipc } from '../lib/ipc';
import { DataSourcesPage } from '../pages/DataSourcesPage';
import { OverviewPage } from '../pages/OverviewPage';
import { SubscriptionsPage } from '../pages/SubscriptionsPage';
import { TrayPopupPage } from '../pages/TrayPopupPage';
import { useWorkspaceStore } from '../stores/workspaceStore';
import type {
  ObservedRateLimitWindow,
  ProviderConfig,
  SubscriptionPlanStatus,
  SubscriptionPlanTemplate,
} from '../types/ipc';
import {
  renderWithQuery,
  makePreferences,
  makeTraySummary,
  makeOverviewReport,
  makePlanStatus,
  makeTemplates,
  resetWorkspaceState,
} from './testUtils';

describe('Usage page unified account connection, observed rate limits & quota deletion', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('shows clean positive empty state and renders authoritative observed rate limits without bind button', async () => {
    const observed: ObservedRateLimitWindow[] = [
      {
        provider_id: 'codex',
        window_name: '5h',
        used_percent: 34.5,
        window_minutes: 300,
        resets_at_ms: Date.now() + 7200_000,
        observed_at_ms: Date.now() - 60_000,
      },
    ];
    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue(observed);

    renderWithQuery(<SubscriptionsPage />);

    expect(await screen.findByRole('heading', { name: '用量' })).toBeInTheDocument();
    expect(screen.getAllByRole('button', { name: '添加账户' })).toHaveLength(1);
    expect(await screen.findByText('配额窗口')).toBeInTheDocument();
    expect(screen.getByText('官方实时限额')).toBeInTheDocument();
    expect(screen.getByText('5h 官方限额')).toBeInTheDocument();
    expect(screen.getByText('34.5%')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '绑定订阅' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '添加我的订阅' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '添加按量计费' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '同步套餐目录' })).not.toBeInTheDocument();
  });

  it('connects provider CLI account via unified Add Account login drawer', async () => {
    const now = Date.now();
    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue([]);
    const refreshSpy = vi.spyOn(ipc, 'refreshProviderBalance').mockResolvedValue({
      snapshot: {
        provider_id: 'antigravity',
        currency: 'QUOTA',
        total_balance_micro: null,
        granted_balance_micro: null,
        topped_up_balance_micro: null,
        daily_quota_used_micro: null,
        daily_quota_limit_micro: null,
        fetched_at_ms: now,
      },
      rate_limits_updated: 4,
      endpoint: 'agy -p "/usage" --output-format json',
      from_cache: false,
      cooldown_remaining_secs: 0,
    });
    const loginSpy = vi
      .spyOn(ipc, 'launchProviderCliLogin')
      .mockResolvedValue('已在系统终端打开登录窗口，完成授权后点击同步即可');

    renderWithQuery(<SubscriptionsPage />);

    expect(
      await screen.findByText('添加账户以同步配额与余额')
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '添加账户' }));
    expect(await screen.findByRole('heading', { name: '添加账户' })).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '启动终端登录' }));
    await waitFor(() => {
      expect(loginSpy).toHaveBeenCalledWith('antigravity');
    });

    fireEvent.click(screen.getByRole('button', { name: '同步账户用量' }));
    await waitFor(() => {
      expect(refreshSpy).toHaveBeenCalledWith('antigravity');
    });
  });

  it('deletes an authoritative quota entry after inline confirmation and hides bound observed snapshots', async () => {
    const plan = makePlanStatus();
    const observed: ObservedRateLimitWindow[] = [
      {
        provider_id: 'claude_code',
        window_name: '5h',
        used_percent: 25,
        window_minutes: 300,
        resets_at_ms: Date.now() + 3600_000,
        observed_at_ms: Date.now() - 30_000,
      },
    ];
    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([plan]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue(observed);
    const deleteSpy = vi.spyOn(ipc, 'deleteSubscriptionPlan').mockResolvedValue(undefined);

    renderWithQuery(<SubscriptionsPage />);

    expect(await screen.findByText('1 项数据源')).toBeInTheDocument();
    expect(screen.getByText('Claude Max 5x')).toBeInTheDocument();
    // Bound provider (claude_code) must not duplicate inside unbound observed rows
    expect(screen.queryByText('官方实时限额')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '删除 Claude Max 5x' }));
    fireEvent.click(screen.getByRole('button', { name: '确认' }));

    await waitFor(() => {
      expect(deleteSpy).toHaveBeenCalledWith('claude_max_5x');
    });
  });
});

describe('Bug fixes verification: Overview today shares, Kimi QUOTA display & Live/Auto Subscriptions', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders fixed today share headings and subtitle in OverviewPage', async () => {
    useWorkspaceStore.getState().setTimePreset('30d');
    vi.spyOn(ipc, 'getOverview').mockResolvedValue(
      makeOverviewReport({
        provider_shares: [
          {
            key: 'claude_code',
            label: 'Claude Code',
            tokens: 45_000,
            token_share_percent: 100,
            cost_micro_usd: 900_000,
            cost_share_percent: 100,
          },
        ],
        top_model_shares: [
          {
            key: 'claude-sonnet-4-5',
            label: 'claude-sonnet-4-5',
            tokens: 45_000,
            token_share_percent: 100,
            cost_micro_usd: 900_000,
            cost_share_percent: 100,
          },
        ],
      })
    );

    renderWithQuery(<OverviewPage />);

    expect(await screen.findByRole('columnheader', { name: 'Token 占比' })).toBeInTheDocument();
    expect(screen.getByText('今日 Token 占比')).toBeInTheDocument();
    expect(screen.getByRole('columnheader', { name: 'API 等效费用' })).toBeInTheDocument();
    expect(screen.getByText('今日主力模型 Top 3')).toBeInTheDocument();
    expect(screen.getByText('今日实时概览')).toBeInTheDocument();
  });

  it('renders auto-discovered official API plans, live reference tiers, and converts open drawer currency reactively', async () => {
    const now = Date.now();
    const autoPlan: SubscriptionPlanStatus = {
      plan: {
        plan_id: 'auto_kimi_code_kimi_code_allegretto',
        provider_id: 'kimi_code',
        plan_name: 'Kimi Code Allegretto',
        enabled: true,
        monthly_fee_micro_usd: 7_000_000,
        is_user_configured: true,
        auto_source: 'official_api',
        updated_at_ms: now,
        windows: [
          {
            window_id: '5h',
            label: '5h 滚动窗口',
            duration_hours: 5,
            rolling: true,
            unit: 'micro_usd',
            limit_value: 8_000_000,
            warning_percent: 80,
            critical_percent: 95,
          },
        ],
      },
      windows: [
        {
          window_id: '5h',
          label: '5h 滚动窗口',
          duration_hours: 5,
          rolling: true,
          unit: 'micro_usd',
          used_value: 1_920_000,
          limit_value: 8_000_000,
          used_percent: 24.0,
          warning_percent: 80,
          critical_percent: 95,
          burn_rate_per_hour: 384_000,
          projected_exhaustion_hours: null,
          resets_at_ms: now + 3_600_000,
          resets_in_seconds: 3_600,
          source: 'official_api',
          alert_level: 'normal',
        },
      ],
      today_cost_micro_usd: 1_200_000,
      week_cost_micro_usd: 5_000_000,
      month_cost_micro_usd: 12_000_000,
      month_tokens: 450_000,
    };

    const liveTemplates: SubscriptionPlanTemplate[] = makeTemplates().map((tpl) =>
      tpl.template_id === 'claude_max_5x'
        ? {
            ...tpl,
            live_windows: [
              {
                window_id: '5h',
                label: '5h 滚动窗口',
                duration_hours: 5,
                rolling: true,
                unit: 'micro_usd',
                used_value: 18_750_000,
                limit_value: 75_000_000,
                used_percent: 25.0,
                warning_percent: 80,
                critical_percent: 95,
                burn_rate_per_hour: 3_750_000,
                projected_exhaustion_hours: 15.0,
                resets_at_ms: now + 5_400_000,
                resets_in_seconds: 5_400,
                source: 'local_estimate',
                alert_level: 'normal',
              },
            ],
          }
        : tpl
    );

    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([autoPlan]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue([]);
    vi.spyOn(ipc, 'getSubscriptionTemplates').mockResolvedValue(liveTemplates);
    vi.spyOn(ipc, 'getRemoteSubscriptionSnapshots').mockResolvedValue([]);
    vi.spyOn(ipc, 'getPaygAccounts').mockResolvedValue([]);
    vi.spyOn(ipc, 'getProviderConfigs').mockResolvedValue([]);

    renderWithQuery(<SubscriptionsPage />);

    expect(await screen.findByText('自动获取 · 官方 API')).toBeInTheDocument();
    expect(screen.getAllByText('官方 API 实时').length).toBeGreaterThanOrEqual(1);
    expect(screen.queryByText('各档位参考配额实时对照')).not.toBeInTheDocument();
    expect(screen.getByText('24.0%')).toBeInTheDocument();

    // Open unified Add Account drawer, switch to API mode & verify reactive USD -> CNY conversion
    fireEvent.click(screen.getByRole('button', { name: '添加账户' }));
    fireEvent.click(await screen.findByRole('button', { name: 'API 密钥接入' }));

    const prepaidInput = screen.getByLabelText(/起始余额/) as HTMLInputElement;
    fireEvent.change(prepaidInput, { target: { value: '100' } });

    act(() => {
      useWorkspaceStore.getState().setCurrency('CNY');
    });

    await waitFor(() => {
      expect((screen.getByLabelText(/起始余额/) as HTMLInputElement).value).toBe('725');
    });
  });

  it('lets the user pick a catalog tier and pauses a plan without customizing it', async () => {
    const now = Date.now();
    const autoPlan: SubscriptionPlanStatus = {
      plan: {
        plan_id: 'auto_kimi_code',
        provider_id: 'kimi_code',
        plan_name: 'Kimi Code 会员订阅',
        enabled: true,
        monthly_fee_micro_usd: 0,
        is_user_configured: true,
        auto_source: 'official_api',
        updated_at_ms: now,
        windows: [],
      },
      windows: [
        {
          window_id: '5h',
          label: '5h 滚动窗口',
          duration_hours: 5,
          rolling: true,
          unit: 'tokens',
          used_value: 10,
          limit_value: 100,
          used_percent: 10.0,
          warning_percent: 80,
          critical_percent: 95,
          burn_rate_per_hour: 0,
          projected_exhaustion_hours: null,
          resets_at_ms: now + 3_600_000,
          resets_in_seconds: 3_600,
          source: 'official_api',
          alert_level: 'normal',
        },
      ],
      today_cost_micro_usd: 0,
      week_cost_micro_usd: 0,
      month_cost_micro_usd: 0,
      month_tokens: 0,
    };
    const kimiTiers: SubscriptionPlanTemplate[] = [
      {
        template_id: 'kimi_code_plus',
        provider_id: 'kimi_code',
        plan_name: 'Kimi Plus (Moderato)',
        monthly_fee_micro_usd: 19_000_000,
        windows: [],
        description: '',
      },
      {
        template_id: 'kimi_code_allegretto',
        provider_id: 'kimi_code',
        plan_name: 'Kimi Pro (Allegretto)',
        monthly_fee_micro_usd: 39_000_000,
        windows: [],
        description: '',
      },
    ];

    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([autoPlan]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue([]);
    vi.spyOn(ipc, 'getSubscriptionTemplates').mockResolvedValue(kimiTiers);
    vi.spyOn(ipc, 'getRemoteSubscriptionSnapshots').mockResolvedValue([]);
    vi.spyOn(ipc, 'getPaygAccounts').mockResolvedValue([]);
    vi.spyOn(ipc, 'getProviderConfigs').mockResolvedValue([]);
    const upsertSpy = vi.spyOn(ipc, 'upsertSubscriptionPlan').mockResolvedValue(undefined);
    const enableSpy = vi.spyOn(ipc, 'setSubscriptionPlanEnabled').mockResolvedValue(undefined);

    renderWithQuery(<SubscriptionsPage />);

    const picker = (await screen.findByLabelText('Kimi Code 会员订阅 订阅档位')) as HTMLSelectElement;
    expect(picker.value).toBe('');
    fireEvent.change(picker, { target: { value: 'kimi_code_allegretto' } });

    await waitFor(() => {
      expect(upsertSpy).toHaveBeenCalledWith(
        expect.objectContaining({
          plan_id: 'auto_kimi_code',
          plan_name: 'Kimi Pro (Allegretto)',
          monthly_fee_micro_usd: 39_000_000,
          auto_source: 'official_api',
        })
      );
    });

    fireEvent.click(screen.getByRole('button', { name: '已启用' }));
    await waitFor(() => {
      expect(enableSpy).toHaveBeenCalledWith('auto_kimi_code', false);
    });
    expect(upsertSpy).toHaveBeenCalledTimes(1);
  });

  it('renders Kimi Code QUOTA snapshot cleanly in DataSourcesPage', async () => {
    const now = Date.now();
    const configs: ProviderConfig[] = [
      {
        provider_id: 'kimi_code',
        enabled: true,
        collection_mode: 'official_api',
        custom_log_paths: [],
        has_credential: true,
        last_sync_at_ms: now - 5_000,
        last_error: null,
        updated_at_ms: now,
      },
    ];

    vi.spyOn(ipc, 'getProviderConfigs').mockResolvedValue(configs);
    vi.spyOn(ipc, 'getIngestionCursors').mockResolvedValue([]);
    vi.spyOn(ipc, 'listCustomProviderRules').mockResolvedValue([]);
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(makePreferences());
    vi.spyOn(ipc, 'getProviderBalances').mockResolvedValue([
      {
        provider_id: 'kimi_code',
        currency: 'QUOTA',
        total_balance_micro: null,
        granted_balance_micro: null,
        topped_up_balance_micro: null,
        daily_quota_used_micro: 18,
        daily_quota_limit_micro: 100,
        fetched_at_ms: now,
      },
    ]);

    renderWithQuery(<DataSourcesPage />);

    expect(
      await screen.findByText('Kimi Code 实时配额已同步至「用量」')
    ).toBeInTheDocument();
    expect(screen.getByText('周期配额: 18 / 100')).toBeInTheDocument();
    expect(screen.queryByText(/QUOTA 0\.00/)).not.toBeInTheDocument();
  });
});

describe('v0.2.5 Pay-As-You-Go, Credits & Floating Toggle', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    useWorkspaceStore.setState({
      activePage: 'subscriptions',
      providerFilter: 'all',
      deviceFilter: 'all',
      timePreset: '7d',
      currency: 'USD',
      usdToCnyRate: '7.25',
      theme: 'dark',
      resolvedTheme: 'dark',
      tzOffsetMinutes: 480,
    });
  });

  it('renders DeepSeek CNY balance + Codex Credits and creates a PAYG account in SubscriptionsPage', async () => {
    const now = Date.now();
    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue([]);
    vi.spyOn(ipc, 'getSubscriptionTemplates').mockResolvedValue([]);
    vi.spyOn(ipc, 'getRemoteSubscriptionSnapshots').mockResolvedValue([]);
    vi.spyOn(ipc, 'getProviderConfigs').mockResolvedValue([
      {
        provider_id: 'deepseek_harness',
        enabled: true,
        collection_mode: 'official_api',
        custom_log_paths: [],
        has_credential: true,
        last_sync_at_ms: now,
        last_error: null,
        updated_at_ms: now,
      },
    ]);
    vi.spyOn(ipc, 'getPaygAccounts').mockResolvedValue([
      {
        account: {
          account_id: 'auto_deepseek_harness',
          provider_id: 'deepseek_harness',
          account_name: 'DeepSeek 官方账户余额',
          enabled: true,
          manual_balance_micro_usd: null,
          balance_anchor_ms: null,
          monthly_budget_micro_usd: 50_000_000,
          is_auto: true,
          updated_at_ms: now,
        },
        official_balance: {
          provider_id: 'deepseek_harness',
          currency: 'CNY',
          total_balance_micro: 45_200_000,
          granted_balance_micro: 10_000_000,
          topped_up_balance_micro: 35_200_000,
          daily_quota_used_micro: null,
          daily_quota_limit_micro: null,
          fetched_at_ms: now - 5_000,
        },
        remaining_manual_balance_micro_usd: null,
        today_cost_micro_usd: 1_200_000,
        month_cost_micro_usd: 12_500_000,
        budget_used_percent: 25.0,
      },
      {
        account: {
          account_id: 'auto_codex',
          provider_id: 'codex',
          account_name: 'Codex Credits',
          enabled: true,
          manual_balance_micro_usd: null,
          balance_anchor_ms: null,
          monthly_budget_micro_usd: null,
          is_auto: true,
          updated_at_ms: now,
        },
        official_balance: {
          provider_id: 'codex',
          currency: 'CREDITS',
          total_balance_micro: 766_760_000,
          granted_balance_micro: null,
          topped_up_balance_micro: null,
          daily_quota_used_micro: null,
          daily_quota_limit_micro: null,
          fetched_at_ms: now - 2_000,
        },
        remaining_manual_balance_micro_usd: null,
        today_cost_micro_usd: 2_000_000,
        month_cost_micro_usd: 8_000_000,
        budget_used_percent: null,
      },
    ]);

    const setKeySpy = vi.spyOn(ipc, 'setProviderApiKey').mockResolvedValue(undefined);
    const upsertPaygSpy = vi.spyOn(ipc, 'upsertPaygAccount').mockResolvedValue(undefined);

    renderWithQuery(<SubscriptionsPage />);

    expect(await screen.findByText('DeepSeek 官方账户余额')).toBeInTheDocument();
    expect(screen.getByText('¥45.20')).toBeInTheDocument();
    expect(screen.getByText('766.76 Credits')).toBeInTheDocument();
    expect(screen.queryByText('各档位参考配额实时对照')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '添加账户' }));
    fireEvent.click(await screen.findByRole('button', { name: 'API 密钥接入' }));
    expect(document.querySelector('body > .drawer-backdrop-enter')).not.toBeNull();

    const keyInput = screen.getByLabelText(/官方 API Key/) as HTMLInputElement;
    fireEvent.change(keyInput, { target: { value: 'sk-deepseek-test-123' } });
    const budgetInput = screen.getByLabelText(/月度上限/) as HTMLInputElement;
    fireEvent.change(budgetInput, { target: { value: '80' } });

    fireEvent.click(screen.getByRole('button', { name: '保存账户' }));

    await waitFor(() => {
      expect(upsertPaygSpy).toHaveBeenCalledWith(
        expect.objectContaining({
          provider_id: 'deepseek_harness',
          monthly_budget_micro_usd: 80_000_000,
        })
      );
      expect(setKeySpy).toHaveBeenCalledWith('deepseek_harness', 'sk-deepseek-test-123');
    });
  });

  it('reflects active floating window state in TitleBar and toggles via IPC', async () => {
    vi.spyOn(ipc, 'listCustomProviderRules').mockResolvedValue([]);
    vi.spyOn(ipc, 'listDevices').mockResolvedValue([]);
    const toggleSpy = vi.spyOn(ipc, 'toggleFloatingWindow').mockResolvedValue(true);

    renderWithQuery(
      <TitleBar
        collectorWatching={true}
        isScanning={false}
        floatingEnabled={false}
        onScanNow={vi.fn()}
      />
    );

    const btn = screen.getByRole('button', { name: '切换桌面悬浮窗' });
    expect(btn).toHaveAttribute('aria-pressed', 'false');
    fireEvent.click(btn);

    await waitFor(() => {
      expect(toggleSpy).toHaveBeenCalledTimes(1);
    });
  });
});

describe('v0.2.7 Unified 用量 Account Hub & Positive UI Copy', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('omits windowless manual plans and exposes single 添加账户 entry point with login/API modes', async () => {
    const now = Date.now();
    const windowlessPlan: SubscriptionPlanStatus = {
      plan: {
        plan_id: 'manual_cursor',
        provider_id: 'cursor',
        plan_name: 'Cursor Pro (Manual)',
        enabled: true,
        monthly_fee_micro_usd: 20_000_000,
        is_user_configured: true,
        auto_source: null,
        updated_at_ms: now,
        windows: [],
      },
      windows: [],
      today_cost_micro_usd: 5_000_000,
      week_cost_micro_usd: 20_000_000,
      month_cost_micro_usd: 50_000_000,
      month_tokens: 1_000_000,
    };

    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([windowlessPlan]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue([]);
    vi.spyOn(ipc, 'getRemoteSubscriptionSnapshots').mockResolvedValue([]);
    vi.spyOn(ipc, 'getPaygAccounts').mockResolvedValue([]);
    vi.spyOn(ipc, 'getProviderConfigs').mockResolvedValue([]);

    renderWithQuery(<SubscriptionsPage />);

    expect(
      await screen.findByText('添加账户以同步配额与余额')
    ).toBeInTheDocument();
    // Windowless manual plan must not be displayed
    expect(screen.queryByText('Cursor Pro (Manual)')).not.toBeInTheDocument();
    expect(screen.queryByRole('progressbar')).not.toBeInTheDocument();
    expect(screen.getAllByRole('button', { name: '添加账户' })).toHaveLength(1);

    fireEvent.click(screen.getByRole('button', { name: '添加账户' }));
    expect(document.querySelector('body > .drawer-backdrop-enter')).not.toBeNull();
    expect(screen.getByRole('button', { name: '提供商账号登录' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'API 密钥接入' })).toBeInTheDocument();
  });

  it('hides non-API plans without official windows from TrayPopupPage quota section', async () => {
    const now = Date.now();
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(makePreferences());
    vi.spyOn(ipc, 'getTraySummary').mockResolvedValue(
      makeTraySummary({
        subscriptions: [
          {
            plan: {
              plan_id: 'claude_max_5x',
              provider_id: 'claude_code',
              plan_name: 'Claude Max 5x',
              enabled: true,
              monthly_fee_micro_usd: 100_000_000,
              windows: [],
              is_user_configured: true,
              updated_at_ms: now,
            },
            windows: [],
            today_cost_micro_usd: 5_000_000,
            week_cost_micro_usd: 15_000_000,
            month_cost_micro_usd: 45_000_000,
            month_tokens: 500_000,
          },
          makePlanStatus({
            plan: {
              plan_id: 'codex_plus',
              provider_id: 'codex',
              plan_name: 'ChatGPT Plus / Codex',
              enabled: true,
              monthly_fee_micro_usd: 20_000_000,
              windows: [],
              is_user_configured: true,
              updated_at_ms: now,
            },
            windows: [
              {
                window_id: '5h',
                label: '5h 官方限额',
                duration_hours: 5,
                rolling: true,
                unit: 'micro_usd',
                used_value: 42,
                limit_value: 100,
                used_percent: 42.0,
                warning_percent: 80,
                critical_percent: 95,
                burn_rate_per_hour: 10,
                projected_exhaustion_hours: null,
                resets_at_ms: now + 3600_000,
                resets_in_seconds: 3600,
                source: 'official_log_snapshot',
                alert_level: 'normal',
              },
            ],
          }),
        ],
      })
    );

    renderWithQuery(<TrayPopupPage />);

    expect(await screen.findByText('Codex · 5h 官方限额')).toBeInTheDocument();
    expect(screen.queryByText('等待官方配额同步')).not.toBeInTheDocument();
  });

  it('renders Antigravity official_cli pool windows and percentage readout in SubscriptionsPage', async () => {
    const now = Date.now();
    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([
      {
        plan: {
          plan_id: 'auto_antigravity',
          provider_id: 'antigravity',
          plan_name: 'Google AI Ultra',
          enabled: true,
          monthly_fee_micro_usd: 199_990_000,
          is_user_configured: true,
          auto_source: 'official_cli',
          updated_at_ms: now,
          windows: [],
        },
        windows: [
          {
            window_id: 'gemini-5h',
            label: 'Gemini 模型池 · 5h 限额',
            duration_hours: 5,
            rolling: true,
            unit: 'tokens',
            used_value: 37,
            limit_value: 100,
            used_percent: 36.5,
            warning_percent: 80,
            critical_percent: 95,
            burn_rate_per_hour: 0,
            projected_exhaustion_hours: null,
            resets_at_ms: now + 3_600_000,
            resets_in_seconds: 3600,
            source: 'official_cli',
            alert_level: 'normal',
          },
          {
            window_id: '3p-weekly',
            label: 'Claude / GPT 模型池 · 7d 周限额',
            duration_hours: 168,
            rolling: true,
            unit: 'tokens',
            used_value: 0,
            limit_value: 100,
            used_percent: 0.5,
            warning_percent: 80,
            critical_percent: 95,
            burn_rate_per_hour: 0,
            projected_exhaustion_hours: null,
            resets_at_ms: now + 86_400_000,
            resets_in_seconds: 86400,
            source: 'official_cli',
            alert_level: 'normal',
          },
        ],
        today_cost_micro_usd: 2_000_000,
        week_cost_micro_usd: 10_000_000,
        month_cost_micro_usd: 40_000_000,
        month_tokens: 500_000,
      },
    ]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue([]);
    vi.spyOn(ipc, 'getSubscriptionTemplates').mockResolvedValue([]);

    renderWithQuery(<SubscriptionsPage />);

    expect(await screen.findByText('Google AI Ultra')).toBeInTheDocument();
    expect(screen.getByText('自动获取 · 官方 CLI')).toBeInTheDocument();
    expect(screen.getByText('Gemini 模型池 · 5h 限额')).toBeInTheDocument();
    expect(screen.getByText('Claude / GPT 模型池 · 7d 周限额')).toBeInTheDocument();
    expect(screen.getAllByText('官方 CLI 实时').length).toBe(2);
    expect(screen.getByText('已用 36.5%')).toBeInTheDocument();
    expect(screen.getByText('已用 0.5%')).toBeInTheDocument();
    // Right column keeps a single reset countdown line per window, nothing else.
    const resetLines = screen.getAllByTestId('quota-reset-line');
    expect(resetLines.length).toBe(2);
    for (const line of resetLines) {
      expect(line.textContent).toMatch(/后重置$/);
    }
    expect(screen.queryByText(/剩余 63\.5%/)).not.toBeInTheDocument();
  });

  it('renders Antigravity CLI quota refresh button and synced readout in DataSourcesPage without API key', async () => {
    const now = Date.now();
    const configs: ProviderConfig[] = [
      {
        provider_id: 'antigravity',
        enabled: true,
        collection_mode: 'local_log',
        custom_log_paths: [],
        has_credential: false,
        last_sync_at_ms: now - 2_000,
        last_error: null,
        updated_at_ms: now,
      },
    ];

    vi.spyOn(ipc, 'getProviderConfigs').mockResolvedValue(configs);
    vi.spyOn(ipc, 'getIngestionCursors').mockResolvedValue([]);
    vi.spyOn(ipc, 'listCustomProviderRules').mockResolvedValue([]);
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(makePreferences());
    vi.spyOn(ipc, 'getProviderBalances').mockResolvedValue([
      {
        provider_id: 'antigravity',
        currency: 'QUOTA',
        total_balance_micro: null,
        granted_balance_micro: null,
        topped_up_balance_micro: null,
        daily_quota_used_micro: null,
        daily_quota_limit_micro: null,
        fetched_at_ms: now,
      },
    ]);
    const refreshSpy = vi.spyOn(ipc, 'refreshProviderBalance').mockResolvedValue({
      snapshot: {
        provider_id: 'antigravity',
        currency: 'QUOTA',
        total_balance_micro: null,
        granted_balance_micro: null,
        topped_up_balance_micro: null,
        daily_quota_used_micro: null,
        daily_quota_limit_micro: null,
        fetched_at_ms: now,
      },
      rate_limits_updated: 4,
      endpoint: 'agy -p "/usage" --output-format json',
      from_cache: false,
      cooldown_remaining_secs: 0,
    });

    renderWithQuery(<DataSourcesPage />);

    expect(
      await screen.findByText('Antigravity 官方 CLI 配额已同步至「用量」')
    ).toBeInTheDocument();
    const refreshBtn = screen.getByRole('button', { name: '刷新 CLI 配额' });
    fireEvent.click(refreshBtn);

    await waitFor(() => {
      expect(refreshSpy).toHaveBeenCalledWith('antigravity');
    });
  });
});
