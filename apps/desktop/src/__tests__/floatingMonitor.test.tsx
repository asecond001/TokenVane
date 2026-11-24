import { act, cleanup, fireEvent, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TitleBar } from '../components/TitleBar';
import { ipc } from '../lib/ipc';
import { FloatingMonitorPage } from '../pages/FloatingMonitorPage';
import { OverviewPage } from '../pages/OverviewPage';
import { PreferencesPage } from '../pages/PreferencesPage';
import { SubscriptionsPage } from '../pages/SubscriptionsPage';
import { TrayPopupPage } from '../pages/TrayPopupPage';
import type { SubscriptionPlanStatus } from '../types/ipc';
import {
  renderWithQuery,
  makePreferences,
  makeBootstrap,
  makeTraySummary,
  makeOverviewReport,
  resetWorkspaceState,
} from './testUtils';

describe('v0.2.2 Floating Monitor, Session Titles & Subscription Tier Labels', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders FloatingMonitorPage with live token counter, session title, compact toggle, and close IPCs', async () => {
    const now = Date.now();
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(
      makePreferences({ floating_window_enabled: true })
    );
    vi.spyOn(ipc, 'getTraySummary').mockResolvedValue(
      makeTraySummary({
        today_tokens: 215_000,
        today_cost_micro_usd: 3_400_000,
        latest_output_tps: 78.4,
        active_sessions: [
          {
            session_key: '#a1b2c3',
            session_title: 'Refactor Floating Monitor HUD',
            provider_id: 'claude_code',
            model_id: 'claude-sonnet-4-5',
            project_label: 'TokenVane',
            activity_state: 'active',
            started_at_ms: now - 120_000,
            last_active_at_ms: now - 5_000,
            request_count: 8,
            total_tokens: 80_000,
            input_tokens: 60_000,
            output_tokens: 20_000,
            cache_read_tokens: 0,
            estimated_cost_micro_usd: 1_100_000,
            latest_output_tps: 78.4,
          },
        ],
      })
    );
    const compactSpy = vi.spyOn(ipc, 'setFloatingWindowCompact').mockResolvedValue(undefined);
    const toggleSpy = vi.spyOn(ipc, 'toggleFloatingWindow').mockResolvedValue(false);

    renderWithQuery(<FloatingMonitorPage />);

    expect(screen.getByText('TokenVane')).toBeInTheDocument();
    expect(await screen.findByText('Refactor Floating Monitor HUD')).toBeInTheDocument();
    const speedEl = screen.getByText('78.4 tok/s');
    expect(speedEl).toBeInTheDocument();

    // Hovering over the live speed displays the model breakdown tooltip
    fireEvent.mouseEnter(speedEl);
    const speedTooltip = await screen.findByTestId('live-speed-tooltip');
    expect(within(speedTooltip).getByText('实时速率来源模型')).toBeInTheDocument();
    expect(within(speedTooltip).getByText('claude-sonnet-4-5')).toBeInTheDocument();
    fireEvent.mouseLeave(speedEl);

    fireEvent.click(screen.getByRole('button', { name: '切换精简胶囊模式' }));
    expect(compactSpy).toHaveBeenCalledWith(true);
    expect(await screen.findByRole('button', { name: '展开悬浮窗' })).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '关闭悬浮窗' }));
    expect(toggleSpy).toHaveBeenCalled();
  });

  it('displays average speed across multiple active models and lists all active models on hover', async () => {
    const now = Date.now();
    window.localStorage.removeItem('tokenvane.floating_config.v1');
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(
      makePreferences({ floating_window_enabled: true })
    );
    vi.spyOn(ipc, 'getTraySummary').mockResolvedValue(
      makeTraySummary({
        today_tokens: 320_000,
        today_cost_micro_usd: 4_800_000,
        latest_output_tps: 90.0,
        active_sessions: [
          {
            session_key: '#111111',
            session_title: 'Claude Task',
            provider_id: 'claude_code',
            model_id: 'claude-sonnet-4-5',
            project_label: 'TokenVane',
            activity_state: 'active',
            started_at_ms: now - 60_000,
            last_active_at_ms: now - 5_000,
            request_count: 4,
            total_tokens: 40_000,
            input_tokens: 30_000,
            output_tokens: 10_000,
            cache_read_tokens: 0,
            estimated_cost_micro_usd: 600_000,
            latest_output_tps: 80.0,
          },
          {
            session_key: '#222222',
            session_title: 'Codex Task',
            provider_id: 'codex',
            model_id: 'gpt-5.3-codex',
            project_label: 'TokenVane',
            activity_state: 'active',
            started_at_ms: now - 50_000,
            last_active_at_ms: now - 3_000,
            request_count: 3,
            total_tokens: 30_000,
            input_tokens: 20_000,
            output_tokens: 10_000,
            cache_read_tokens: 0,
            estimated_cost_micro_usd: 500_000,
            latest_output_tps: 100.0,
          },
        ],
      })
    );

    renderWithQuery(<FloatingMonitorPage />);

    // Mean of 80.0 and 100.0 is 90.0 tok/s
    const avgSpeedEl = await screen.findByText('90.0 tok/s');
    expect(avgSpeedEl).toBeInTheDocument();
    expect(screen.getByText('实时生成速率 (2 模型均值)')).toBeInTheDocument();

    fireEvent.mouseEnter(avgSpeedEl);
    const tooltip = await screen.findByTestId('live-speed-tooltip');
    expect(within(tooltip).getByText('活跃模型速率 (2 模型均值)')).toBeInTheDocument();
    expect(within(tooltip).getByText('claude-sonnet-4-5')).toBeInTheDocument();
    expect(within(tooltip).getByText('80.0 tok/s')).toBeInTheDocument();
    expect(within(tooltip).getByText('gpt-5.3-codex')).toBeInTheDocument();
    expect(within(tooltip).getByText('100.0 tok/s')).toBeInTheDocument();
  });

  it('supports categorized PreferencesPage, in-place update buttons, and customizable FloatingMonitorPage elements', async () => {
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(
      makePreferences({ floating_window_enabled: true })
    );
    vi.spyOn(ipc, 'getBootstrapState').mockResolvedValue(makeBootstrap());
    vi.spyOn(ipc, 'listCustomProviderRules').mockResolvedValue([]);
    vi.spyOn(ipc, 'listDevices').mockResolvedValue([]);

    const { useUpdaterStore } = await import('../lib/updater');
    act(() => {
      useUpdaterStore.setState({
        status: 'available',
        availableUpdate: {
          version: '0.2.3',
          currentVersion: '0.2.2',
          date: '2026-11-16',
          body: 'Update test',
        },
        downloadedBytes: 0,
        totalBytes: null,
        errorMessage: null,
      });
    });

    renderWithQuery(
      <div>
        <TitleBar
          collectorWatching={true}
          isScanning={false}
          onScanNow={() => {}}
        />
        <PreferencesPage />
      </div>
    );

    // Top-bar update button in TitleBar
    expect(await screen.findByRole('button', { name: '立即更新客户端' })).toBeInTheDocument();
    expect(screen.getByText('更新 v0.2.3')).toBeInTheDocument();

    // Categorized sections in PreferencesPage
    expect(screen.getByText('外观')).toBeInTheDocument();
    expect(screen.getByText('深色/浅色模式')).toBeInTheDocument();
    expect(screen.getByText('自定义背景')).toBeInTheDocument();
    expect(screen.getByText('桌面与悬浮窗')).toBeInTheDocument();
    expect(screen.getByText('货币与定价')).toBeInTheDocument();
    expect(screen.getByText('系统与更新')).toBeInTheDocument();

    // Check-for-update button is replaced by 立即更新 v0.2.3 when update is available
    expect(screen.getByRole('button', { name: '立即更新 v0.2.3' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '检查更新' })).not.toBeInTheDocument();

    act(() => {
      useUpdaterStore.setState({
        status: 'idle',
        availableUpdate: null,
      });
    });

    expect(await screen.findByRole('button', { name: '检查更新' })).toBeInTheDocument();
  });

  it('displays detected subscription tier status without ever showing 未设月费', async () => {
    const now = Date.now();
    const zeroFeePlans: SubscriptionPlanStatus[] = [
      {
        plan: {
          plan_id: 'auto_kimi_code',
          provider_id: 'kimi_code',
          plan_name: 'Kimi Code Vivace',
          enabled: true,
          monthly_fee_micro_usd: 0,
          is_user_configured: true,
          auto_source: 'official_api',
          updated_at_ms: now,
          windows: [
            {
              window_id: '5h',
              label: '5h 官方限额',
              duration_hours: 5,
              rolling: true,
              unit: 'tokens',
              limit_value: 100,
              warning_percent: 80,
              critical_percent: 95,
            },
          ],
        },
        windows: [
          {
            window_id: '5h',
            label: '5h 官方限额',
            duration_hours: 5,
            rolling: true,
            unit: 'tokens',
            used_value: 25,
            limit_value: 100,
            used_percent: 25,
            warning_percent: 80,
            critical_percent: 95,
            burn_rate_per_hour: 5,
            projected_exhaustion_hours: null,
            resets_at_ms: now + 3600_000,
            resets_in_seconds: 3600,
            source: 'official_api',
            alert_level: 'normal',
          },
        ],
        today_cost_micro_usd: 1_000_000,
        week_cost_micro_usd: 4_000_000,
        month_cost_micro_usd: 9_000_000,
        month_tokens: 200_000,
      },
      {
        plan: {
          plan_id: 'custom_window_plan',
          provider_id: 'qcode',
          plan_name: 'QCode 日额度套餐',
          enabled: true,
          monthly_fee_micro_usd: 0,
          is_user_configured: true,
          auto_source: null,
          updated_at_ms: now,
          windows: [
            {
              window_id: '24h',
              label: '24h 日额度',
              duration_hours: 24,
              rolling: true,
              unit: 'tokens',
              limit_value: 100,
              warning_percent: 80,
              critical_percent: 95,
            },
          ],
        },
        windows: [
          {
            window_id: '24h',
            label: '24h 日额度',
            duration_hours: 24,
            rolling: true,
            unit: 'tokens',
            used_value: 10,
            limit_value: 100,
            used_percent: 10,
            warning_percent: 80,
            critical_percent: 95,
            burn_rate_per_hour: 1,
            projected_exhaustion_hours: null,
            resets_at_ms: null,
            resets_in_seconds: 0,
            source: 'local_log',
            alert_level: 'normal',
          },
        ],
        today_cost_micro_usd: 500_000,
        week_cost_micro_usd: 1_500_000,
        month_cost_micro_usd: 3_000_000,
        month_tokens: 80_000,
      },
    ];

    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue(zeroFeePlans);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue([]);
    vi.spyOn(ipc, 'getSubscriptionTemplates').mockResolvedValue([]);

    renderWithQuery(<SubscriptionsPage />);

    // The name shows as the row title and as the current entry of the tier picker.
    expect((await screen.findAllByText('Kimi Code Vivace')).length).toBeGreaterThan(0);
    expect(screen.getByText('官方同步档位')).toBeInTheDocument();
    expect(screen.getByText('官方配额窗口监测')).toBeInTheDocument();
    expect(screen.queryByText(/未设月费/)).not.toBeInTheDocument();
  });

  it('displays explicit or prompt-derived session_title in OverviewPage and TrayPopupPage', async () => {
    const now = Date.now();
    const titledSession = {
      session_key: '#ef12ab',
      session_title: 'Refactor Chart Stacking',
      provider_id: 'claude_code',
      model_id: 'claude-sonnet-4-5',
      project_label: 'TokenVane',
      activity_state: 'active',
      started_at_ms: now - 60_000,
      last_active_at_ms: now - 3_000,
      request_count: 5,
      total_tokens: 45_000,
      input_tokens: 30_000,
      output_tokens: 15_000,
      cache_read_tokens: 0,
      estimated_cost_micro_usd: 500_000,
      latest_output_tps: 68.5,
    };

    vi.spyOn(ipc, 'getOverview').mockResolvedValue(
      makeOverviewReport({
        active_sessions: [titledSession],
      })
    );
    renderWithQuery(<OverviewPage />);
    expect(await screen.findByText('Refactor Chart Stacking')).toBeInTheDocument();

    cleanup();
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(makePreferences());
    vi.spyOn(ipc, 'getTraySummary').mockResolvedValue(
      makeTraySummary({
        active_sessions: [titledSession],
      })
    );
    renderWithQuery(<TrayPopupPage />);
    expect(await screen.findByText('Refactor Chart Stacking')).toBeInTheDocument();
  });
});
