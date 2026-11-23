import { useCallback, useEffect, useRef, useState } from 'react';
import { ActionNotice } from '../components/ActionNotice';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { disable as disableAutostart, enable as enableAutostart } from '@tauri-apps/plugin-autostart';
import { open } from '@tauri-apps/plugin-dialog';
import {
  Check,
  Cloud,
  Copy,
  Download,
  Image as ImageIcon,
  KeyRound,
  QrCode,
  RefreshCw,
  RotateCcw,
  Save,
  ShieldCheck,
  Sparkles,
  Wand2,
} from 'lucide-react';
import { InlineProgressBar } from '../components/InlineProgressBar';
import { ProvisionProgress } from '../components/ProvisionProgress';
import {
  BACKGROUND_GRADIENT_PRESETS,
  type BackgroundPresetId,
} from '../components/WorkspaceBackdrop';
import {
  FLOATING_MODULES,
  computeFloatingDimensions,
  toggleFloatingModule,
  useFloatingConfig,
  type FloatingModuleId,
} from '../lib/floatingConfig';
import { formatRelativeTime } from '../lib/format';
import { APP_VERSION } from '../lib/appVersion';
import { TRAY_SUMMARY_QUERY_KEY } from '../lib/queryKeys';
import { ipc, isTauriRuntime, onCloudSyncProvisionProgress } from '../lib/ipc';
import { QrCodeSvg } from '../lib/qrcode';
import {
  checkForUpdates,
  downloadAndInstallUpdate,
  useUpdaterStore,
} from '../lib/updater';
import { syncDocumentAndNativeTheme, useWorkspaceStore } from '../stores/workspaceStore';
import type {
  AppPreferences,
  BootstrapState,
  CloudSyncDeployMode,
  CloudSyncProvisionResult,
  CloudSyncProvisionStep,
  CurrencyCode,
  SyncEnvelopeV1,
  SyncPairingBundle,
  SyncStatusView,
  ThemeMode,
} from '../types/ipc';

export interface PreferencesPageProps {
  onApplyTheme?: (nextTheme: ThemeMode) => void;
  onPreviewPreferences?: (patch: Partial<AppPreferences>) => AppPreferences | null;
  onPersistPreferences?: (patch: Partial<AppPreferences>) => Promise<AppPreferences>;
}

const SYNC_FIELD_STYLE = {
  backgroundColor: 'var(--bg-input)',
  border: '1px solid var(--border-subtle)',
  color: 'var(--text-primary)',
} as const;

interface ProvisionRunState {
  mode: CloudSyncDeployMode;
  steps: CloudSyncProvisionStep[];
  status: 'running' | 'failed';
  startedAtMs: number;
}

interface NoticeState {
  text: string;
  kind: 'success' | 'error';
}

interface SyncedFormSnapshot {
  currency: CurrencyCode;
  usd_to_cny_rate: string;
  minimize_to_tray: boolean;
  launch_at_startup: boolean;
  floating_window_enabled: boolean;
  background_cli_quota_poll: boolean;
  device_display_name: string;
  auto_sync_pricing: boolean;
  exchange_rate_auto: boolean;
  auto_check_updates: boolean;
}

type BackgroundMode = 'none' | 'gradient' | 'image';

const THEME_OPTIONS: ReadonlyArray<{ id: ThemeMode; label: string }> = [
  { id: 'dark', label: '深色 (Dark)' },
  { id: 'light', label: '浅色 (Light)' },
  { id: 'system', label: '跟随系统' },
];

const BG_MODE_OPTIONS: ReadonlyArray<{ id: BackgroundMode; label: string }> = [
  { id: 'none', label: '纯色默认' },
  { id: 'gradient', label: '微光渐变' },
  { id: 'image', label: '本地壁纸' },
];


function normalizeThemeMode(raw: string | undefined | null): ThemeMode {
  return raw === 'light' || raw === 'dark' || raw === 'system' ? raw : 'dark';
}

function normalizeBgMode(raw: string | undefined | null): BackgroundMode {
  return raw === 'gradient' || raw === 'image' ? raw : 'none';
}

function clampOpacity(raw: number | undefined): number {
  if (raw === undefined || !Number.isFinite(raw)) return 0.18;
  return Math.min(0.6, Math.max(0.05, raw));
}

function clampBlurPx(raw: number | undefined): number {
  if (raw === undefined || !Number.isFinite(raw)) return 16;
  return Math.round(Math.min(32, Math.max(0, raw)));
}

function generateSecretToken(byteLength = 24): string {
  const bytes = new Uint8Array(byteLength);
  if (typeof globalThis.crypto?.getRandomValues === 'function') {
    globalThis.crypto.getRandomValues(bytes);
  } else {
    for (let i = 0; i < byteLength; i += 1) {
      bytes[i] = Math.floor(Math.random() * 256);
    }
  }
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}

export function PreferencesPage({
  onApplyTheme,
  onPreviewPreferences,
  onPersistPreferences,
}: PreferencesPageProps) {
  const queryClient = useQueryClient();
  const {
    theme,
    resolvedTheme,
    setCurrency,
    setUsdToCnyRate,
    setTheme,
  } = useWorkspaceStore();
  const [floatingConfig, updateFloatingConfig] = useFloatingConfig();

  const { data: bootstrap } = useQuery({
    queryKey: ['bootstrap'],
    queryFn: () => ipc.getBootstrapState(),
  });

  const { data: prefs } = useQuery({
    queryKey: ['preferences'],
    queryFn: () => ipc.getPreferences(),
  });

  const { data: syncStatus } = useQuery({
    queryKey: ['sync-status'],
    queryFn: () => ipc.getSyncStatus(),
    refetchInterval: 15_000,
  });

  const [currencyDraft, setCurrencyDraft] = useState<CurrencyCode>('USD');
  const [rateDraft, setRateDraft] = useState('7.25');
  const [minimizeToTray, setMinimizeToTray] = useState(true);
  const [launchAtStartup, setLaunchAtStartup] = useState(false);
  const [floatingWindowEnabled, setFloatingWindowEnabled] = useState(false);
  const [backgroundCliQuotaPoll, setBackgroundCliQuotaPoll] = useState(true);
  const [deviceDisplayName, setDeviceDisplayName] = useState('');
  const [autoSyncPricing, setAutoSyncPricing] = useState(true);
  const [exchangeRateAuto, setExchangeRateAuto] = useState(true);
  const [autoCheckUpdates, setAutoCheckUpdates] = useState(true);
  const [syncEnabledDraft, setSyncEnabledDraft] = useState(false);
  const [syncWorkerUrlDraft, setSyncWorkerUrlDraft] = useState('');
  const [syncE2eeDraft, setSyncE2eeDraft] = useState(false);
  const [syncBearerTokenInput, setSyncBearerTokenInput] = useState('');
  const [syncPassphraseInput, setSyncPassphraseInput] = useState('');
  const [deployMode, setDeployMode] = useState<CloudSyncDeployMode>('wrangler_cli');
  const [connectMode, setConnectMode] = useState<'oneclick' | 'manual'>('oneclick');
  const [provisionRun, setProvisionRun] = useState<ProvisionRunState | null>(null);
  const [workerNameDraft, setWorkerNameDraft] = useState('tokenvane-sync-worker');
  const [cfApiTokenInput, setCfApiTokenInput] = useState('');
  const [cfAccountIdInput, setCfAccountIdInput] = useState('');
  const [pairingBundle, setPairingBundle] = useState<SyncPairingBundle | null>(null);
  const [pairingExpanded, setPairingExpanded] = useState(false);
  const [includePassphraseInPairing, setIncludePassphraseInPairing] = useState(false);
  const [pairingCopied, setPairingCopied] = useState(false);
  const [notice, setNotice] = useState<NoticeState | null>(null);
  const clearNotice = useCallback(() => setNotice(null), []);
  const [syncEnvelope, setSyncEnvelope] = useState<SyncEnvelopeV1 | null>(null);
  const updater = useUpdaterStore();

  const lastSyncedSnapshotRef = useRef<SyncedFormSnapshot | null>(null);
  const lastSyncCfgSnapshotRef = useRef<{
    enabled: boolean;
    worker_url: string;
    e2ee_enabled: boolean;
  } | null>(null);
  const sliderDraftRef = useRef<{ bg_opacity?: number; bg_blur_px?: number }>({});
  const sliderTimerRef = useRef<number | null>(null);

  const activePrefs = prefs ?? bootstrap?.preferences ?? null;
  const bgMode = normalizeBgMode(activePrefs?.bg_mode);
  const bgPresetId = activePrefs?.bg_preset_id ?? 'aurora_slate';
  const bgImagePath = activePrefs?.bg_image_path ?? null;
  const bgOpacity = clampOpacity(activePrefs?.bg_opacity);
  const bgBlurPx = clampBlurPx(activePrefs?.bg_blur_px);

  useEffect(() => {
    const src = prefs ?? bootstrap?.preferences;
    if (!src) return;

    const prev = lastSyncedSnapshotRef.current;
    const nextCurrency: CurrencyCode = src.currency ?? 'USD';
    const nextRate = src.usd_to_cny_rate ?? '7.25';
    const nextMinimize = src.minimize_to_tray ?? true;
    const nextAutostart = src.launch_at_startup ?? false;
    const nextFloating = src.floating_window_enabled ?? false;
    const nextCliPoll = src.background_cli_quota_poll ?? true;
    const nextDeviceName = src.device_display_name ?? '';
    const nextAutoSync = src.auto_sync_pricing ?? true;
    const nextRateAuto = src.exchange_rate_auto ?? true;
    const nextAutoUpdates = src.auto_check_updates ?? true;

    if (!prev || prev.currency !== nextCurrency) {
      setCurrencyDraft(nextCurrency);
    }
    if (!prev || prev.usd_to_cny_rate !== nextRate) {
      setRateDraft(nextRate);
    }
    if (!prev || prev.minimize_to_tray !== nextMinimize) {
      setMinimizeToTray(nextMinimize);
    }
    if (!prev || prev.launch_at_startup !== nextAutostart) {
      setLaunchAtStartup(nextAutostart);
    }
    if (!prev || prev.floating_window_enabled !== nextFloating) {
      setFloatingWindowEnabled(nextFloating);
    }
    if (!prev || prev.background_cli_quota_poll !== nextCliPoll) {
      setBackgroundCliQuotaPoll(nextCliPoll);
    }
    if (!prev || prev.device_display_name !== nextDeviceName) {
      setDeviceDisplayName(nextDeviceName);
    }
    if (!prev || prev.auto_sync_pricing !== nextAutoSync) {
      setAutoSyncPricing(nextAutoSync);
    }
    if (!prev || prev.exchange_rate_auto !== nextRateAuto) {
      setExchangeRateAuto(nextRateAuto);
    }
    if (!prev || prev.auto_check_updates !== nextAutoUpdates) {
      setAutoCheckUpdates(nextAutoUpdates);
    }

    lastSyncedSnapshotRef.current = {
      currency: nextCurrency,
      usd_to_cny_rate: nextRate,
      minimize_to_tray: nextMinimize,
      launch_at_startup: nextAutostart,
      floating_window_enabled: nextFloating,
      background_cli_quota_poll: nextCliPoll,
      device_display_name: nextDeviceName,
      auto_sync_pricing: nextAutoSync,
      exchange_rate_auto: nextRateAuto,
      auto_check_updates: nextAutoUpdates,
    };
  }, [prefs, bootstrap]);

  useEffect(() => {
    if (!syncStatus) return;
    const nextUrl = syncStatus.worker_url ?? '';
    const prev = lastSyncCfgSnapshotRef.current;
    if (!prev || prev.enabled !== syncStatus.enabled) {
      setSyncEnabledDraft(syncStatus.enabled);
    }
    if (!prev || prev.worker_url !== nextUrl) {
      setSyncWorkerUrlDraft(nextUrl);
    }
    if (!prev || prev.e2ee_enabled !== syncStatus.e2ee_enabled) {
      setSyncE2eeDraft(syncStatus.e2ee_enabled);
    }
    lastSyncCfgSnapshotRef.current = {
      enabled: syncStatus.enabled,
      worker_url: nextUrl,
      e2ee_enabled: syncStatus.e2ee_enabled,
    };
  }, [syncStatus]);

  const previewPreferences = useCallback(
    (patch: Partial<AppPreferences>): AppPreferences | null => {
      if (onPreviewPreferences) {
        return onPreviewPreferences(patch);
      }

      if (patch.theme !== undefined) {
        const normalized = normalizeThemeMode(patch.theme);
        setTheme(normalized);
        syncDocumentAndNativeTheme(normalized);
      }
      if (patch.currency !== undefined) {
        setCurrency(patch.currency);
      }
      if (patch.usd_to_cny_rate !== undefined) {
        setUsdToCnyRate(patch.usd_to_cny_rate);
      }

      const base =
        queryClient.getQueryData<AppPreferences>(['preferences']) ??
        queryClient.getQueryData<BootstrapState>(['bootstrap'])?.preferences;
      if (!base) {
        return null;
      }

      const next: AppPreferences = {
        ...base,
        ...patch,
      };
      queryClient.setQueryData<AppPreferences>(['preferences'], next);
      queryClient.setQueryData<BootstrapState>(['bootstrap'], (prev) =>
        prev ? { ...prev, preferences: next } : prev
      );
      return next;
    },
    [onPreviewPreferences, queryClient, setCurrency, setTheme, setUsdToCnyRate]
  );

  const persistPreferences = useCallback(
    async (patch: Partial<AppPreferences>): Promise<AppPreferences> => {
      if (onPersistPreferences) {
        return onPersistPreferences(patch);
      }

      let next = previewPreferences(patch);
      if (!next) {
        const fetched = await ipc.getPreferences();
        next = { ...fetched, ...patch };
      }
      const saved = await ipc.updatePreferences(next);
      queryClient.setQueryData<AppPreferences>(['preferences'], saved);
      queryClient.setQueryData<BootstrapState>(['bootstrap'], (prev) =>
        prev ? { ...prev, preferences: saved } : prev
      );
      return saved;
    },
    [onPersistPreferences, previewPreferences, queryClient]
  );

  const persistPreferencesRef = useRef(persistPreferences);
  persistPreferencesRef.current = persistPreferences;

  const flushSliderPersist = useCallback(() => {
    if (sliderTimerRef.current !== null) {
      window.clearTimeout(sliderTimerRef.current);
      sliderTimerRef.current = null;
    }
    const pending = sliderDraftRef.current;
    if (pending.bg_opacity === undefined && pending.bg_blur_px === undefined) {
      return;
    }
    sliderDraftRef.current = {};
    void persistPreferencesRef.current(pending).catch((err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '更新背景参数失败',
        kind: 'error',
      });
    });
  }, []);

  useEffect(() => {
    return () => {
      flushSliderPersist();
    };
  }, [flushSliderPersist]);

  const handleThemeSelect = (nextTheme: ThemeMode) => {
    if (onApplyTheme) {
      onApplyTheme(nextTheme);
      return;
    }
    void persistPreferences({ theme: nextTheme });
  };

  const handleBgModeSelect = (nextMode: BackgroundMode) => {
    void persistPreferences({ bg_mode: nextMode }).catch((err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '切换背景模式失败',
        kind: 'error',
      });
    });
  };

  const handlePresetSelect = (presetId: BackgroundPresetId) => {
    void persistPreferences({
      bg_mode: 'gradient',
      bg_preset_id: presetId,
    }).catch((err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '应用渐变预设失败',
        kind: 'error',
      });
    });
  };

  const handleSliderPreview = (patch: { bg_opacity?: number; bg_blur_px?: number }) => {
    sliderDraftRef.current = {
      ...sliderDraftRef.current,
      ...patch,
    };
    previewPreferences(patch);

    if (sliderTimerRef.current !== null) {
      window.clearTimeout(sliderTimerRef.current);
    }
    sliderTimerRef.current = window.setTimeout(() => {
      flushSliderPersist();
    }, 180);
  };

  const importBgMutation = useMutation({
    mutationFn: (sourcePath: string) => ipc.importBackgroundImage(sourcePath),
    onSuccess: async (filename) => {
      previewPreferences({
        bg_mode: 'image',
        bg_image_path: filename,
      });
      await queryClient.invalidateQueries({ queryKey: ['bg-image-data-url'] });
      if (isTauriRuntime()) {
        await queryClient.invalidateQueries({ queryKey: ['preferences'] });
        await queryClient.invalidateQueries({ queryKey: ['bootstrap'] });
      }
      setNotice({ text: `已导入本地壁纸：${filename}`, kind: 'success' });
    },
    onError: (err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '导入壁纸失败',
        kind: 'error',
      });
    },
  });

  const clearBgMutation = useMutation({
    mutationFn: () => ipc.clearBackgroundImage(),
    onSuccess: async () => {
      previewPreferences({
        bg_image_path: null,
        bg_mode: 'none',
      });
      queryClient.removeQueries({ queryKey: ['bg-image-data-url'] });
      if (isTauriRuntime()) {
        await queryClient.invalidateQueries({ queryKey: ['preferences'] });
        await queryClient.invalidateQueries({ queryKey: ['bootstrap'] });
      }
      setNotice({ text: '已清除自定义壁纸并恢复纯色默认', kind: 'success' });
    },
    onError: (err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '清除壁纸失败',
        kind: 'error',
      });
    },
  });

  const handlePickBackgroundImage = async () => {
    if (!isTauriRuntime()) {
      setNotice({
        text: '本地壁纸文件选择器仅在桌面端原生运行时可用',
        kind: 'error',
      });
      return;
    }
    try {
      const selected = await open({
        directory: false,
        multiple: false,
        title: '选择工作台背景壁纸',
        filters: [
          {
            name: '图片文件',
            extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp'],
          },
        ],
      });
      if (typeof selected === 'string' && selected.trim()) {
        await importBgMutation.mutateAsync(selected);
      }
    } catch (err: unknown) {
      setNotice({
        text: err instanceof Error ? err.message : '打开文件选择器失败',
        kind: 'error',
      });
    }
  };

  const savePrefsMutation = useMutation({
    mutationFn: async () => {
      if (sliderTimerRef.current !== null) {
        window.clearTimeout(sliderTimerRef.current);
        sliderTimerRef.current = null;
      }
      const pendingSliders = sliderDraftRef.current;
      sliderDraftRef.current = {};

      const base =
        queryClient.getQueryData<AppPreferences>(['preferences']) ??
        prefs ??
        bootstrap?.preferences;
      if (!base) {
        throw new Error('请稍候重试保存偏好设置');
      }

      const rateNum = Number.parseFloat(rateDraft);
      if (!Number.isFinite(rateNum) || rateNum <= 0) {
        throw new Error('USD 对 CNY 汇率必须为正数');
      }

      const currentTheme = useWorkspaceStore.getState().theme;
      const patch: Partial<AppPreferences> = {
        currency: currencyDraft,
        usd_to_cny_rate: rateDraft.trim(),
        theme: currentTheme,
        minimize_to_tray: minimizeToTray,
        launch_at_startup: launchAtStartup,
        floating_window_enabled: floatingWindowEnabled,
        background_cli_quota_poll: backgroundCliQuotaPoll,
        device_display_name: deviceDisplayName.trim() || base.device_display_name,
        auto_sync_pricing: autoSyncPricing,
        exchange_rate_auto: exchangeRateAuto,
        auto_check_updates: autoCheckUpdates,
        ...pendingSliders,
      };

      const saved = await persistPreferences(patch);

      if (isTauriRuntime()) {
        try {
          if (saved.launch_at_startup) {
            await enableAutostart();
          } else {
            await disableAutostart();
          }
        } catch {
          // Non-fatal if OS registry access is restricted
        }
      }

      return saved;
    },
    onSuccess: async (saved) => {
      setCurrency(saved.currency);
      setUsdToCnyRate(saved.usd_to_cny_rate);
      const nextTheme = normalizeThemeMode(saved.theme);
      setTheme(nextTheme);
      syncDocumentAndNativeTheme(nextTheme);

      lastSyncedSnapshotRef.current = {
        currency: saved.currency,
        usd_to_cny_rate: saved.usd_to_cny_rate,
        minimize_to_tray: saved.minimize_to_tray,
        launch_at_startup: saved.launch_at_startup,
        floating_window_enabled: saved.floating_window_enabled,
        background_cli_quota_poll: saved.background_cli_quota_poll,
        device_display_name: saved.device_display_name,
        auto_sync_pricing: saved.auto_sync_pricing,
        exchange_rate_auto: saved.exchange_rate_auto,
        auto_check_updates: saved.auto_check_updates,
      };

      setNotice({ text: '偏好设置已持久化至本地 SQLite', kind: 'success' });
      await queryClient.invalidateQueries({ queryKey: [TRAY_SUMMARY_QUERY_KEY] });
    },
    onError: (err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '保存偏好失败',
        kind: 'error',
      });
    },
  });

  const syncNowMutation = useMutation({
    mutationFn: () => ipc.syncPricingAndRate(true),
    onSuccess: async (res) => {
      if (res.rate_updated) {
        setRateDraft(res.usd_to_cny_rate);
        setUsdToCnyRate(res.usd_to_cny_rate);
      }
      await queryClient.invalidateQueries({
        predicate: (query) => query.queryKey[0] !== 'bg-image-data-url',
      });
      if (
        res.warning &&
        !res.rate_updated &&
        !res.pricing_updated &&
        !res.subscriptions_updated
      ) {
        setNotice({
          text: `在线刷新受限（已保持本地基线）：${res.warning}`,
          kind: 'error',
        });
      } else {
        setNotice({
          text: `已刷新汇率、公开模型单价与套餐目录（1 USD = ${res.usd_to_cny_rate} CNY · ${res.synced_models_count} 款模型 · ${res.synced_plans_count} 款订阅）`,
          kind: 'success',
        });
      }
    },
    onError: (err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '刷新汇率与模型单价失败',
        kind: 'error',
      });
    },
  });

  const handleExchangeRateAutoToggle = (auto: boolean) => {
    setExchangeRateAuto(auto);
    void persistPreferences({ exchange_rate_auto: auto }).catch((err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '切换汇率模式失败',
        kind: 'error',
      });
    });
  };

  const handleAutoSyncPricingToggle = (enabled: boolean) => {
    setAutoSyncPricing(enabled);
    void persistPreferences({ auto_sync_pricing: enabled }).catch((err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '切换自动更新设置失败',
        kind: 'error',
      });
    });
  };

  const handleAutoCheckUpdatesToggle = (enabled: boolean) => {
    setAutoCheckUpdates(enabled);
    void persistPreferences({ auto_check_updates: enabled }).catch((err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '切换自动检查更新设置失败',
        kind: 'error',
      });
    });
  };

  const handleMinimizeToTrayToggle = (enabled: boolean) => {
    setMinimizeToTray(enabled);
    void persistPreferences({ minimize_to_tray: enabled }).catch((err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '切换最小化至托盘设置失败',
        kind: 'error',
      });
    });
  };

  const handleLaunchAtStartupToggle = (enabled: boolean) => {
    setLaunchAtStartup(enabled);
    void persistPreferences({ launch_at_startup: enabled })
      .then(async (saved) => {
        if (isTauriRuntime()) {
          try {
            if (saved.launch_at_startup) {
              await enableAutostart();
            } else {
              await disableAutostart();
            }
          } catch {
            // Handled by backend update_preferences
          }
        }
      })
      .catch((err: unknown) => {
        setNotice({
          text: err instanceof Error ? err.message : '切换开机自启设置失败',
          kind: 'error',
        });
      });
  };

  const handleFloatingWindowToggle = (enabled: boolean) => {
    setFloatingWindowEnabled(enabled);
    void persistPreferences({ floating_window_enabled: enabled })
      .then(() => queryClient.invalidateQueries({ queryKey: ['floating-visible'] }))
      .catch((err: unknown) => {
        setNotice({
          text: err instanceof Error ? err.message : '切换桌面悬浮窗状态失败',
          kind: 'error',
        });
      });
  };

  const handleBackgroundCliQuotaPollToggle = (enabled: boolean) => {
    setBackgroundCliQuotaPoll(enabled);
    void persistPreferences({ background_cli_quota_poll: enabled }).catch((err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '切换后台 CLI 配额轮询设置失败',
        kind: 'error',
      });
    });
  };

  const handleFloatingElementToggle = (id: FloatingModuleId) => {
    const nextCfg = updateFloatingConfig({
      modules: toggleFloatingModule(floatingConfig.modules, id),
    });
    if (!nextCfg.compact) {
      const dims = computeFloatingDimensions(nextCfg);
      void ipc.setFloatingWindowCompact(false, dims.width, dims.height);
    }
  };

  const updateDownloadPercent =
    updater.totalBytes && updater.totalBytes > 0
      ? Math.min(100, (updater.downloadedBytes / updater.totalBytes) * 100)
      : updater.status === 'installing'
        ? 100
        : 0;

  const previewSyncMutation = useMutation({
    mutationFn: () => ipc.previewSyncEnvelopeV1(0),
    onSuccess: (env) => {
      setSyncEnvelope(env);
    },
  });

  const loadPairingMutation = useMutation({
    mutationFn: (includePassphrase: boolean) => ipc.getSyncPairingBundle(includePassphrase),
    onSuccess: (bundle: SyncPairingBundle) => {
      setPairingBundle(bundle);
      setPairingExpanded(true);
    },
    onError: (err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '生成配对码失败',
        kind: 'error',
      });
    },
  });

  const saveCloudSyncSetupMutation = useMutation({
    mutationFn: () => {
      const workerUrl = syncWorkerUrlDraft.trim() || null;
      const token = syncBearerTokenInput.trim() || null;
      const pass = syncE2eeDraft ? syncPassphraseInput.trim() || null : null;
      return ipc.saveCloudSyncSetup(
        syncEnabledDraft,
        workerUrl,
        token,
        syncE2eeDraft,
        pass
      );
    },
    onSuccess: async (status: SyncStatusView) => {
      lastSyncCfgSnapshotRef.current = {
        enabled: status.enabled,
        worker_url: status.worker_url ?? '',
        e2ee_enabled: status.e2ee_enabled,
      };
      setSyncBearerTokenInput('');
      setSyncPassphraseInput('');
      queryClient.setQueryData<SyncStatusView>(['sync-status'], status);
      await queryClient.invalidateQueries({ queryKey: ['sync-status'] });
      if (status.enabled && status.worker_url && status.has_bearer_token) {
        try {
          const bundle = await ipc.getSyncPairingBundle(includePassphraseInPairing);
          setPairingBundle(bundle);
          setPairingExpanded(true);
        } catch {
          setPairingBundle(null);
        }
      } else {
        setPairingBundle(null);
        setPairingExpanded(false);
      }
      setNotice({ text: '云端同步已保存并应用', kind: 'success' });
    },
    onError: (err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '保存云端同步失败',
        kind: 'error',
      });
    },
  });

  const provisionWorkerMutation = useMutation({
    mutationFn: async () => {
      const mode = deployMode;
      setProvisionRun({ mode, steps: [], status: 'running', startedAtMs: Date.now() });
      const unlisten = await onCloudSyncProvisionProgress((step) => {
        setProvisionRun((prev) =>
          prev && !prev.steps.includes(step) ? { ...prev, steps: [...prev.steps, step] } : prev
        );
      });
      try {
        // One-click setup reuses the token / passphrase stored in the system
        // credential vault (or generates fresh ones); manual drafts are ignored.
        return await ipc.provisionCloudSyncWorker({
          mode,
          worker_name: workerNameDraft.trim() || null,
          bearer_token: null,
          e2ee_enabled: syncE2eeDraft,
          e2ee_passphrase: null,
          cf_api_token: mode === 'cloudflare_api' ? cfApiTokenInput.trim() || null : null,
          cf_account_id: mode === 'cloudflare_api' ? cfAccountIdInput.trim() || null : null,
        });
      } finally {
        unlisten();
      }
    },
    onSuccess: async (res: CloudSyncProvisionResult) => {
      setProvisionRun(null);
      lastSyncCfgSnapshotRef.current = {
        enabled: res.status.enabled,
        worker_url: res.worker_url,
        e2ee_enabled: res.status.e2ee_enabled,
      };
      setSyncEnabledDraft(true);
      setSyncWorkerUrlDraft(res.worker_url);
      setSyncBearerTokenInput(res.bearer_token);
      setSyncPassphraseInput(res.e2ee_passphrase ?? '');
      setWorkerNameDraft(res.worker_name);
      setCfApiTokenInput('');
      queryClient.setQueryData<SyncStatusView>(['sync-status'], res.status);
      await queryClient.invalidateQueries({ queryKey: ['sync-status'] });
      await queryClient.invalidateQueries({ queryKey: ['devices'] });
      await queryClient.invalidateQueries({ queryKey: ['remote-subscriptions'] });
      setIncludePassphraseInPairing(res.pairing.includes_passphrase);
      setPairingBundle(res.pairing);
      setPairingExpanded(true);
      setNotice({
        text: res.initial_sync_ok
          ? 'Worker 已部署并完成首次同步，可用移动端扫码配对'
          : 'Worker 已部署，首次同步将在边缘路由生效后自动重试',
        kind: 'success',
      });
    },
    onError: (err: unknown) => {
      setProvisionRun((prev) => (prev ? { ...prev, status: 'failed' } : prev));
      setNotice({
        text: err instanceof Error ? `一键配置失败：${err.message}` : '一键配置失败',
        kind: 'error',
      });
    },
  });

  const clearSyncCredsMutation = useMutation({
    mutationFn: () => ipc.clearSyncCredentials(),
    onSuccess: async (status: SyncStatusView) => {
      setSyncBearerTokenInput('');
      setSyncPassphraseInput('');
      setCfApiTokenInput('');
      setPairingBundle(null);
      setPairingExpanded(false);
      queryClient.setQueryData<SyncStatusView>(['sync-status'], status);
      await queryClient.invalidateQueries({ queryKey: ['sync-status'] });
      setNotice({ text: '已从系统凭据库清除云端同步凭据', kind: 'success' });
    },
    onError: (err: unknown) => {
      setNotice({
        text: err instanceof Error ? err.message : '清除同步凭据失败',
        kind: 'error',
      });
    },
  });

  const triggerCloudSyncMutation = useMutation({
    mutationFn: () => ipc.triggerCloudSyncNow(),
    onSuccess: async (report) => {
      await queryClient.invalidateQueries({ queryKey: ['sync-status'] });
      await queryClient.invalidateQueries({ queryKey: ['devices'] });
      await queryClient.invalidateQueries({ queryKey: ['remote-subscriptions'] });
      await queryClient.invalidateQueries({ queryKey: ['overview'] });
      await queryClient.invalidateQueries({ queryKey: ['trends'] });
      await queryClient.invalidateQueries({ queryKey: ['monthly'] });
      await queryClient.invalidateQueries({ queryKey: ['models_pricing'] });
      await queryClient.invalidateQueries({ queryKey: ['speed_latency'] });
      await queryClient.invalidateQueries({ queryKey: [TRAY_SUMMARY_QUERY_KEY] });
      setNotice({
        text: `云端同步完成：上传 ${report.pushed_hourly_deltas} 条小时增量 / ${report.pushed_subscriptions} 条订阅快照，拉取并合并 ${report.applied_hourly_buckets} 条远端小时聚合`,
        kind: 'success',
      });
    },
    onError: (err: unknown) => {
      void queryClient.invalidateQueries({ queryKey: ['sync-status'] });
      setNotice({
        text: err instanceof Error ? err.message : '云端同步失败',
        kind: 'error',
      });
    },
  });

  const envelopeJson = syncEnvelope ? JSON.stringify(syncEnvelope, null, 2) : null;

  return (
    <div className="space-y-6">
      <section
        className="flex flex-wrap items-center justify-between gap-4 pb-4"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <div className="flex items-center gap-2">
          <h1 className="text-[16px] font-semibold tracking-tight">偏好设置</h1>
          <span
            className="px-2 py-0.5 rounded-md text-[10.5px] font-mono tabular-nums"
            style={{
              backgroundColor: 'var(--accent-tonal-bg)',
              boxShadow: 'var(--border-specular)',
              border: '1px solid var(--border-hairline)',
              color: 'var(--accent-tonal-text)',
            }}
          >
            v{APP_VERSION}
          </span>
        </div>

        <button
          type="button"
          onClick={() => savePrefsMutation.mutate()}
          disabled={savePrefsMutation.isPending}
          className="btn-primary inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium cursor-pointer disabled:opacity-50"
          style={{
            backgroundColor: 'var(--accent-primary)',
            color: '#ffffff',
          }}
        >
          <Save className="w-3.5 h-3.5" />
          <span>{savePrefsMutation.isPending ? '保存中…' : '保存设置'}</span>
        </button>
      </section>

      <ActionNotice text={notice?.text ?? null} kind={notice?.kind} onClose={clearNotice} />

      {/* 1. 外观 */}
      <section className="space-y-1">
        <div
          className="pb-2 text-[11px] font-semibold uppercase tracking-wider"
          style={{
            color: 'var(--text-muted)',
            borderBottom: '1px solid var(--border-subtle)',
          }}
        >
          外观
        </div>

        <div
          className="py-3.5 grid grid-cols-12 gap-4 items-center"
          style={{ borderBottom: '1px solid var(--border-hairline)' }}
        >
          <div className="col-span-4">
            <div className="text-[13px] font-medium">深色/浅色模式</div>
          </div>
          <div className="col-span-8 flex flex-wrap items-center gap-2">
            {THEME_OPTIONS.map((opt) => {
              const active = theme === opt.id;
              return (
                <button
                  key={opt.id}
                  type="button"
                  data-active={active ? 'true' : undefined}
                  onClick={() => handleThemeSelect(opt.id)}
                  className="seg-pill px-3 py-1 rounded-lg text-[12px] cursor-pointer"
                  style={{
                    backgroundColor: active ? 'var(--bg-hover)' : 'var(--bg-input)',
                    border: active
                      ? '1px solid var(--accent-primary)'
                      : '1px solid var(--border-subtle)',
                    color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
                    fontWeight: active ? 500 : 400,
                  }}
                >
                  {opt.label}
                </button>
              );
            })}
          </div>
        </div>

        <div
          className="py-3.5 grid grid-cols-12 gap-4 items-start"
          style={{ borderBottom: '1px solid var(--border-subtle)' }}
        >
          <div className="col-span-4 pt-1">
            <div className="text-[13px] font-medium">自定义背景</div>
          </div>

          <div className="col-span-8 space-y-3.5">
            <div className="seg-group">
              {BG_MODE_OPTIONS.map((modeOpt) => {
                const active = bgMode === modeOpt.id;
                return (
                  <button
                    key={modeOpt.id}
                    type="button"
                    data-active={active ? 'true' : undefined}
                    onClick={() => handleBgModeSelect(modeOpt.id)}
                    className="seg-pill px-3 py-1 text-[12px] cursor-pointer"
                    style={{
                      color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
                      fontWeight: active ? 500 : 400,
                    }}
                  >
                    {modeOpt.label}
                  </button>
                );
              })}
            </div>

            {bgMode === 'gradient' && (
              <div className="space-y-2">
                <div
                  className="flex items-center gap-1.5 text-[11.5px]"
                  style={{ color: 'var(--text-secondary)' }}
                >
                  <Sparkles className="w-3.5 h-3.5" style={{ color: 'var(--accent-primary)' }} />
                  <span>渐变预设</span>
                </div>
                <div className="grid grid-cols-2 sm:grid-cols-4 gap-2.5">
                  {BACKGROUND_GRADIENT_PRESETS.map((preset) => {
                    const selected = bgPresetId === preset.id;
                    return (
                      <button
                        key={preset.id}
                        type="button"
                        data-selected={selected ? 'true' : undefined}
                        onClick={() => handlePresetSelect(preset.id)}
                        className="btn-interactive text-left p-2 rounded-lg cursor-pointer space-y-1.5"
                        style={{
                          backgroundColor: selected ? 'var(--bg-hover)' : 'var(--bg-input)',
                          border: selected
                            ? '1px solid var(--accent-primary)'
                            : '1px solid var(--border-subtle)',
                        }}
                      >
                        <div
                          className="h-7 w-full rounded-md overflow-hidden"
                          style={{
                            backgroundColor: 'var(--bg-canvas)',
                            border: '1px solid var(--border-hairline)',
                          }}
                        >
                          <div
                            className="w-full h-full"
                            style={{
                              backgroundImage: preset.gradients[resolvedTheme],
                              opacity: 0.72,
                            }}
                          />
                        </div>
                        <div
                          className="text-[12px] font-medium leading-tight"
                          style={{ color: 'var(--text-primary)' }}
                        >
                          {preset.label}
                        </div>
                      </button>
                    );
                  })}
                </div>
              </div>
            )}

            {bgMode === 'image' && (
              <div
                className="flex flex-wrap items-center justify-between gap-3 px-3 py-2.5 rounded-lg"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                }}
              >
                <div className="flex items-center gap-2 min-w-0">
                  <ImageIcon
                    className="w-4 h-4 shrink-0"
                    style={{ color: 'var(--text-muted)' }}
                  />
                  <div
                    className="text-[12px] truncate"
                    style={{ color: 'var(--text-primary)' }}
                  >
                    {bgImagePath ? `当前壁纸：${bgImagePath}` : '可选择本地壁纸图片'}
                  </div>
                </div>

                <div className="flex items-center gap-2 shrink-0">
                  <button
                    type="button"
                    onClick={() => void handlePickBackgroundImage()}
                    disabled={importBgMutation.isPending}
                    className="btn-interactive px-2.5 py-1 rounded-lg text-[11.5px] font-medium cursor-pointer disabled:opacity-50"
                    style={{
                      backgroundColor: 'var(--bg-elevated)',
                      border: '1px solid var(--border-subtle)',
                      color: 'var(--text-primary)',
                    }}
                  >
                    {importBgMutation.isPending
                      ? '导入中…'
                      : bgImagePath
                        ? '更换壁纸'
                        : '选择壁纸'}
                  </button>

                  {bgImagePath && (
                    <button
                      type="button"
                      onClick={() => clearBgMutation.mutate()}
                      disabled={clearBgMutation.isPending}
                      className="btn-interactive inline-flex items-center gap-1 px-2.5 py-1 rounded-lg text-[11.5px] cursor-pointer disabled:opacity-50"
                      style={{
                        backgroundColor: 'var(--bg-elevated)',
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-secondary)',
                      }}
                    >
                      <RotateCcw className="w-3 h-3" />
                      <span>{clearBgMutation.isPending ? '清除中…' : '清除壁纸'}</span>
                    </button>
                  )}
                </div>
              </div>
            )}

            {bgMode !== 'none' && (
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 pt-1">
                <label className="space-y-1.5 block">
                  <div className="flex items-center justify-between text-[11.5px]">
                    <span style={{ color: 'var(--text-secondary)' }}>背景可见度</span>
                    <span
                      className="font-mono tabular-nums"
                      style={{ color: 'var(--text-primary)' }}
                    >
                      {Math.round(bgOpacity * 100)}%
                    </span>
                  </div>
                  <input
                    type="range"
                    aria-label="背景可见度"
                    min={0.05}
                    max={0.6}
                    step={0.01}
                    value={bgOpacity}
                    onChange={(e) => {
                      const val = Number.parseFloat(e.target.value);
                      if (Number.isFinite(val)) {
                        handleSliderPreview({ bg_opacity: clampOpacity(val) });
                      }
                    }}
                    onPointerUp={flushSliderPersist}
                    onKeyUp={flushSliderPersist}
                    onBlur={flushSliderPersist}
                    className="w-full cursor-pointer accent-[var(--accent-primary)]"
                  />
                </label>

                <label className="space-y-1.5 block">
                  <div className="flex items-center justify-between text-[11.5px]">
                    <span style={{ color: 'var(--text-secondary)' }}>模糊度</span>
                    <span
                      className="font-mono tabular-nums"
                      style={{ color: 'var(--text-primary)' }}
                    >
                      {bgBlurPx}px
                    </span>
                  </div>
                  <input
                    type="range"
                    aria-label="模糊度"
                    min={0}
                    max={32}
                    step={1}
                    value={bgBlurPx}
                    onChange={(e) => {
                      const val = Number.parseInt(e.target.value, 10);
                      if (Number.isFinite(val)) {
                        handleSliderPreview({ bg_blur_px: clampBlurPx(val) });
                      }
                    }}
                    onPointerUp={flushSliderPersist}
                    onKeyUp={flushSliderPersist}
                    onBlur={flushSliderPersist}
                    className="w-full cursor-pointer accent-[var(--accent-primary)]"
                  />
                </label>
              </div>
            )}
          </div>
        </div>
      </section>

      {/* 2. 桌面与悬浮窗 */}
      <section className="space-y-1">
        <div
          className="pb-2 text-[11px] font-semibold uppercase tracking-wider"
          style={{
            color: 'var(--text-muted)',
            borderBottom: '1px solid var(--border-subtle)',
          }}
        >
          桌面与悬浮窗
        </div>

        <div
          className="py-3.5 grid grid-cols-12 gap-4 items-center"
          style={{ borderBottom: '1px solid var(--border-hairline)' }}
        >
          <div className="col-span-4">
            <div className="text-[13px] font-medium">窗口与启动</div>
          </div>
          <div className="col-span-8 flex flex-wrap items-center gap-6 text-[12px]">
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={minimizeToTray}
                onChange={(e) => handleMinimizeToTrayToggle(e.target.checked)}
              />
              <span>关闭窗口时最小化至托盘</span>
            </label>
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={launchAtStartup}
                onChange={(e) => handleLaunchAtStartupToggle(e.target.checked)}
              />
              <span>Windows 登录后自动启动</span>
            </label>
          </div>
        </div>

        <div
          className="py-3.5 grid grid-cols-12 gap-4 items-start"
          style={{ borderBottom: '1px solid var(--border-hairline)' }}
        >
          <div className="col-span-4 pt-0.5">
            <div className="text-[13px] font-medium">后台 CLI 配额轮询</div>
          </div>
          <div className="col-span-8 space-y-1.5 text-[12px]">
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={backgroundCliQuotaPoll}
                onChange={(e) => handleBackgroundCliQuotaPollToggle(e.target.checked)}
              />
              <span>后台自动调用官方 CLI 刷新配额（Antigravity / Claude Code）</span>
            </label>
            <p className="text-[11px] leading-relaxed" style={{ color: 'var(--text-muted)' }}>
              默认开启静默轮询：仅对已成功同步过至少一次的 CLI（Antigravity / Claude Code）在后台每 5 分钟隐藏窗口探测，自动注入禁浏览器变量；检测到全屏游戏或演示时自动跳过，遇到未登录或超时将持久化暂停直到手动同步。关闭后仅保留手动刷新，本地日志采集不受影响。
            </p>
          </div>
        </div>

        <div
          className="py-3.5 grid grid-cols-12 gap-4 items-start"
          style={{ borderBottom: '1px solid var(--border-subtle)' }}
        >
          <div className="col-span-4 pt-0.5">
            <div className="text-[13px] font-medium">桌面悬浮窗</div>
          </div>
          <div className="col-span-8 space-y-3 text-[12px]">
            <div className="flex flex-wrap items-center gap-4">
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={floatingWindowEnabled}
                  onChange={(e) => handleFloatingWindowToggle(e.target.checked)}
                />
                <span>显示桌面实时悬浮窗</span>
              </label>
            </div>

            {floatingWindowEnabled && (
              <>
                <div className="flex flex-wrap items-center gap-1.5">
                  {FLOATING_MODULES.map((item) => {
                    const enabled = floatingConfig.modules.includes(item.id);
                    return (
                      <button
                        key={item.id}
                        type="button"
                        aria-pressed={enabled}
                        onClick={() => handleFloatingElementToggle(item.id)}
                        className="btn-interactive px-2 py-0.5 rounded-lg text-[11px] cursor-pointer"
                        style={{
                          backgroundColor: enabled ? 'var(--bg-hover)' : 'var(--bg-input)',
                          border: enabled
                            ? '1px solid var(--accent-primary)'
                            : '1px solid var(--border-subtle)',
                          color: enabled ? 'var(--text-primary)' : 'var(--text-muted)',
                          fontWeight: enabled ? 500 : 400,
                        }}
                      >
                        {item.label}
                      </button>
                    );
                  })}
                </div>

                <div className="max-w-xs">
                  <label className="space-y-1 block">
                    <div className="flex items-center justify-between text-[11.5px]">
                      <span style={{ color: 'var(--text-secondary)' }}>悬浮窗不透明度</span>
                      <span
                        className="font-mono tabular-nums"
                        style={{ color: 'var(--text-primary)' }}
                      >
                        {Math.round(floatingConfig.opacity * 100)}%
                      </span>
                    </div>
                    <input
                      type="range"
                      aria-label="悬浮窗不透明度"
                      min={0.55}
                      max={0.98}
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
              </>
            )}
          </div>
        </div>
      </section>

      {/* 3. 货币与定价 */}
      <section className="space-y-1">
        <div
          className="pb-2 text-[11px] font-semibold uppercase tracking-wider"
          style={{
            color: 'var(--text-muted)',
            borderBottom: '1px solid var(--border-subtle)',
          }}
        >
          货币与定价
        </div>

        <div
          className="py-3.5 grid grid-cols-12 gap-4 items-center"
          style={{ borderBottom: '1px solid var(--border-hairline)' }}
        >
          <div className="col-span-4">
            <div className="text-[13px] font-medium">显示币种与汇率</div>
          </div>
          <div className="col-span-8 flex flex-wrap items-center gap-3">
            <select
              aria-label="显示币种"
              value={currencyDraft}
              onChange={(e) => setCurrencyDraft(e.target.value as CurrencyCode)}
              className="px-2.5 py-1 text-[12px] rounded-lg outline-none cursor-pointer"
              style={{
                backgroundColor: 'var(--bg-input)',
                border: '1px solid var(--border-subtle)',
                color: 'var(--text-primary)',
              }}
            >
              <option value="USD">USD（美元）</option>
              <option value="CNY">CNY（人民币）</option>
            </select>

            <div className="seg-group">
              <button
                type="button"
                data-active={exchangeRateAuto ? 'true' : undefined}
                onClick={() => handleExchangeRateAutoToggle(true)}
                className="seg-pill px-2.5 py-0.5 text-[11.5px] cursor-pointer"
                style={{
                  color: exchangeRateAuto ? 'var(--text-primary)' : 'var(--text-secondary)',
                  fontWeight: exchangeRateAuto ? 500 : 400,
                }}
              >
                自动汇率
              </button>
              <button
                type="button"
                data-active={!exchangeRateAuto ? 'true' : undefined}
                onClick={() => handleExchangeRateAutoToggle(false)}
                className="seg-pill px-2.5 py-0.5 text-[11.5px] cursor-pointer"
                style={{
                  color: !exchangeRateAuto ? 'var(--text-primary)' : 'var(--text-secondary)',
                  fontWeight: !exchangeRateAuto ? 500 : 400,
                }}
              >
                手动锁定
              </button>
            </div>

            <label className="flex items-center gap-2 text-[12px]">
              <span style={{ color: 'var(--text-secondary)' }}>1 USD =</span>
              <input
                type="text"
                aria-label="USD 对 CNY 汇率"
                readOnly={exchangeRateAuto}
                value={rateDraft}
                onChange={(e) => setRateDraft(e.target.value)}
                className="w-24 px-2 py-1 text-[12px] rounded-lg outline-none tabular-nums"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                  color: exchangeRateAuto ? 'var(--text-secondary)' : 'var(--text-primary)',
                }}
              />
              <span style={{ color: 'var(--text-secondary)' }}>CNY</span>
            </label>

            <button
              type="button"
              disabled={syncNowMutation.isPending}
              onClick={() => syncNowMutation.mutate()}
              className="btn-interactive inline-flex items-center gap-1.5 px-2.5 py-1 text-[11.5px] rounded-lg cursor-pointer disabled:opacity-50"
              style={{
                backgroundColor: 'var(--bg-elevated)',
                border: '1px solid var(--border-subtle)',
                color: 'var(--text-primary)',
              }}
            >
              <RefreshCw
                className={`w-3 h-3 ${syncNowMutation.isPending ? 'animate-spin' : ''}`}
              />
              <span>{syncNowMutation.isPending ? '刷新中…' : '刷新目录与汇率'}</span>
            </button>
          </div>
        </div>

        <div
          className="py-3.5 grid grid-cols-12 gap-4 items-center"
          style={{ borderBottom: '1px solid var(--border-subtle)' }}
        >
          <div className="col-span-4">
            <div className="text-[13px] font-medium">模型与套餐目录同步</div>
          </div>
          <div className="col-span-8 flex flex-wrap items-center gap-4 text-[11.5px]">
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={autoSyncPricing}
                onChange={(e) => handleAutoSyncPricingToggle(e.target.checked)}
              />
              <span style={{ color: 'var(--text-secondary)' }}>
                自动联网更新模型价格、汇率与 AI 订阅目录
              </span>
            </label>
            <span className="tabular-nums" style={{ color: 'var(--text-muted)' }}>
              {activePrefs?.last_pricing_sync_at_ms
                ? `定价同步于 ${formatRelativeTime(activePrefs.last_pricing_sync_at_ms)}`
                : '定价使用内置基线'}
              {' · '}
              {activePrefs?.last_rate_sync_at_ms
                ? `汇率同步于 ${formatRelativeTime(activePrefs.last_rate_sync_at_ms)}`
                : '汇率待首次同步'}
              {' · '}
              {activePrefs?.last_subscription_sync_at_ms
                ? `订阅目录同步于 ${formatRelativeTime(activePrefs.last_subscription_sync_at_ms)}`
                : '订阅目录使用内置基线'}
            </span>
          </div>
        </div>
      </section>

      {/* 4. 系统与更新 */}
      <section className="space-y-1">
        <div
          className="pb-2 text-[11px] font-semibold uppercase tracking-wider"
          style={{
            color: 'var(--text-muted)',
            borderBottom: '1px solid var(--border-subtle)',
          }}
        >
          系统与更新
        </div>

        <div
          className="py-3.5 grid grid-cols-12 gap-4 items-center"
          style={{ borderBottom: '1px solid var(--border-hairline)' }}
        >
          <div className="col-span-4">
            <div className="text-[13px] font-medium">版本更新</div>
          </div>
          <div className="col-span-8 space-y-2 text-[12px]">
            <div className="flex flex-wrap items-center gap-3">
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={autoCheckUpdates}
                  onChange={(e) => handleAutoCheckUpdatesToggle(e.target.checked)}
                />
                <span>启动时与每 12 小时自动检查客户端更新</span>
              </label>

              {updater.availableUpdate ? (
                <button
                  type="button"
                  disabled={updater.status === 'downloading' || updater.status === 'installing'}
                  onClick={() => void downloadAndInstallUpdate()}
                  className="btn-primary inline-flex items-center gap-1.5 px-3 py-1 rounded-lg text-[11.5px] font-medium cursor-pointer disabled:opacity-50"
                  style={{
                    backgroundColor: 'var(--accent-emerald)',
                    color: '#ffffff',
                  }}
                >
                  <Download className="w-3.5 h-3.5" />
                  <span>
                    {updater.status === 'downloading'
                      ? `下载中 ${updateDownloadPercent.toFixed(0)}%`
                      : updater.status === 'installing'
                        ? '正在安装并重启…'
                        : `立即更新 v${updater.availableUpdate.version}`}
                  </span>
                </button>
              ) : (
                <button
                  type="button"
                  disabled={
                    updater.status === 'checking' ||
                    updater.status === 'downloading' ||
                    updater.status === 'installing'
                  }
                  onClick={() => void checkForUpdates(true)}
                  className="btn-interactive inline-flex items-center gap-1.5 px-2.5 py-1 text-[11.5px] rounded-lg cursor-pointer disabled:opacity-50"
                  style={{
                    backgroundColor: 'var(--bg-elevated)',
                    border: '1px solid var(--border-subtle)',
                    color: 'var(--text-primary)',
                  }}
                >
                  <RefreshCw
                    className={`w-3 h-3 ${updater.status === 'checking' ? 'animate-spin' : ''}`}
                  />
                  <span>{updater.status === 'checking' ? '检查中…' : '检查更新'}</span>
                </button>
              )}

              {(updater.status === 'error' ||
                updater.status === 'up_to_date' ||
                updater.availableUpdate ||
                updater.lastCheckedAtMs) && (
                <span className="text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
                  {updater.status === 'error' && updater.errorMessage
                    ? `检查更新受限：${updater.errorMessage}`
                    : updater.status === 'up_to_date'
                      ? '当前已是最新版本'
                      : updater.availableUpdate
                        ? `检测到新版本 v${updater.availableUpdate.version}`
                        : updater.lastCheckedAtMs
                          ? `上次检查于 ${formatRelativeTime(updater.lastCheckedAtMs)}`
                          : null}
                </span>
              )}
            </div>

            {(updater.status === 'downloading' || updater.status === 'installing') && (
              <div className="max-w-xs">
                <InlineProgressBar
                  percent={updateDownloadPercent}
                  accentColor="var(--accent-emerald)"
                  heightPx={4}
                />
              </div>
            )}
          </div>
        </div>

        <div
          className="py-3.5 grid grid-cols-12 gap-4 items-center"
          style={{ borderBottom: '1px solid var(--border-subtle)' }}
        >
          <div className="col-span-4">
            <div className="text-[13px] font-medium">设备与存储</div>
          </div>
          <div className="col-span-8 space-y-2 text-[12px]">
            <div className="flex flex-wrap items-center gap-3">
              <span style={{ color: 'var(--text-secondary)' }}>设备显示名：</span>
              <input
                type="text"
                value={deviceDisplayName}
                onChange={(e) => setDeviceDisplayName(e.target.value)}
                className="w-56 px-2.5 py-1 text-[12px] rounded-lg outline-none"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--text-primary)',
                }}
              />
              <span
                className="text-[11px] font-mono tabular-nums"
                style={{ color: 'var(--text-muted)' }}
              >
                ID: {bootstrap?.device.device_id ?? '—'}
              </span>
            </div>
            <div className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
              SQLite WAL 数据库路径：
              <code className="ml-1 select-text">{bootstrap?.db_path ?? '—'}</code>
            </div>
          </div>
        </div>
      </section>

      {/* 5. 云端同步 */}
      <section className="space-y-4 pb-6">
        <div
          className="flex flex-wrap items-center justify-between gap-3 pb-2"
          style={{ borderBottom: '1px solid var(--border-subtle)' }}
        >
          <div className="flex items-center gap-2">
            <Cloud className="w-4 h-4" style={{ color: 'var(--accent-sky)' }} />
            <h2
              className="text-[11px] font-semibold uppercase tracking-wider"
              style={{ color: 'var(--text-muted)' }}
            >
              云端同步
            </h2>
          </div>

          <div className="flex flex-wrap items-center gap-2 text-[11px] tabular-nums">
            <span
              className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md"
              style={{
                backgroundColor: 'var(--bg-elevated)',
                border: '1px solid var(--border-subtle)',
                color: syncStatus?.sse_connected ? 'var(--accent-emerald)' : 'var(--text-muted)',
              }}
            >
              <span
                className="inline-block w-1.5 h-1.5 rounded-full"
                style={{
                  backgroundColor: syncStatus?.sse_connected
                    ? 'var(--accent-emerald)'
                    : 'var(--text-muted)',
                }}
              />
              {syncStatus?.sse_connected ? 'SSE 实时已连接' : 'SSE 待机'}
            </span>
            <button
              type="button"
              onClick={() => triggerCloudSyncMutation.mutate()}
              disabled={triggerCloudSyncMutation.isPending}
              className="btn-interactive inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-[12px] font-medium cursor-pointer disabled:opacity-50"
              style={{
                backgroundColor: 'var(--bg-elevated)',
                color: 'var(--text-primary)',
                border: '1px solid var(--border-strong)',
              }}
            >
              <RefreshCw
                className={`w-3.5 h-3.5 ${triggerCloudSyncMutation.isPending ? 'animate-spin' : ''}`}
              />
              <span>{triggerCloudSyncMutation.isPending ? '同步中…' : '立即同步'}</span>
            </button>
          </div>
        </div>

        {/* Unified Setup Form */}
        <div
          className="pb-4 grid grid-cols-12 gap-4 items-start"
          style={{ borderBottom: '1px solid var(--border-hairline)' }}
        >
          <div className="col-span-4 space-y-1.5">
            <div className="flex items-center gap-1.5 text-[13px] font-medium">
              <KeyRound className="w-3.5 h-3.5" style={{ color: 'var(--text-secondary)' }} />
              <span>连接配置</span>
            </div>
            <div className="text-[11px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
              {syncStatus?.last_sync_at_ms
                ? `最近同步 ${formatRelativeTime(syncStatus.last_sync_at_ms)}`
                : '等待首次同步'}
            </div>
          </div>

          <div className="col-span-8 space-y-3 text-[12px]">
            <div className="flex flex-wrap items-center gap-4">
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={syncEnabledDraft}
                  onChange={(e) => setSyncEnabledDraft(e.target.checked)}
                />
                <span>启用云端同步</span>
              </label>
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={syncE2eeDraft}
                  onChange={(e) => setSyncE2eeDraft(e.target.checked)}
                />
                <span className="inline-flex items-center gap-1">
                  <ShieldCheck className="w-3.5 h-3.5" style={{ color: 'var(--accent-emerald)' }} />
                  <span>端到端加密 (AES-256-GCM)</span>
                </span>
              </label>
            </div>

            <div className="seg-group" role="group" aria-label="连接方式">
              {(
                [
                  { id: 'oneclick', label: '一键部署' },
                  { id: 'manual', label: '连接已有 Worker' },
                ] as const
              ).map((opt) => (
                <button
                  key={opt.id}
                  type="button"
                  data-active={connectMode === opt.id ? 'true' : undefined}
                  onClick={() => setConnectMode(opt.id)}
                  className="seg-pill px-2.5 py-0.5 text-[11.5px] cursor-pointer whitespace-nowrap"
                  style={{
                    color: connectMode === opt.id ? 'var(--text-primary)' : 'var(--text-secondary)',
                    fontWeight: connectMode === opt.id ? 500 : 400,
                  }}
                >
                  {opt.label}
                </button>
              ))}
            </div>

            {connectMode === 'oneclick' ? (
              <div className="space-y-3">
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <div className="seg-group" role="group" aria-label="Worker 部署方式">
                    {(
                      [
                        { id: 'wrangler_cli', label: 'Wrangler 浏览器授权' },
                        { id: 'cloudflare_api', label: 'Cloudflare API 令牌' },
                      ] as const
                    ).map((opt) => (
                      <button
                        key={opt.id}
                        type="button"
                        data-active={deployMode === opt.id ? 'true' : undefined}
                        onClick={() => setDeployMode(opt.id)}
                        className="seg-pill px-2.5 py-0.5 text-[11.5px] cursor-pointer whitespace-nowrap"
                        style={{
                          color: deployMode === opt.id ? 'var(--text-primary)' : 'var(--text-secondary)',
                          fontWeight: deployMode === opt.id ? 500 : 400,
                        }}
                      >
                        {opt.label}
                      </button>
                    ))}
                  </div>

                  <label className="flex items-center gap-2 text-[11.5px]">
                    <span style={{ color: 'var(--text-secondary)' }}>Worker 名称</span>
                    <input
                      type="text"
                      aria-label="Worker 名称"
                      placeholder="tokenvane-sync-worker"
                      value={workerNameDraft}
                      onChange={(e) => setWorkerNameDraft(e.target.value)}
                      className="w-48 px-2.5 py-1 text-[11.5px] font-mono rounded-lg outline-none"
                      style={SYNC_FIELD_STYLE}
                    />
                  </label>
                </div>

                {deployMode === 'cloudflare_api' && (
                  <div className="grid grid-cols-12 gap-2.5 items-end">
                    <div className="col-span-8 space-y-1">
                      <div className="flex items-center justify-between text-[11px]">
                        <span style={{ color: 'var(--text-secondary)' }}>Cloudflare API 令牌</span>
                        <span
                          style={{
                            color: syncStatus?.has_cf_api_token
                              ? 'var(--accent-emerald)'
                              : 'var(--text-muted)',
                          }}
                        >
                          {syncStatus?.has_cf_api_token ? '已存入凭据库' : '待录入'}
                        </span>
                      </div>
                      <input
                        type="password"
                        aria-label="Cloudflare API 令牌"
                        placeholder={
                          syncStatus?.has_cf_api_token ? '输入新值可更新' : 'Workers 编辑权限 Token'
                        }
                        value={cfApiTokenInput}
                        onChange={(e) => setCfApiTokenInput(e.target.value)}
                        className="w-full px-2.5 py-1.5 text-[12px] font-mono rounded-lg outline-none"
                        style={SYNC_FIELD_STYLE}
                      />
                    </div>
                    <div className="col-span-4 space-y-1">
                      <span className="block text-[11px]" style={{ color: 'var(--text-secondary)' }}>
                        Account ID
                      </span>
                      <input
                        type="text"
                        aria-label="Cloudflare Account ID"
                        placeholder="自动识别"
                        value={cfAccountIdInput}
                        onChange={(e) => setCfAccountIdInput(e.target.value)}
                        className="w-full px-2.5 py-1.5 text-[12px] font-mono rounded-lg outline-none"
                        style={SYNC_FIELD_STYLE}
                      />
                    </div>
                  </div>
                )}

                <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
                  <button
                    type="button"
                    onClick={() => provisionWorkerMutation.mutate()}
                    disabled={provisionWorkerMutation.isPending}
                    className="btn-primary inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium cursor-pointer disabled:opacity-50 whitespace-nowrap"
                    style={{
                      backgroundColor: 'var(--accent-primary)',
                      color: '#ffffff',
                    }}
                  >
                    <Wand2 className="w-3.5 h-3.5" />
                    <span>{provisionWorkerMutation.isPending ? '一键配置中…' : '一键配置'}</span>
                  </button>
                  {provisionRun && (
                    <ProvisionProgress
                      mode={provisionRun.mode}
                      seen={provisionRun.steps}
                      status={provisionRun.status}
                      startedAtMs={provisionRun.startedAtMs}
                    />
                  )}
                </div>

                {(syncWorkerUrlDraft || syncStatus?.worker_url) && (
                  <div
                    data-testid="sync-connection-summary"
                    className="grid grid-cols-12 gap-2.5 pt-2.5"
                    style={{ borderTop: '1px solid var(--border-hairline)' }}
                  >
                    <label className="col-span-12 space-y-1">
                      <span className="block text-[11px]" style={{ color: 'var(--text-secondary)' }}>
                        Worker 服务地址
                      </span>
                      <input
                        type="url"
                        readOnly
                        aria-label="Worker 服务地址"
                        value={syncWorkerUrlDraft || syncStatus?.worker_url || ''}
                        onFocus={(e) => e.currentTarget.select()}
                        className="w-full px-2.5 py-1.5 text-[12px] font-mono rounded-lg outline-none"
                        style={SYNC_FIELD_STYLE}
                      />
                    </label>
                    <label className={`${syncE2eeDraft ? 'col-span-6' : 'col-span-12'} space-y-1`}>
                      <span className="flex items-center justify-between text-[11px]">
                        <span style={{ color: 'var(--text-secondary)' }}>访问令牌</span>
                        {syncStatus?.has_bearer_token && (
                          <span style={{ color: 'var(--accent-emerald)' }}>已存入凭据库</span>
                        )}
                      </span>
                      <input
                        type="password"
                        readOnly
                        aria-label="Sync Bearer Token"
                        value={syncBearerTokenInput}
                        placeholder={syncStatus?.has_bearer_token ? '••••••••' : ''}
                        className="w-full px-2.5 py-1.5 text-[12px] font-mono rounded-lg outline-none"
                        style={SYNC_FIELD_STYLE}
                      />
                    </label>
                    {syncE2eeDraft && (
                      <label className="col-span-6 space-y-1">
                        <span className="flex items-center justify-between text-[11px]">
                          <span style={{ color: 'var(--text-secondary)' }}>E2EE 口令</span>
                          {syncStatus?.has_e2ee_passphrase && (
                            <span style={{ color: 'var(--accent-emerald)' }}>已存入凭据库</span>
                          )}
                        </span>
                        <input
                          type="password"
                          readOnly
                          aria-label="E2EE 同步口令"
                          value={syncPassphraseInput}
                          placeholder={syncStatus?.has_e2ee_passphrase ? '••••••••' : '部署时自动生成'}
                          className="w-full px-2.5 py-1.5 text-[12px] font-mono rounded-lg outline-none"
                          style={SYNC_FIELD_STYLE}
                        />
                      </label>
                    )}
                  </div>
                )}
              </div>
            ) : (
              <div className="space-y-3">
                <div className="space-y-1">
                  <span className="text-[11px]" style={{ color: 'var(--text-secondary)' }}>
                    Worker 服务地址
                  </span>
                  <input
                    type="url"
                    aria-label="Worker 服务地址"
                    placeholder="https://tokenvane-sync.your-subdomain.workers.dev"
                    value={syncWorkerUrlDraft}
                    onChange={(e) => setSyncWorkerUrlDraft(e.target.value)}
                    className="w-full px-2.5 py-1.5 text-[12px] font-mono rounded-lg outline-none"
                    style={SYNC_FIELD_STYLE}
                  />
                </div>

                <div className="space-y-1">
                  <div className="flex items-center justify-between text-[11px]">
                    <span style={{ color: 'var(--text-secondary)' }}>访问令牌</span>
                    <span
                      style={{
                        color: syncStatus?.has_bearer_token
                          ? 'var(--accent-emerald)'
                          : 'var(--text-muted)',
                      }}
                    >
                      {syncStatus?.has_bearer_token ? '已存入凭据库' : '待录入'}
                    </span>
                  </div>
                  <div className="flex items-center gap-2">
                    <input
                      type="password"
                      aria-label="Sync Bearer Token"
                      placeholder={syncStatus?.has_bearer_token ? '输入新值可更新' : 'Worker SYNC_TOKEN'}
                      value={syncBearerTokenInput}
                      onChange={(e) => setSyncBearerTokenInput(e.target.value)}
                      className="flex-1 px-2.5 py-1.5 text-[12px] font-mono rounded-lg outline-none"
                      style={SYNC_FIELD_STYLE}
                    />
                    <button
                      type="button"
                      aria-label="随机生成访问令牌"
                      onClick={() => setSyncBearerTokenInput(generateSecretToken(24))}
                      className="btn-interactive inline-flex items-center gap-1 px-2.5 py-1.5 rounded-lg text-[11.5px] cursor-pointer whitespace-nowrap shrink-0"
                      style={{
                        backgroundColor: 'var(--bg-elevated)',
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-secondary)',
                      }}
                    >
                      <Wand2 className="w-3 h-3" />
                      <span>随机生成</span>
                    </button>
                  </div>
                </div>

                {syncE2eeDraft && (
                  <div className="space-y-1">
                    <div className="flex items-center justify-between text-[11px]">
                      <span style={{ color: 'var(--text-secondary)' }}>E2EE 同步口令</span>
                      <span
                        style={{
                          color: syncStatus?.has_e2ee_passphrase
                            ? 'var(--accent-emerald)'
                            : 'var(--text-muted)',
                        }}
                      >
                        {syncStatus?.has_e2ee_passphrase ? '已存入凭据库' : '待录入'}
                      </span>
                    </div>
                    <div className="flex items-center gap-2">
                      <input
                        type="password"
                        aria-label="E2EE 同步口令"
                        placeholder={syncStatus?.has_e2ee_passphrase ? '输入新值可更新' : '多端共享加密口令'}
                        value={syncPassphraseInput}
                        onChange={(e) => setSyncPassphraseInput(e.target.value)}
                        className="flex-1 px-2.5 py-1.5 text-[12px] font-mono rounded-lg outline-none"
                        style={SYNC_FIELD_STYLE}
                      />
                      <button
                        type="button"
                        aria-label="随机生成 E2EE 口令"
                        onClick={() => setSyncPassphraseInput(generateSecretToken(16))}
                        className="btn-interactive inline-flex items-center gap-1 px-2.5 py-1.5 rounded-lg text-[11.5px] cursor-pointer whitespace-nowrap shrink-0"
                        style={{
                          backgroundColor: 'var(--bg-elevated)',
                          border: '1px solid var(--border-subtle)',
                          color: 'var(--text-secondary)',
                        }}
                      >
                        <Wand2 className="w-3 h-3" />
                        <span>随机生成</span>
                      </button>
                    </div>
                  </div>
                )}

                <button
                  type="button"
                  onClick={() => saveCloudSyncSetupMutation.mutate()}
                  disabled={saveCloudSyncSetupMutation.isPending || provisionWorkerMutation.isPending}
                  className="btn-primary inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium cursor-pointer disabled:opacity-50 whitespace-nowrap"
                  style={{
                    backgroundColor: 'var(--accent-primary)',
                    color: '#ffffff',
                  }}
                >
                  <Save className="w-3.5 h-3.5" />
                  <span>{saveCloudSyncSetupMutation.isPending ? '保存中…' : '保存并连接'}</span>
                </button>
              </div>
            )}

            <div
              className="flex flex-wrap items-center gap-2 pt-2.5"
              style={{ borderTop: '1px solid var(--border-hairline)' }}
            >
              {syncStatus?.has_bearer_token && syncStatus?.worker_url && (
                <button
                  type="button"
                  onClick={() => {
                    if (pairingExpanded && pairingBundle) {
                      setPairingExpanded(false);
                    } else {
                      loadPairingMutation.mutate(includePassphraseInPairing);
                    }
                  }}
                  disabled={loadPairingMutation.isPending}
                  className="btn-interactive inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-[11.5px] cursor-pointer whitespace-nowrap"
                  style={{
                    backgroundColor: 'var(--bg-elevated)',
                    border: '1px solid var(--border-strong)',
                    color: 'var(--text-primary)',
                  }}
                >
                  <QrCode className="w-3.5 h-3.5" />
                  <span>{pairingExpanded && pairingBundle ? '收起配对码' : '移动端配对'}</span>
                </button>
              )}

              <button
                type="button"
                onClick={() => previewSyncMutation.mutate()}
                disabled={previewSyncMutation.isPending}
                className="btn-interactive px-2.5 py-1.5 rounded-lg text-[11.5px] cursor-pointer disabled:opacity-50 whitespace-nowrap"
                style={{
                  backgroundColor: 'var(--bg-elevated)',
                  color: 'var(--text-secondary)',
                  border: '1px solid var(--border-subtle)',
                }}
              >
                {previewSyncMutation.isPending ? '生成中…' : '同步负载预览'}
              </button>

              <button
                type="button"
                onClick={() => clearSyncCredsMutation.mutate()}
                disabled={clearSyncCredsMutation.isPending}
                className="btn-danger px-2.5 py-1.5 rounded-lg text-[11.5px] cursor-pointer disabled:opacity-50 whitespace-nowrap"
                style={{
                  backgroundColor: 'var(--bg-elevated)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--accent-rose)',
                }}
              >
                清除凭据
              </button>
            </div>

            {syncStatus?.last_sync_error && (
              <div
                className="px-3 py-2 rounded-lg text-[11.5px]"
                style={{
                  backgroundColor: 'rgba(244, 63, 94, 0.08)',
                  border: '1px solid rgba(244, 63, 94, 0.35)',
                  color: 'var(--accent-rose)',
                }}
              >
                最近同步异常：{syncStatus.last_sync_error}
              </div>
            )}
          </div>
        </div>

        {/* Mobile QR & Pairing Link */}
        {pairingExpanded && pairingBundle && (
          <div
            data-testid="mobile-pairing-panel"
            className="pb-4 grid grid-cols-12 gap-4 items-start"
            style={{ borderBottom: '1px solid var(--border-hairline)' }}
          >
            <div className="col-span-4 space-y-1">
              <div className="flex items-center gap-1.5 text-[13px] font-medium">
                <QrCode className="w-3.5 h-3.5" style={{ color: 'var(--accent-primary)' }} />
                <span>移动端扫码配对</span>
              </div>
            </div>

            <div className="col-span-8 flex flex-wrap items-start gap-5">
              <div
                className="p-2 rounded-lg shrink-0"
                style={{
                  backgroundColor: '#ffffff',
                  border: '1px solid var(--border-subtle)',
                }}
              >
                <QrCodeSvg value={pairingBundle.pairing_uri} size={140} />
              </div>

              <div className="flex-1 min-w-[240px] space-y-2.5 text-[12px]">
                <input
                  type="text"
                  readOnly
                  aria-label="移动端配对链接"
                  value={pairingBundle.pairing_uri}
                  onFocus={(e) => e.currentTarget.select()}
                  className="w-full px-2.5 py-1.5 text-[11px] font-mono rounded-lg outline-none select-all"
                  style={{
                    backgroundColor: 'var(--bg-input)',
                    border: '1px solid var(--border-subtle)',
                    color: 'var(--text-secondary)',
                  }}
                />

                <div className="flex flex-wrap items-center gap-3">
                  <button
                    type="button"
                    onClick={() => {
                      void navigator.clipboard?.writeText(pairingBundle.pairing_uri);
                      setPairingCopied(true);
                      window.setTimeout(() => setPairingCopied(false), 1800);
                    }}
                    className="btn-interactive inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-[11.5px] cursor-pointer"
                    style={{
                      backgroundColor: 'var(--bg-elevated)',
                      border: '1px solid var(--border-strong)',
                      color: 'var(--text-primary)',
                    }}
                  >
                    {pairingCopied ? (
                      <Check className="w-3.5 h-3.5" style={{ color: 'var(--accent-emerald)' }} />
                    ) : (
                      <Copy className="w-3.5 h-3.5" />
                    )}
                    <span>{pairingCopied ? '已复制链接' : '复制配对链接'}</span>
                  </button>

                  {syncStatus?.e2ee_enabled && (
                    <label className="flex items-center gap-1.5 text-[11.5px] cursor-pointer">
                      <input
                        type="checkbox"
                        checked={includePassphraseInPairing}
                        onChange={(e) => {
                          const next = e.target.checked;
                          setIncludePassphraseInPairing(next);
                          loadPairingMutation.mutate(next);
                        }}
                      />
                      <span>配对码包含口令</span>
                    </label>
                  )}
                </div>
              </div>
            </div>
          </div>
        )}

        {syncEnvelope && envelopeJson && (
          <div
            className="p-3 rounded-lg space-y-2"
            style={{
              backgroundColor: 'var(--bg-elevated)',
              border: '1px solid var(--border-subtle)',
            }}
          >
            <div
              className="flex flex-wrap items-center gap-4 text-[11px] tabular-nums"
              style={{ color: 'var(--text-secondary)' }}
            >
              <span>schema_version: {syncEnvelope.schema_version}</span>
              <span>小时聚合条数: {syncEnvelope.hourly_deltas.length}</span>
              <span>订阅快照数: {syncEnvelope.subscription_snapshots.length}</span>
              <span>负载体积: {envelopeJson.length.toLocaleString('en-US')} Bytes</span>
            </div>
            <pre
              className="m-0 max-h-64 overflow-auto text-[11px] font-mono select-text"
              style={{ color: 'var(--text-secondary)' }}
            >
              {envelopeJson}
            </pre>
          </div>
        )}
      </section>
    </div>
  );
}
