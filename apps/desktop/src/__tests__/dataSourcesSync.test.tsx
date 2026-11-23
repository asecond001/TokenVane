import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TimeRangeControl } from '../components/TimeRangeControl';
import { TitleBar } from '../components/TitleBar';
import { formatBucketInTz } from '../lib/format';
import { ipc } from '../lib/ipc';
import { QrCodeSvg, encodeQrMatrix, gfMul, rsComputeRemainder } from '../lib/qrcode';
import { DataSourcesPage } from '../pages/DataSourcesPage';
import { ModelsPricingPage } from '../pages/ModelsPricingPage';
import { MonthlyPage } from '../pages/MonthlyPage';
import { OverviewPage } from '../pages/OverviewPage';
import { PreferencesPage } from '../pages/PreferencesPage';
import { SpeedLatencyPage } from '../pages/SpeedLatencyPage';
import { SubscriptionsPage } from '../pages/SubscriptionsPage';
import { TrayPopupPage } from '../pages/TrayPopupPage';
import { TrendsPage } from '../pages/TrendsPage';
import { computeTimeRange, useWorkspaceStore } from '../stores/workspaceStore';
import type {
  BalanceRefreshOutcome,
  CloudSyncProvisionResult,
  CustomRuleTestResult,
  DeviceRecord,
  ProviderConfig,
  RemoteSubscriptionSnapshot,
  SyncPairingBundle,
  SyncStatusView,
} from '../types/ipc';
import {
  renderWithQuery,
  makePreferences,
  makeBootstrap,
  makeTraySummary,
  makeOverviewReport,
  makeTrendsReport,
  makeMonthlyReport,
  makeSpeedLatencyReport,
  resetWorkspaceState,
  type EChartMockOption,
} from './testUtils';

describe('v0.2.0 Phase 2 Live Summary, Phase 2 Providers/Custom Rules & Cloud Sync', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders 15m active/recent sessions & share bars in OverviewPage and TrayPopupPage click-to-workspace', async () => {
    const now = Date.now();
    vi.spyOn(ipc, 'getOverview').mockResolvedValue(
      makeOverviewReport({
        today_tokens: 96_000,
        today_cost_micro_usd: 1_250_000,
        provider_shares: [
          {
            key: 'kimi_code',
            label: 'Kimi Code',
            tokens: 60_000,
            token_share_percent: 62.5,
            cost_micro_usd: 800_000,
            cost_share_percent: 64.0,
          },
          {
            key: 'claude_code',
            label: 'Claude Code',
            tokens: 36_000,
            token_share_percent: 37.5,
            cost_micro_usd: 450_000,
            cost_share_percent: 36.0,
          },
        ],
        top_model_shares: [
          {
            key: 'kimi-k2',
            label: 'kimi-k2',
            tokens: 60_000,
            token_share_percent: 62.5,
            cost_micro_usd: 800_000,
            cost_share_percent: 64.0,
          },
        ],
        active_sessions: [
          {
            session_key: 'sess-live-1',
            provider_id: 'kimi_code',
            model_id: 'kimi-k2',
            project_label: 'token-tracker',
            activity_state: 'active',
            started_at_ms: now - 180_000,
            last_active_at_ms: now - 15_000,
            request_count: 6,
            total_tokens: 60_000,
            input_tokens: 45_000,
            output_tokens: 15_000,
            cache_read_tokens: 8_000,
            estimated_cost_micro_usd: 800_000,
            latest_output_tps: 92.4,
          },
          {
            session_key: 'sess-recent-2',
            provider_id: 'claude_code',
            model_id: 'claude-sonnet-4-5',
            project_label: 'sync-worker',
            activity_state: 'recent',
            started_at_ms: now - 600_000,
            last_active_at_ms: now - 360_000,
            request_count: 3,
            total_tokens: 36_000,
            input_tokens: 24_000,
            output_tokens: 12_000,
            cache_read_tokens: 0,
            estimated_cost_micro_usd: 450_000,
            latest_output_tps: 64.0,
          },
        ],
      })
    );

    renderWithQuery(<OverviewPage />);

    expect(await screen.findByText('token-tracker')).toBeInTheDocument();
    expect(screen.getByText('1 活跃')).toBeInTheDocument();
    expect(screen.getByText('1 近期')).toBeInTheDocument();
    expect(screen.getByText('活跃中')).toBeInTheDocument();
    expect(screen.getByText('sync-worker')).toBeInTheDocument();

    cleanup();
    const openMainSpy = vi.spyOn(ipc, 'openMainWorkspace').mockResolvedValue(undefined);
    vi.spyOn(ipc, 'getTraySummary').mockResolvedValue(
      makeTraySummary({
        today_tokens: 96_000,
        today_cost_micro_usd: 1_250_000,
        provider_shares: [
          {
            key: 'kimi_code',
            label: 'Kimi Code',
            tokens: 60_000,
            token_share_percent: 62.5,
            cost_micro_usd: 800_000,
            cost_share_percent: 64.0,
          },
        ],
        active_sessions: [
          {
            session_key: 'sess-live-1',
            provider_id: 'kimi_code',
            model_id: 'kimi-k2',
            project_label: 'token-tracker',
            activity_state: 'active',
            started_at_ms: now - 180_000,
            last_active_at_ms: now - 15_000,
            request_count: 6,
            total_tokens: 60_000,
            input_tokens: 45_000,
            output_tokens: 15_000,
            cache_read_tokens: 8_000,
            estimated_cost_micro_usd: 800_000,
            latest_output_tps: 92.4,
          },
        ],
      })
    );

    renderWithQuery(<TrayPopupPage />);
    expect(await screen.findByText('活跃会话（15m）')).toBeInTheDocument();
    expect(await screen.findByText('63%')).toBeInTheDocument();

    const sessionBtn = screen.getByRole('button', { name: /Kimi Code · kimi-k2/ });
    fireEvent.click(sessionBtn);
    await waitFor(() => {
      expect(openMainSpy).toHaveBeenCalledWith('overview', 'kimi_code');
    });
  });

  it('refreshes Phase 2 official balance and dry-runs declarative custom rule lines in DataSourcesPage', async () => {
    const now = Date.now();
    const configs: ProviderConfig[] = [
      {
        provider_id: 'kimi_code',
        enabled: true,
        collection_mode: 'local_log',
        custom_log_paths: [],
        has_credential: true,
        last_sync_at_ms: now - 10_000,
        last_error: null,
        updated_at_ms: now,
      },
    ];

    vi.spyOn(ipc, 'getProviderConfigs').mockResolvedValue(configs);
    vi.spyOn(ipc, 'getIngestionCursors').mockResolvedValue([]);
    vi.spyOn(ipc, 'listCustomProviderRules').mockResolvedValue([]);
    vi.spyOn(ipc, 'getProviderBalances').mockResolvedValue([]);
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(makePreferences());

    const balanceOutcome: BalanceRefreshOutcome = {
      snapshot: {
        provider_id: 'kimi_code',
        currency: 'CNY',
        total_balance_micro: 88_500_000,
        granted_balance_micro: 10_000_000,
        topped_up_balance_micro: 78_500_000,
        daily_quota_used_micro: null,
        daily_quota_limit_micro: null,
        fetched_at_ms: now,
      },
      rate_limits_updated: 0,
      endpoint: 'https://api.moonshot.cn/v1/users/me/balance',
      from_cache: false,
      cooldown_remaining_secs: 60,
    };
    const refreshBalanceSpy = vi
      .spyOn(ipc, 'refreshProviderBalance')
      .mockResolvedValue(balanceOutcome);

    const dryRunResult: CustomRuleTestResult = {
      matched: true,
      provider_id: 'custom_demo',
      model_id: 'gpt-4o',
      timestamp_ms: now,
      input_tokens: 1_200,
      output_tokens: 340,
      cache_read_tokens: 500,
      cache_write_tokens: 0,
      reasoning_tokens: 0,
      estimated_cost_micro_usd: 12_500,
      diagnostic: 'parsed ok',
    };
    const testSampleSpy = vi
      .spyOn(ipc, 'testCustomProviderSampleLine')
      .mockResolvedValue(dryRunResult);

    renderWithQuery(<DataSourcesPage />);

    const refreshBtn = await screen.findByRole('button', { name: '刷新余额' });
    fireEvent.click(refreshBtn);

    await waitFor(() => {
      expect(refreshBalanceSpy).toHaveBeenCalledWith('kimi_code');
    });
    expect(await screen.findByText('CNY 88.50')).toBeInTheDocument();

    const dryRunBtn = screen.getByRole('button', { name: '试跑验证样本' });
    fireEvent.click(dryRunBtn);

    await waitFor(() => {
      expect(testSampleSpy).toHaveBeenCalledTimes(1);
    });
    expect(await screen.findByText('匹配成功')).toBeInTheDocument();
    expect(screen.getByText('模型: gpt-4o')).toBeInTheDocument();
    expect(screen.getByText('输入: 1,200')).toBeInTheDocument();
  });

  it('switches multi-device filter in TitleBar, configures E2EE sync in PreferencesPage & shows read-only remote snapshots in SubscriptionsPage', async () => {
    const now = Date.now();
    const devices: DeviceRecord[] = [
      {
        device_id: 'dev-win-local',
        device_name: 'WorkPC',
        platform: 'windows',
        created_at_ms: now - 100_000,
      },
      {
        device_id: 'dev-mac-remote',
        device_name: 'MacBookPro',
        platform: 'macos',
        created_at_ms: now - 50_000,
      },
    ];
    vi.spyOn(ipc, 'listCustomProviderRules').mockResolvedValue([]);
    vi.spyOn(ipc, 'listDevices').mockResolvedValue(devices);

    render(
      <TitleBar
        collectorWatching={true}
        isScanning={false}
        onScanNow={() => {}}
      />
    );

    const deviceSelect = await screen.findByLabelText('设备筛选');
    expect(within(deviceSelect).getByRole('option', { name: '全部设备' })).toBeInTheDocument();
    expect(
      within(deviceSelect).getByRole('option', { name: '本机 · WorkPC' })
    ).toBeInTheDocument();
    expect(within(deviceSelect).getByRole('option', { name: 'MacBookPro' })).toBeInTheDocument();

    fireEvent.change(deviceSelect, { target: { value: 'dev-mac-remote' } });
    expect(useWorkspaceStore.getState().deviceFilter).toBe('dev-mac-remote');

    // Now test PreferencesPage Cloud Sync & E2EE workflow
    cleanup();
    let currentSyncStatus: SyncStatusView = {
      enabled: false,
      worker_url: '',
      e2ee_enabled: false,
      e2ee_kdf_salt_hex: null,
      has_bearer_token: false,
      has_e2ee_passphrase: false,
      sse_connected: false,
      last_push_watermark_ms: 0,
      last_pull_watermark_ms: 0,
      last_sync_at_ms: now - 30_000,
      last_sync_error: null,
      updated_at_ms: now,
    };
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(makePreferences());
    vi.spyOn(ipc, 'getBootstrapState').mockResolvedValue(makeBootstrap());
    const getSyncStatusSpy = vi
      .spyOn(ipc, 'getSyncStatus')
      .mockImplementation(async () => currentSyncStatus);

    const saveSetupSpy = vi
      .spyOn(ipc, 'saveCloudSyncSetup')
      .mockImplementation(async (enabled, workerUrl, bearerToken, e2eeEnabled, e2eePassphrase) => {
        currentSyncStatus = {
          ...currentSyncStatus,
          enabled,
          worker_url: workerUrl,
          e2ee_enabled: e2eeEnabled,
          e2ee_kdf_salt_hex: e2eeEnabled ? 'aabbccddeeff00112233445566778899' : null,
          has_bearer_token: Boolean(bearerToken) || currentSyncStatus.has_bearer_token,
          has_e2ee_passphrase: Boolean(e2eePassphrase) || currentSyncStatus.has_e2ee_passphrase,
          sse_connected: Boolean(bearerToken || currentSyncStatus.has_bearer_token) && enabled,
        };
        return currentSyncStatus;
      });
    const pairingBundleSpy = vi
      .spyOn(ipc, 'getSyncPairingBundle')
      .mockImplementation(async (includePassphrase) => {
        const uri = includePassphrase
          ? 'tokenvane://pair?v=1&url=https%3A%2F%2Fsync.example.workers.dev&token=tok-secret-123&e2ee=1&key=e2ee-pass-999'
          : 'tokenvane://pair?v=1&url=https%3A%2F%2Fsync.example.workers.dev&token=tok-secret-123&e2ee=1';
        const bundle: SyncPairingBundle = {
          pairing_uri: uri,
          pairing_json: JSON.stringify({
            v: 1,
            url: 'https://sync.example.workers.dev',
            token: 'tok-secret-123',
            e2ee: true,
            ...(includePassphrase ? { key: 'e2ee-pass-999' } : {}),
          }),
          worker_url: 'https://sync.example.workers.dev',
          e2ee_enabled: true,
          includes_passphrase: includePassphrase,
        };
        return bundle;
      });
    const triggerSyncSpy = vi.spyOn(ipc, 'triggerCloudSyncNow').mockResolvedValue({
      pushed_hourly_deltas: 2,
      pushed_subscriptions: 1,
      pulled_items: 1,
      applied_hourly_buckets: 3,
      applied_subscriptions: 1,
      server_seq_ms: now,
      synced_at_ms: now,
    });

    renderWithQuery(<PreferencesPage />);

    expect(await screen.findByText('云端同步')).toBeInTheDocument();
    await waitFor(() => {
      expect(getSyncStatusSpy).toHaveBeenCalled();
    });
    expect(await screen.findByText(/30s 前/)).toBeInTheDocument();

    fireEvent.click(screen.getByLabelText('启用云端同步'));
    fireEvent.click(screen.getByLabelText(/端到端加密/));
    fireEvent.click(screen.getByRole('button', { name: '连接已有 Worker' }));
    fireEvent.change(screen.getByLabelText('Worker 服务地址'), {
      target: { value: 'https://sync.example.workers.dev' },
    });
    const tokenInput = screen.getByLabelText('Sync Bearer Token') as HTMLInputElement;
    const passInput = screen.getByLabelText('E2EE 同步口令') as HTMLInputElement;
    fireEvent.change(tokenInput, { target: { value: 'tok-secret-123' } });
    fireEvent.change(passInput, { target: { value: 'e2ee-pass-999' } });
    fireEvent.click(screen.getByRole('button', { name: '保存并连接' }));

    await waitFor(() => {
      expect(saveSetupSpy).toHaveBeenCalledWith(
        true,
        'https://sync.example.workers.dev',
        'tok-secret-123',
        true,
        'e2ee-pass-999'
      );
    });
    await waitFor(() => {
      expect(pairingBundleSpy).toHaveBeenCalledWith(false);
    });
    expect(tokenInput.value).toBe('');
    expect(passInput.value).toBe('');
    expect(await screen.findByText('SSE 实时已连接')).toBeInTheDocument();
    expect(await screen.findByTestId('sync-pairing-qr')).toBeInTheDocument();
    expect(screen.getByLabelText('移动端配对链接')).toHaveValue(
      'tokenvane://pair?v=1&url=https%3A%2F%2Fsync.example.workers.dev&token=tok-secret-123&e2ee=1'
    );

    fireEvent.click(screen.getByRole('button', { name: '立即同步' }));
    await waitFor(() => {
      expect(triggerSyncSpy).toHaveBeenCalledTimes(1);
    });

    // Finally verify read-only remote subscription snapshots in SubscriptionsPage
    cleanup();
    const remoteSnaps: RemoteSubscriptionSnapshot[] = [
      {
        device_id: 'dev-mac-remote',
        device_display_name: 'MacBookPro',
        plan_id: 'minimax-plan',
        window_id: '5h',
        provider_id: 'minimax_code',
        plan_name: 'MiniMax Coding Plan',
        window_label: '5h 滚动窗口',
        used_percent: 42.5,
        resets_at_ms: now + 3_600_000,
        source: 'local_estimate',
        updated_at_ms: now - 12_000,
      },
    ];
    vi.spyOn(ipc, 'getSubscriptions').mockResolvedValue([]);
    vi.spyOn(ipc, 'getObservedRateLimits').mockResolvedValue([]);
    vi.spyOn(ipc, 'getSubscriptionTemplates').mockResolvedValue([]);
    vi.spyOn(ipc, 'getRemoteSubscriptionSnapshots').mockResolvedValue(remoteSnaps);

    renderWithQuery(<SubscriptionsPage />);
    expect(await screen.findByText('其他设备配额')).toBeInTheDocument();
    expect(screen.getByText('MacBookPro')).toBeInTheDocument();
    expect(screen.getByText('只读远端快照')).toBeInTheDocument();
    expect(screen.getByText('已用 42.5%')).toBeInTheDocument();
  });

  it('one-click provisions Cloudflare Worker via Wrangler or API token, auto-fills fields, and opens mobile pairing QR', async () => {
    const now = Date.now();
    let currentSyncStatus: SyncStatusView = {
      enabled: false,
      worker_url: null,
      e2ee_enabled: false,
      e2ee_kdf_salt_hex: null,
      has_bearer_token: false,
      has_e2ee_passphrase: false,
      has_cf_api_token: false,
      sse_connected: false,
      last_push_watermark_ms: 0,
      last_pull_watermark_ms: 0,
      last_sync_at_ms: null,
      last_sync_error: null,
      updated_at_ms: now,
    };
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(makePreferences());
    vi.spyOn(ipc, 'getBootstrapState').mockResolvedValue(makeBootstrap());
    vi.spyOn(ipc, 'getSyncStatus').mockImplementation(async () => currentSyncStatus);

    const provisionSpy = vi
      .spyOn(ipc, 'provisionCloudSyncWorker')
      .mockImplementation(async (input) => {
        const workerUrl =
          input.mode === 'wrangler_cli'
            ? 'https://tokenvane-sync-worker.demo.workers.dev'
            : 'https://custom-sync-hub.api-edge.workers.dev';
        const bearer = input.mode === 'wrangler_cli' ? 'auto-wrangler-token-48' : 'auto-api-token-48';
        const passphrase = input.e2ee_enabled ? 'auto-e2ee-pass-32' : null;
        currentSyncStatus = {
          ...currentSyncStatus,
          enabled: true,
          worker_url: workerUrl,
          e2ee_enabled: input.e2ee_enabled,
          e2ee_kdf_salt_hex: input.e2ee_enabled ? '00112233445566778899aabbccddeeff' : null,
          has_bearer_token: true,
          has_e2ee_passphrase: Boolean(passphrase),
          has_cf_api_token: input.mode === 'cloudflare_api',
          sse_connected: true,
          last_sync_at_ms: now,
        };
        const params = new URLSearchParams({
          v: '1',
          url: workerUrl,
          token: bearer,
          e2ee: input.e2ee_enabled ? '1' : '0',
        });
        if (passphrase) {
          params.set('key', passphrase);
        }
        const result: CloudSyncProvisionResult = {
          mode: input.mode,
          worker_name: input.worker_name ?? 'tokenvane-sync-worker',
          worker_url: workerUrl,
          bearer_token: bearer,
          e2ee_passphrase: passphrase,
          initial_sync_ok: true,
          status: currentSyncStatus,
          pairing: {
            pairing_uri: `tokenvane://pair?${params.toString()}`,
            pairing_json: JSON.stringify({
              v: 1,
              url: workerUrl,
              token: bearer,
              e2ee: input.e2ee_enabled,
              ...(passphrase ? { key: passphrase } : {}),
            }),
            worker_url: workerUrl,
            e2ee_enabled: input.e2ee_enabled,
            includes_passphrase: Boolean(passphrase),
          },
        };
        return result;
      });

    renderWithQuery(<PreferencesPage />);

    // 1. Default Wrangler CLI mode: click 一键配置 without manual URL or token
    expect(await screen.findByRole('button', { name: 'Wrangler 浏览器授权' })).toHaveAttribute(
      'data-active',
      'true'
    );
    // One-click view keeps the first screen clean: no empty address / token inputs.
    expect(screen.queryByLabelText('Worker 服务地址')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Sync Bearer Token')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '一键配置' }));

    await waitFor(() => {
      expect(provisionSpy).toHaveBeenCalledWith({
        mode: 'wrangler_cli',
        worker_name: 'tokenvane-sync-worker',
        bearer_token: null,
        e2ee_enabled: false,
        e2ee_passphrase: null,
        cf_api_token: null,
        cf_account_id: null,
      });
    });

    expect((screen.getByLabelText('启用云端同步') as HTMLInputElement).checked).toBe(true);
    expect(screen.getByLabelText('Worker 服务地址')).toHaveValue(
      'https://tokenvane-sync-worker.demo.workers.dev'
    );
    expect(screen.getByLabelText('Sync Bearer Token')).toHaveValue('auto-wrangler-token-48');
    expect(await screen.findByTestId('mobile-pairing-panel')).toBeInTheDocument();
    expect(screen.getByTestId('sync-pairing-qr')).toBeInTheDocument();
    expect(screen.getByLabelText('移动端配对链接')).toHaveValue(
      'tokenvane://pair?v=1&url=https%3A%2F%2Ftokenvane-sync-worker.demo.workers.dev&token=auto-wrangler-token-48&e2ee=0'
    );

    // 2. Switch to Cloudflare API 令牌 mode with custom worker name + E2EE
    fireEvent.click(screen.getByRole('button', { name: 'Cloudflare API 令牌' }));
    const cfTokenInput = screen.getByLabelText('Cloudflare API 令牌') as HTMLInputElement;
    fireEvent.change(cfTokenInput, { target: { value: 'cf-pat-secret-xyz' } });
    fireEvent.change(screen.getByLabelText('Worker 名称'), {
      target: { value: 'custom-sync-hub' },
    });
    fireEvent.click(screen.getByLabelText(/端到端加密/));
    fireEvent.click(screen.getByRole('button', { name: '一键配置' }));

    await waitFor(() => {
      expect(provisionSpy).toHaveBeenLastCalledWith({
        mode: 'cloudflare_api',
        worker_name: 'custom-sync-hub',
        bearer_token: null,
        e2ee_enabled: true,
        e2ee_passphrase: null,
        cf_api_token: 'cf-pat-secret-xyz',
        cf_account_id: null,
      });
    });

    expect(cfTokenInput.value).toBe('');
    expect(screen.getByLabelText('Worker 服务地址')).toHaveValue(
      'https://custom-sync-hub.api-edge.workers.dev'
    );
    expect(screen.getByLabelText('Sync Bearer Token')).toHaveValue('auto-api-token-48');
    expect(screen.getByLabelText('E2EE 同步口令')).toHaveValue('auto-e2ee-pass-32');
    expect((screen.getByLabelText('配对码包含口令') as HTMLInputElement).checked).toBe(true);
    expect(screen.getByLabelText('移动端配对链接')).toHaveValue(
      'tokenvane://pair?v=1&url=https%3A%2F%2Fcustom-sync-hub.api-edge.workers.dev&token=auto-api-token-48&e2ee=1&key=auto-e2ee-pass-32'
    );
  });
});

describe('v0.3.0 Analytics Differentiation, Sync Pairing QR, TZ & UX Fixes', () => {
  beforeEach(() => {
    resetWorkspaceState();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('encodes QR matrices with GF(256) RS parity, finder patterns, and SVG path for pairing URIs', () => {
    expect(gfMul(0x57, 0x83)).toBe(0x31);
    const rs = rsComputeRemainder(new Uint8Array([0x10, 0x20, 0x0c, 0x56]), 7);
    expect(rs).toHaveLength(7);

    const uri =
      'tokenvane://pair?v=1&url=https%3A%2F%2Fsync.example.workers.dev&token=tok-123&e2ee=1';
    const matrix = encodeQrMatrix(uri);
    expect(matrix.version).toBeGreaterThanOrEqual(4);
    expect(matrix.size).toBe(17 + 4 * matrix.version);
    // Top-left 7x7 finder pattern outer border & center 3x3
    for (let i = 0; i < 7; i += 1) {
      expect(matrix.modules[0][i]).toBe(true);
      expect(matrix.modules[6][i]).toBe(true);
      expect(matrix.modules[i][0]).toBe(true);
      expect(matrix.modules[i][6]).toBe(true);
    }
    expect(matrix.modules[3][3]).toBe(true);
    expect(matrix.modules[1][1]).toBe(false);

    render(<QrCodeSvg value={uri} size={140} />);
    const svg = screen.getByTestId('sync-pairing-qr');
    expect(svg.tagName.toLowerCase()).toBe('svg');
    expect(svg.getAttribute('data-qr-version')).toBe(String(matrix.version));
    const pathEl = svg.querySelector('path');
    expect(pathEl?.getAttribute('d')).toMatch(/^M0 0/);
  });

  it('aligns computeTimeRange 7d to exact 7 local calendar days and formats buckets in explicit tzOffsetMinutes', () => {
    const fixedNow = new Date(2026, 10, 17, 15, 22, 10, 500); // Nov 17 15:22:10 local
    const range7d = computeTimeRange('7d', undefined, fixedNow);
    const expectedStart = new Date(2026, 10, 11, 0, 0, 0, 0).getTime();
    const expectedEnd = new Date(2026, 10, 17, 23, 59, 59, 999).getTime();
    expect(range7d.start_ms).toBe(expectedStart);
    expect(range7d.end_ms).toBe(expectedEnd);
    expect(Math.round((range7d.end_ms - range7d.start_ms + 1) / 86_400_000)).toBe(7);

    // Custom reversed dates swap cleanly to chronological local bounds
    const reversedCustom = computeTimeRange(
      'custom',
      { start: '2026-11-17', end: '2026-11-12' },
      fixedNow
    );
    expect(reversedCustom.start_ms).toBe(new Date(2026, 10, 12, 0, 0, 0, 0).getTime());
    expect(reversedCustom.end_ms).toBe(new Date(2026, 10, 17, 23, 59, 59, 999).getTime());

    // Bucket UTC+8 vs UTC formatting
    const utcMidnight = Date.UTC(2026, 10, 16, 16, 0, 0); // 2026-11-17 00:00 in UTC+8 (+480)
    expect(formatBucketInTz(utcMidnight, 'daily', 480)).toBe('2026-11-17');
    expect(formatBucketInTz(utcMidnight, 'hour_only', 480)).toBe('00:00');
    expect(formatBucketInTz(utcMidnight, 'daily', 0)).toBe('2026-11-16');
  });

  it('defers custom date changes in TimeRangeControl until 确定 is clicked and keeps action buttons non-wrapping', () => {
    render(<TimeRangeControl />);

    expect(useWorkspaceStore.getState().timePreset).toBe('7d');
    const initialStart = useWorkspaceStore.getState().customStartDate;
    const initialEnd = useWorkspaceStore.getState().customEndDate;

    fireEvent.click(screen.getByRole('button', { name: '自定义' }));
    expect(screen.getByRole('dialog', { name: '自定义日期范围' })).toBeInTheDocument();

    const startInput = screen.getByLabelText('开始日期');
    const endInput = screen.getByLabelText('结束日期');
    fireEvent.change(startInput, { target: { value: '2026-11-01' } });
    fireEvent.change(endInput, { target: { value: '2026-11-05' } });

    // Store must remain unchanged while editing drafts
    expect(useWorkspaceStore.getState().timePreset).toBe('7d');
    expect(useWorkspaceStore.getState().customStartDate).toBe(initialStart);
    expect(useWorkspaceStore.getState().customEndDate).toBe(initialEnd);

    // 取消 discards drafts
    fireEvent.click(screen.getByRole('button', { name: '取消' }));
    expect(screen.queryByRole('dialog', { name: '自定义日期范围' })).not.toBeInTheDocument();
    expect(useWorkspaceStore.getState().timePreset).toBe('7d');

    // Reopen, edit drafts, confirm commits custom dates cleanly with whitespace-nowrap button
    fireEvent.click(screen.getByRole('button', { name: '自定义' }));
    fireEvent.change(screen.getByLabelText('开始日期'), { target: { value: '2026-11-01' } });
    fireEvent.change(screen.getByLabelText('结束日期'), { target: { value: '2026-11-05' } });
    const confirmBtn = screen.getByRole('button', { name: '确定' });
    expect(confirmBtn.className).toContain('whitespace-nowrap');
    fireEvent.click(confirmBtn);

    expect(useWorkspaceStore.getState().timePreset).toBe('custom');
    expect(useWorkspaceStore.getState().customStartDate).toBe('2026-11-01');
    expect(useWorkspaceStore.getState().customEndDate).toBe('2026-11-05');
  });

  it('collapses floating window element toggles and opacity slider when floating_window_enabled is false', async () => {
    const prefs = makePreferences({ floating_window_enabled: false });
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(prefs);
    vi.spyOn(ipc, 'getBootstrapState').mockResolvedValue(makeBootstrap(prefs));
    vi.spyOn(ipc, 'updatePreferences').mockImplementation(async (next) => next);

    renderWithQuery(<PreferencesPage />);

    const toggle = (await screen.findByLabelText('显示桌面实时悬浮窗')) as HTMLInputElement;
    expect(toggle.checked).toBe(false);
    expect(screen.queryByRole('button', { name: 'Token 用量' })).not.toBeInTheDocument();
    expect(screen.queryByLabelText('悬浮窗不透明度')).not.toBeInTheDocument();

    fireEvent.click(toggle);
    expect(await screen.findByRole('button', { name: 'Token 用量' })).toBeInTheDocument();
    expect(screen.getByLabelText('悬浮窗不透明度')).toBeInTheDocument();
  });

  it('defaults background_cli_quota_poll to on in PreferencesPage and persists opt-out toggle', async () => {
    const prefs = makePreferences({ background_cli_quota_poll: true });
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(prefs);
    vi.spyOn(ipc, 'getBootstrapState').mockResolvedValue(makeBootstrap(prefs));
    const updateSpy = vi
      .spyOn(ipc, 'updatePreferences')
      .mockImplementation(async (next) => next);

    renderWithQuery(<PreferencesPage />);

    const toggle = (await screen.findByLabelText(
      '后台自动调用官方 CLI 刷新配额（Antigravity / Claude Code）'
    )) as HTMLInputElement;
    expect(toggle.checked).toBe(true);

    fireEvent.click(toggle);
    await waitFor(() => {
      expect(updateSpy).toHaveBeenCalledWith(
        expect.objectContaining({ background_cli_quota_poll: false })
      );
    });
  });

  it('supports specialized analytics modes in TrendsPage, MonthlyPage, ModelsPricingPage & SpeedLatencyPage', async () => {
    vi.spyOn(ipc, 'getTrends').mockResolvedValue(makeTrendsReport());
    renderWithQuery(<TrendsPage />);

    expect(await screen.findByTestId('trends-date-span')).toBeInTheDocument();
    expect(screen.getByText('区间总 Token')).toBeInTheDocument();

    // Switch metric to 缓存命中率 -> keeps 柱状 / 折线 switcher visible, grouped (unstacked) bars, and switches cleanly back to line
    fireEvent.click(screen.getByRole('button', { name: '缓存命中率' }));
    expect(screen.getByRole('button', { name: '按 Token 类型' })).toBeDisabled();
    expect(screen.getByRole('button', { name: '折线' })).toHaveAttribute('data-active', 'true');
    fireEvent.click(screen.getByRole('button', { name: '柱状' }));

    const cacheBarOpt = JSON.parse(
      screen.getByTestId('echart-stub').getAttribute('data-option-json') || '{}'
    ) as EChartMockOption & { series?: Array<{ stack?: string }> };
    expect(cacheBarOpt.series?.[0]?.type).toBe('bar');
    expect(cacheBarOpt.series?.[0]?.stack).toBeUndefined();
    expect(cacheBarOpt.series?.[0]?.data).toEqual([58, 68]);

    fireEvent.click(screen.getByRole('button', { name: '折线' }));
    const cacheOpt = JSON.parse(
      screen.getByTestId('echart-stub').getAttribute('data-option-json') || '{}'
    ) as EChartMockOption;
    expect(cacheOpt.series?.[0]?.type).toBe('line');
    expect(cacheOpt.series?.[0]?.data).toEqual([58, 68]);

    cleanup();
    vi.spyOn(ipc, 'getMonthly').mockResolvedValue(makeMonthlyReport());
    renderWithQuery(<MonthlyPage />);
    expect(await screen.findByRole('columnheader', { name: '缓存节省 (USD)' })).toBeInTheDocument();
    expect(screen.getByText('账单构成')).toBeInTheDocument();
    expect(screen.getByText('提供商账单占比')).toBeInTheDocument();
    expect(screen.getByText('主力模型账单 Top 5')).toBeInTheDocument();

    cleanup();
    vi.spyOn(ipc, 'getPreferences').mockResolvedValue(makePreferences());
    vi.spyOn(ipc, 'getModelsPricing').mockResolvedValue({
      rows: [
        {
          model_id: 'claude-opus-4-6',
          provider_id: 'claude_code',
          requests: 20,
          observable_steps: 45,
          input_tokens: 100_000,
          output_tokens: 30_000,
          cache_read_tokens: 300_000,
          cache_write_tokens: 20_000,
          reasoning_tokens: 0,
          total_tokens: 450_000,
          cache_saved_micro_usd: 3_500_000,
          estimated_cost_micro_usd: 6_000_000,
          cost_share_ratio: 1,
          avg_output_tps: 76.5,
          effective_pricing: null,
          list_cost_micro_usd: 9_000_000,
          subscription_cost_micro_usd: null,
        },
      ],
      total_cost_micro_usd: 6_000_000,
      total_cache_saved_micro_usd: 3_500_000,
      subscription_allocations: [],
      catalog: [
        {
          model_pattern: 'claude-opus-4-6',
          provider_id: 'claude_code',
          input_per_m_micro_usd: 15_000_000,
          output_per_m_micro_usd: 75_000_000,
          cache_read_per_m_micro_usd: 1_500_000,
          cache_write_per_m_micro_usd: 18_750_000,
          reasoning_per_m_micro_usd: 75_000_000,
          is_user_override: false,
          effective_from: '2026-11-01',
        },
      ],
    });
    renderWithQuery(<ModelsPricingPage />);
    expect(await screen.findByTestId('models-efficiency-chart')).toBeInTheDocument();
    expect(screen.getByText('76.5 tok/s')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '单价目录' }));
    expect(await screen.findByLabelText('搜索单价目录')).toBeInTheDocument();
    expect(screen.getByText('共 1 条单价规则')).toBeInTheDocument();

    cleanup();
    vi.spyOn(ipc, 'getSpeedLatency').mockResolvedValue(makeSpeedLatencyReport());
    renderWithQuery(<SpeedLatencyPage />);
    expect(await screen.findByText('全局加权均值')).toBeInTheDocument();
    expect(screen.getByText('最快模型 P95')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '响应耗时与首字延迟 (ms)' }));
    const latOpt = JSON.parse(
      screen.getByTestId('echart-stub').getAttribute('data-option-json') || '{}'
    ) as EChartMockOption;
    expect(latOpt.series?.map((s) => s.name)).toEqual(['平均首字延迟', '平均总耗时']);
  });
});
