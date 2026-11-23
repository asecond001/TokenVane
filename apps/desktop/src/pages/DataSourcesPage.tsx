import { useCallback, useMemo, useState } from 'react';
import { ActionNotice } from '../components/ActionNotice';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { open } from '@tauri-apps/plugin-dialog';
import {
  Check,
  Copy,
  FlaskConical,
  FolderPlus,
  KeyRound,
  Plus,
  RefreshCw,
  ShieldCheck,
  Trash2,
} from 'lucide-react';
import {
  formatCost,
  formatExactInt,
  formatRelativeTime,
  formatTokens,
  providerAccentColor,
  providerDisplayName,
} from '../lib/format';
import { ipc, isTauriRuntime } from '../lib/ipc';
import { TRAY_SUMMARY_QUERY_KEY } from '../lib/queryKeys';
import { useWorkspaceStore } from '../stores/workspaceStore';
import type {
  AppPreferences,
  BalanceRefreshOutcome,
  CollectionMode,
  CustomProtocolKind,
  CustomProviderRule,
  CustomRuleTestResult,
  OfficialApiReconcileResult,
  ProviderBalanceSnapshot,
  ProviderConfig,
} from '../types/ipc';
import { CLOCK_REFRESH_MS } from '../lib/liveRefresh';

const DEFAULT_PATH_HINTS: Record<string, string> = {
  claude_code: '~/.claude/projects/ 与 ~/.claude/stats-cache.json',
  codex: '~/.codex/sessions/**/rollout-*.jsonl',
  antigravity: '%APPDATA% / ~/.gemini/antigravity/brain/*/transcript.jsonl',
  kimi_code: '~/.kimi/',
  workbuddy: '~/.workbuddy/ 或 %APPDATA%/Workbuddy/',
  deepseek_harness: '~/.dsh/ 或 ~/.deepseek-harness/',
  qcode: '~/.qcode/',
  minimax_code: '~/.minimax/',
};

const BALANCE_API_LABELS: Record<string, { label: string; placeholder: string }> = {
  kimi_code: {
    label: 'Kimi Code / Moonshot 官方 Key',
    placeholder: '粘贴 sk-kimi-... (Coding 会员实时配额) 或 sk-... (Moonshot 余额)',
  },
  deepseek_harness: {
    label: 'DeepSeek 官方 API Key',
    placeholder: '粘贴 sk-... 查询 DeepSeek 余额与赠金',
  },
  qcode: {
    label: 'QCode 只读 OpenAPI Token / API Key',
    placeholder: '粘贴 qot_... 或 cr_... 查询用量限额',
  },
  minimax_code: {
    label: 'MiniMax 官方 API Key',
    placeholder: '粘贴 MiniMax Key 查询 Token Plan 余量',
  },
};

const PROTOCOL_KINDS: Array<{ id: CustomProtocolKind; label: string }> = [
  { id: 'openai_jsonl', label: 'OpenAI JSONL' },
  { id: 'anthropic_jsonl', label: 'Anthropic JSONL' },
  { id: 'declarative_jsonl', label: 'JSON Pointer 映射' },
];

function createEmptyCustomRule(): CustomProviderRule {
  return {
    rule_id: '',
    display_name: '',
    enabled: true,
    protocol_kind: 'openai_jsonl',
    log_roots: [],
    mapping_spec: {
      provider_id_override: null,
      file_extensions: ['jsonl', 'json'],
      timestamp_path: '/timestamp',
      model_path: '/model',
      session_id_path: '/session_id',
      request_id_path: '/request_id',
      input_tokens_path: '/usage/input_tokens',
      output_tokens_path: '/usage/output_tokens',
      cache_read_tokens_path: null,
      cache_write_tokens_path: null,
      reasoning_tokens_path: null,
      duration_ms_path: null,
      ttft_ms_path: null,
    },
    updated_at_ms: 0,
  };
}

function formatNativeBalance(micro: number | null | undefined, currency: string): string {
  if (micro === null || micro === undefined) {
    return '—';
  }
  const code = currency?.trim() || 'USD';
  return `${code} ${(micro / 1_000_000).toFixed(2)}`;
}

const COLLECTION_MODES: Array<{ id: CollectionMode; label: string }> = [
  { id: 'local_log', label: '本地日志' },
  { id: 'otlp', label: '本地 OTLP' },
  { id: 'official_api', label: '官方 Usage API' },
];

export function DataSourcesPage() {
  const queryClient = useQueryClient();

  const { data: bootstrap } = useQuery({
    queryKey: ['bootstrap'],
    queryFn: () => ipc.getBootstrapState(),
    refetchInterval: CLOCK_REFRESH_MS,
  });

  const { data: providers = [] } = useQuery({
    queryKey: ['provider_configs'],
    queryFn: () => ipc.getProviderConfigs(),
  });

  const { data: cursors = [] } = useQuery({
    queryKey: ['ingestion_cursors'],
    queryFn: () => ipc.getIngestionCursors(),
  });

  const { data: providerBalances = [] } = useQuery({
    queryKey: ['provider-balances'],
    queryFn: () => ipc.getProviderBalances(),
  });

  const { data: customRules = [] } = useQuery({
    queryKey: ['custom-provider-rules'],
    queryFn: () => ipc.listCustomProviderRules(),
  });

  const { currency, usdToCnyRate } = useWorkspaceStore();

  const [pathDrafts, setPathDrafts] = useState<Record<string, string>>({});
  const [apiKeyDrafts, setApiKeyDrafts] = useState<Record<string, string>>({});
  const [reconcileResults, setReconcileResults] = useState<
    Record<string, OfficialApiReconcileResult>
  >({});
  const [balanceOutcomes, setBalanceOutcomes] = useState<
    Record<string, BalanceRefreshOutcome>
  >({});
  const [otlpPortDraft, setOtlpPortDraft] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const clearNotice = useCallback(() => setNotice(null), []);
  const clearErrorMsg = useCallback(() => setErrorMsg(null), []);
  const [copiedSnippet, setCopiedSnippet] = useState(false);

  const [ruleDraft, setRuleDraft] = useState<CustomProviderRule>(() => createEmptyCustomRule());
  const [rootsTextDraft, setRootsTextDraft] = useState('');
  const [extTextDraft, setExtTextDraft] = useState('jsonl, json');
  const [sampleLineDraft, setSampleLineDraft] = useState(
    '{"timestamp":"2026-10-16T10:00:00Z","model":"deepseek-chat","usage":{"prompt_tokens":1200,"completion_tokens":350,"prompt_tokens_details":{"cached_tokens":400}}}'
  );
  const [sampleTestResult, setSampleTestResult] = useState<CustomRuleTestResult | null>(null);

  const balanceByProvider = useMemo(() => {
    const map = new Map<string, ProviderBalanceSnapshot>();
    for (const snap of providerBalances) {
      map.set(snap.provider_id, snap);
    }
    for (const [pid, outcome] of Object.entries(balanceOutcomes)) {
      map.set(pid, outcome.snapshot);
    }
    return map;
  }, [providerBalances, balanceOutcomes]);

  const activeOtlpPort =
    otlpPortDraft ?? String(bootstrap?.preferences.otlp_port ?? 4318);

  const otlpEnvSnippet = [
    'CLAUDE_CODE_ENABLE_TELEMETRY=1',
    `OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:${activeOtlpPort}`,
    'OTEL_EXPORTER_OTLP_PROTOCOL=http/json',
    'OTEL_METRICS_EXPORTER=otlp',
    'OTEL_LOGS_EXPORTER=otlp',
  ].join('\n');

  const invalidateAllSources = async () => {
    await queryClient.invalidateQueries({ queryKey: ['bootstrap'] });
    await queryClient.invalidateQueries({ queryKey: ['provider_configs'] });
    await queryClient.invalidateQueries({ queryKey: ['provider-balances'] });
    await queryClient.invalidateQueries({ queryKey: ['custom-provider-rules'] });
    await queryClient.invalidateQueries({ queryKey: ['ingestion_cursors'] });
    await queryClient.invalidateQueries({ queryKey: ['overview'] });
    await queryClient.invalidateQueries({ queryKey: ['subscriptions'] });
    await queryClient.invalidateQueries({ queryKey: ['observed_rate_limits'] });
    await queryClient.invalidateQueries({ queryKey: [TRAY_SUMMARY_QUERY_KEY] });
  };

  const updateProviderMutation = useMutation({
    mutationFn: (cfg: ProviderConfig) => ipc.updateProviderConfig(cfg),
    onSuccess: async () => {
      setErrorMsg(null);
      await invalidateAllSources();
    },
    onError: (err: unknown) => {
      setErrorMsg(err instanceof Error ? err.message : '更新数据源配置失败');
    },
  });

  const scanMutation = useMutation({
    mutationFn: () => ipc.triggerScanNow(),
    onSuccess: async (report) => {
      setErrorMsg(null);
      setNotice(
        `扫描完成：${report.files_scanned} 个文件，新增 ${report.events_inserted} 条，跳过 ${report.events_deduplicated} 条`
      );
      await invalidateAllSources();
    },
    onError: (err: unknown) => {
      setErrorMsg(err instanceof Error ? err.message : '执行增量扫描失败');
    },
  });

  const saveApiKeyMutation = useMutation({
    mutationFn: async ({ providerId, apiKey }: { providerId: string; apiKey: string }) => {
      if (!apiKey.trim()) {
        throw new Error('请输入有效的 API Key / Admin Key');
      }
      await ipc.setProviderApiKey(providerId, apiKey.trim());
    },
    onSuccess: async (_, vars) => {
      setApiKeyDrafts((prev) => ({ ...prev, [vars.providerId]: '' }));
      setErrorMsg(null);
      setNotice(`${providerDisplayName(vars.providerId)} 密钥已存入凭据库`);
      await invalidateAllSources();
    },
    onError: (err: unknown) => {
      setErrorMsg(err instanceof Error ? err.message : '保存系统凭据失败');
    },
  });

  const deleteApiKeyMutation = useMutation({
    mutationFn: (providerId: string) => ipc.deleteProviderApiKey(providerId),
    onSuccess: async (_, providerId) => {
      setErrorMsg(null);
      setNotice(`已移除 ${providerDisplayName(providerId)} 密钥`);
      await invalidateAllSources();
    },
    onError: (err: unknown) => {
      setErrorMsg(err instanceof Error ? err.message : '删除系统凭据失败');
    },
  });

  const reconcileMutation = useMutation({
    mutationFn: (providerId: string) => ipc.reconcileProviderOfficialApi(providerId, 7),
    onSuccess: (res) => {
      setErrorMsg(null);
      setReconcileResults((prev) => ({ ...prev, [res.provider_id]: res }));
      setNotice(
        `${providerDisplayName(res.provider_id)} 对账成功`
      );
    },
    onError: (err: unknown) => {
      setErrorMsg(err instanceof Error ? err.message : '调用官方 Usage API 对账失败');
    },
  });

  const refreshBalanceMutation = useMutation({
    mutationFn: (providerId: string) => ipc.refreshProviderBalance(providerId),
    onSuccess: async (outcome) => {
      setErrorMsg(null);
      setBalanceOutcomes((prev) => ({
        ...prev,
        [outcome.snapshot.provider_id]: outcome,
      }));
      setNotice(
        outcome.from_cache
          ? `${providerDisplayName(outcome.snapshot.provider_id)} 冷却中 ${outcome.cooldown_remaining_secs}s`
          : `${providerDisplayName(outcome.snapshot.provider_id)} 余额已刷新`
      );
      await invalidateAllSources();
    },
    onError: (err: unknown) => {
      setErrorMsg(err instanceof Error ? err.message : '刷新官方余额接口失败');
    },
  });

  const buildNormalizedDraftRule = (sourceRule: CustomProviderRule): CustomProviderRule => {
    const roots = rootsTextDraft
      .split(/\r?\n|,/)
      .map((s) => s.trim())
      .filter(Boolean);
    const exts = extTextDraft
      .split(',')
      .map((s) => s.trim().replace(/^\./, ''))
      .filter(Boolean);
    return {
      ...sourceRule,
      rule_id: sourceRule.rule_id.trim(),
      display_name: sourceRule.display_name.trim(),
      log_roots: roots,
      mapping_spec: {
        ...sourceRule.mapping_spec,
        provider_id_override: sourceRule.mapping_spec.provider_id_override?.trim() || null,
        file_extensions: exts.length > 0 ? exts : ['jsonl'],
      },
      updated_at_ms: Date.now(),
    };
  };

  const upsertCustomRuleMutation = useMutation({
    mutationFn: (rule: CustomProviderRule) => ipc.upsertCustomProviderRule(rule),
    onSuccess: async (_, saved) => {
      setErrorMsg(null);
      setNotice(`已保存自定义规则「${saved.display_name}」`);
      await invalidateAllSources();
    },
    onError: (err: unknown) => {
      setErrorMsg(err instanceof Error ? err.message : '保存自定义接入规则失败');
    },
  });

  const deleteCustomRuleMutation = useMutation({
    mutationFn: (ruleId: string) => ipc.deleteCustomProviderRule(ruleId),
    onSuccess: async (_, ruleId) => {
      setErrorMsg(null);
      setNotice(`已移除自定义规则 ${ruleId}`);
      await invalidateAllSources();
    },
    onError: (err: unknown) => {
      setErrorMsg(err instanceof Error ? err.message : '删除自定义规则失败');
    },
  });

  const testCustomRuleMutation = useMutation({
    mutationFn: ({ rule, sampleLine }: { rule: CustomProviderRule; sampleLine: string }) =>
      ipc.testCustomProviderSampleLine(rule, sampleLine),
    onSuccess: (res) => {
      setErrorMsg(null);
      setSampleTestResult(res);
    },
    onError: (err: unknown) => {
      setErrorMsg(err instanceof Error ? err.message : '样本行试跑验证失败');
    },
  });

  const updateOtlpPrefsMutation = useMutation({
    mutationFn: async (nextPrefs: AppPreferences) => ipc.updatePreferences(nextPrefs),
    onSuccess: async (saved) => {
      setErrorMsg(null);
      setOtlpPortDraft(String(saved.otlp_port));
      setNotice(
        saved.otlp_enabled
          ? `本地 OTLP 接收器已监听 127.0.0.1:${saved.otlp_port}`
          : '已关闭本地 OTLP 接收器'
      );
      await queryClient.invalidateQueries({ queryKey: ['bootstrap'] });
      await queryClient.invalidateQueries({ queryKey: ['preferences'] });
    },
    onError: (err: unknown) => {
      setErrorMsg(err instanceof Error ? err.message : '更新本地 OTLP 配置失败');
    },
  });

  const handleAddCustomPath = (cfg: ProviderConfig, rawPath: string) => {
    const trimmed = rawPath.trim();
    if (!trimmed) return;
    if (cfg.custom_log_paths.includes(trimmed)) {
      setPathDrafts((prev) => ({ ...prev, [cfg.provider_id]: '' }));
      return;
    }
    updateProviderMutation.mutate({
      ...cfg,
      custom_log_paths: [...cfg.custom_log_paths, trimmed],
    });
    setPathDrafts((prev) => ({ ...prev, [cfg.provider_id]: '' }));
  };

  const handleBrowseFolder = async (cfg: ProviderConfig) => {
    if (!isTauriRuntime()) {
      setNotice('可在输入框手动填写路径');
      return;
    }
    const selected = await open({
      directory: true,
      multiple: false,
      title: `选择 ${providerDisplayName(cfg.provider_id)} 日志目录`,
    });
    if (typeof selected === 'string' && selected.trim()) {
      handleAddCustomPath(cfg, selected);
    }
  };

  const handleRemoveCustomPath = (cfg: ProviderConfig, target: string) => {
    updateProviderMutation.mutate({
      ...cfg,
      custom_log_paths: cfg.custom_log_paths.filter((p) => p !== target),
    });
  };

  const handleCopyOtlpSnippet = async () => {
    try {
      await navigator.clipboard.writeText(otlpEnvSnippet);
      setCopiedSnippet(true);
      setTimeout(() => setCopiedSnippet(false), 1800);
    } catch {
      setNotice('请手动复制环境变量文本');
    }
  };

  const handleToggleOtlp = (enabled: boolean) => {
    const basePrefs = bootstrap?.preferences;
    if (!basePrefs) return;
    const parsedPort = Number.parseInt(activeOtlpPort, 10);
    const safePort =
      Number.isFinite(parsedPort) && parsedPort >= 1024 && parsedPort <= 65535
        ? parsedPort
        : 4318;
    updateOtlpPrefsMutation.mutate({
      ...basePrefs,
      otlp_enabled: enabled,
      otlp_port: safePort,
    });
  };

  return (
    <div className="space-y-6">
      {/* Header & Compliance Declaration Strip */}
      <section
        className="flex flex-wrap items-center justify-between gap-4 pb-5"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <div className="flex items-center gap-2">
          <ShieldCheck className="w-4 h-4" style={{ color: 'var(--accent-emerald)' }} />
          <h1 className="text-[16px] font-semibold tracking-tight">
            数据源与合规
          </h1>
        </div>

        <button
          type="button"
          onClick={() => scanMutation.mutate()}
          disabled={scanMutation.isPending}
          className="btn-primary flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium cursor-pointer disabled:opacity-50"
          style={{
            backgroundColor: 'var(--accent-primary)',
            color: '#ffffff',
          }}
        >
          <RefreshCw
            className={`w-3.5 h-3.5 ${scanMutation.isPending ? 'animate-spin' : ''}`}
          />
          <span>{scanMutation.isPending ? '扫描中…' : '立即增量扫描'}</span>
        </button>
      </section>

      <ActionNotice text={notice} kind="success" onClose={clearNotice} />
      <ActionNotice text={errorMsg} kind="error" onClose={clearErrorMsg} />

      {/* Section 1: Provider Data Sources */}
      <section className="space-y-2">
        <h2
          className="text-[11px] font-semibold uppercase tracking-wider"
          style={{ color: 'var(--text-muted)' }}
        >
          采集适配器
        </h2>

        <div style={{ borderTop: '1px solid var(--border-subtle)' }}>
          {providers.map((cfg) => {
            const supportsOrgUsageApi =
              cfg.provider_id === 'claude_code' || cfg.provider_id === 'codex';
            const balanceApiMeta = BALANCE_API_LABELS[cfg.provider_id];
            const supportsAnyApi = supportsOrgUsageApi || Boolean(balanceApiMeta);
            const reconcile = reconcileResults[cfg.provider_id];
            const balanceSnap = balanceByProvider.get(cfg.provider_id);
            const balanceOutcome = balanceOutcomes[cfg.provider_id];

            return (
              <div
                key={cfg.provider_id}
                className="py-5 space-y-3.5"
                style={{ borderBottom: '1px solid var(--border-subtle)' }}
              >
                <div className="flex flex-wrap items-center justify-between gap-4">
                  <div className="flex flex-wrap items-center gap-2.5">
                    <span
                      className="inline-block w-2 h-2 rounded-full"
                      style={{ backgroundColor: providerAccentColor(cfg.provider_id) }}
                    />
                    <span className="text-[14px] font-semibold">
                      {providerDisplayName(cfg.provider_id)}
                    </span>
                    <span
                      className="text-[11px] px-1.5 py-0.5 rounded-md tabular-nums"
                      style={{
                        backgroundColor: 'var(--bg-hover)',
                        border: '1px solid var(--border-hairline)',
                        color: cfg.enabled ? 'var(--accent-emerald)' : 'var(--text-muted)',
                      }}
                    >
                      {cfg.enabled ? '已启用' : '已暂停'}
                    </span>
                    <span
                      className="text-[11.5px] tabular-nums"
                      style={{ color: 'var(--text-muted)' }}
                    >
                      最近同步：{formatRelativeTime(cfg.last_sync_at_ms)}
                    </span>
                  </div>

                  <div className="flex items-center gap-3">
                    <select
                      aria-label={`${cfg.provider_id} 采集模式`}
                      value={cfg.collection_mode}
                      onChange={(e) =>
                        updateProviderMutation.mutate({
                          ...cfg,
                          collection_mode: e.target.value as CollectionMode,
                        })
                      }
                      className="px-2 py-1 text-[11.5px] rounded-lg outline-none cursor-pointer"
                      style={{
                        backgroundColor: 'var(--bg-input)',
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-primary)',
                      }}
                    >
                      {COLLECTION_MODES.map((m) => (
                        <option key={m.id} value={m.id}>
                          {m.label}
                        </option>
                      ))}
                    </select>

                    <label className="flex items-center gap-1.5 text-[12px] cursor-pointer">
                      <input
                        type="checkbox"
                        checked={cfg.enabled}
                        onChange={(e) =>
                          updateProviderMutation.mutate({
                            ...cfg,
                            enabled: e.target.checked,
                          })
                        }
                      />
                      <span>启用采集</span>
                    </label>
                  </div>
                </div>

                {/* Default Path & Custom Log Directories */}
                <div className="pl-4.5 space-y-2">
                  <div className="text-[11.5px]" style={{ color: 'var(--text-secondary)' }}>
                    默认路径
                    <code className="ml-1 text-[11px] select-text">
                      {DEFAULT_PATH_HINTS[cfg.provider_id] ?? '标准用户数据目录'}
                    </code>
                  </div>

                  {cfg.custom_log_paths.length > 0 && (
                    <div className="space-y-1">
                      {cfg.custom_log_paths.map((customPath) => (
                        <div
                          key={customPath}
                          className="flex items-center justify-between gap-2 px-2.5 py-1 rounded-lg text-[11.5px]"
                          style={{
                            backgroundColor: 'var(--bg-elevated)',
                            border: '1px solid var(--border-subtle)',
                          }}
                        >
                          <code className="select-text truncate">{customPath}</code>
                          <button
                            type="button"
                            onClick={() => handleRemoveCustomPath(cfg, customPath)}
                            className="btn-danger inline-flex items-center gap-1 px-1.5 py-0.5 rounded-lg text-[11px] shrink-0 cursor-pointer"
                            style={{ color: 'var(--accent-rose)' }}
                          >
                            <Trash2 className="w-3 h-3" />
                            <span>移除</span>
                          </button>
                        </div>
                      ))}
                    </div>
                  )}

                  <div className="flex flex-wrap items-center gap-2">
                    <input
                      type="text"
                      placeholder="添加日志目录或文件"
                      value={pathDrafts[cfg.provider_id] ?? ''}
                      onChange={(e) =>
                        setPathDrafts((prev) => ({
                          ...prev,
                          [cfg.provider_id]: e.target.value,
                        }))
                      }
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') {
                          e.preventDefault();
                          handleAddCustomPath(cfg, pathDrafts[cfg.provider_id] ?? '');
                        }
                      }}
                      className="flex-1 min-w-[260px] px-2.5 py-1 text-[11.5px] rounded-lg outline-none"
                      style={{
                        backgroundColor: 'var(--bg-input)',
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-primary)',
                      }}
                    />
                    <button
                      type="button"
                      onClick={() =>
                        handleAddCustomPath(cfg, pathDrafts[cfg.provider_id] ?? '')
                      }
                      className="btn-interactive bg-[var(--bg-elevated)] px-2.5 py-1 text-[11.5px] rounded-lg cursor-pointer"
                      style={{
                        color: 'var(--text-primary)',
                        border: '1px solid var(--border-subtle)',
                      }}
                    >
                      添加路径
                    </button>
                    <button
                      type="button"
                      onClick={() => handleBrowseFolder(cfg)}
                      className="btn-interactive bg-[var(--bg-elevated)] inline-flex items-center gap-1 px-2.5 py-1 text-[11.5px] rounded-lg cursor-pointer"
                      style={{
                        color: 'var(--text-primary)',
                        border: '1px solid var(--border-subtle)',
                      }}
                    >
                      <FolderPlus className="w-3.5 h-3.5" />
                      <span>浏览目录</span>
                    </button>
                  </div>
                </div>

                {/* Official Usage / Balance API Key & Refresh */}
                {supportsAnyApi && (
                  <div
                    className="pl-4.5 pt-2 flex flex-wrap items-center justify-between gap-3 text-[11.5px]"
                    style={{ borderTop: '1px dashed var(--border-subtle)' }}
                  >
                    <div className="flex items-center gap-2">
                      <KeyRound className="w-3.5 h-3.5" style={{ color: 'var(--text-muted)' }} />
                      <span style={{ color: 'var(--text-secondary)' }}>
                        {supportsOrgUsageApi
                          ? cfg.provider_id === 'claude_code'
                            ? 'Anthropic Admin API Key'
                            : 'OpenAI Admin / Usage API Key'
                          : balanceApiMeta?.label}
                      </span>
                      <span
                        className="px-1.5 py-0.5 rounded-md text-[10.5px]"
                        style={{
                          backgroundColor: 'var(--bg-hover)',
                          border: '1px solid var(--border-hairline)',
                          color: cfg.has_credential
                            ? 'var(--accent-emerald)'
                            : 'var(--text-muted)',
                        }}
                      >
                        {cfg.has_credential
                          ? '已存入凭据库'
                          : '可录入密钥'}
                      </span>
                    </div>

                    <div className="flex flex-wrap items-center gap-2">
                      <input
                        type="password"
                        placeholder={
                          cfg.has_credential
                            ? '输入新值可替换'
                            : supportsOrgUsageApi
                              ? 'Admin API Key'
                              : balanceApiMeta?.placeholder
                        }
                        value={apiKeyDrafts[cfg.provider_id] ?? ''}
                        onChange={(e) =>
                          setApiKeyDrafts((prev) => ({
                            ...prev,
                            [cfg.provider_id]: e.target.value,
                          }))
                        }
                        className="w-56 px-2 py-1 text-[11.5px] rounded-lg outline-none"
                        style={{
                          backgroundColor: 'var(--bg-input)',
                          border: '1px solid var(--border-subtle)',
                          color: 'var(--text-primary)',
                        }}
                      />
                      <button
                        type="button"
                        onClick={() =>
                          saveApiKeyMutation.mutate({
                            providerId: cfg.provider_id,
                            apiKey: apiKeyDrafts[cfg.provider_id] ?? '',
                          })
                        }
                        disabled={saveApiKeyMutation.isPending}
                        className="btn-interactive bg-[var(--bg-elevated)] px-2.5 py-1 rounded-lg cursor-pointer disabled:opacity-50"
                        style={{
                          color: 'var(--text-primary)',
                          border: '1px solid var(--border-subtle)',
                        }}
                      >
                        存入凭据库
                      </button>
                      {cfg.has_credential && (
                        <>
                          {supportsOrgUsageApi ? (
                            <button
                              type="button"
                              onClick={() => reconcileMutation.mutate(cfg.provider_id)}
                              disabled={reconcileMutation.isPending}
                              className="btn-primary px-2.5 py-1 rounded-lg cursor-pointer disabled:opacity-50"
                              style={{
                                backgroundColor: 'var(--accent-primary)',
                                color: '#ffffff',
                              }}
                            >
                              对账 7d
                            </button>
                          ) : (
                            <button
                              type="button"
                              onClick={() => refreshBalanceMutation.mutate(cfg.provider_id)}
                              disabled={refreshBalanceMutation.isPending}
                              className="btn-primary px-2.5 py-1 rounded-lg cursor-pointer disabled:opacity-50"
                              style={{
                                backgroundColor: 'var(--accent-primary)',
                                color: '#ffffff',
                              }}
                            >
                              刷新余额
                            </button>
                          )}
                          <button
                            type="button"
                            disabled={deleteApiKeyMutation.isPending}
                            onClick={() => deleteApiKeyMutation.mutate(cfg.provider_id)}
                            className="btn-danger px-2 py-1 rounded-lg cursor-pointer disabled:opacity-50"
                            style={{ color: 'var(--accent-rose)' }}
                          >
                            清除密钥
                          </button>
                        </>
                      )}
                    </div>

                    {reconcile && (
                      <div
                        className="w-full px-2.5 py-1.5 rounded-lg text-[11px] tabular-nums flex flex-wrap items-center gap-4"
                        style={{
                          backgroundColor: 'var(--bg-elevated)',
                          border: '1px solid var(--border-hairline)',
                          color: 'var(--text-secondary)',
                        }}
                      >
                        <span>端点: {reconcile.endpoint}</span>
                        <span>时间桶: {reconcile.buckets_received}</span>
                        <span>输入: {formatTokens(reconcile.total_input_tokens)}</span>
                        <span>输出: {formatTokens(reconcile.total_output_tokens)}</span>
                        {reconcile.reported_cost_usd !== null && (
                          <span>官方账单: {reconcile.reported_cost_usd.toFixed(4)} USD</span>
                        )}
                      </div>
                    )}

                    {balanceSnap && (
                      <div
                        className="w-full px-2.5 py-1.5 rounded-lg text-[11px] tabular-nums flex flex-wrap items-center gap-4"
                        style={{
                          backgroundColor: 'var(--bg-elevated)',
                          border: '1px solid var(--border-hairline)',
                          color: 'var(--text-secondary)',
                        }}
                      >
                        {balanceSnap.currency === 'QUOTA' ? (
                          <>
                            <span>
                              官方接口:{' '}
                              <strong style={{ color: 'var(--text-primary)' }}>
                                Kimi Code 实时配额已同步至「用量」
                              </strong>
                            </span>
                            {balanceSnap.daily_quota_limit_micro !== null && (
                              <span>
                                周期配额: {formatExactInt(balanceSnap.daily_quota_used_micro ?? 0)}{' '}
                                / {formatExactInt(balanceSnap.daily_quota_limit_micro)}
                              </span>
                            )}
                          </>
                        ) : (
                          <>
                            <span>
                              可用余额:{' '}
                              <strong style={{ color: 'var(--text-primary)' }}>
                                {formatNativeBalance(
                                  balanceSnap.total_balance_micro,
                                  balanceSnap.currency
                                )}
                              </strong>
                            </span>
                            {balanceSnap.granted_balance_micro !== null && (
                              <span>
                                赠金/代金券:{' '}
                                {formatNativeBalance(
                                  balanceSnap.granted_balance_micro,
                                  balanceSnap.currency
                                )}
                              </span>
                            )}
                            {balanceSnap.topped_up_balance_micro !== null && (
                              <span>
                                现金/充值:{' '}
                                {formatNativeBalance(
                                  balanceSnap.topped_up_balance_micro,
                                  balanceSnap.currency
                                )}
                              </span>
                            )}
                            {balanceSnap.daily_quota_limit_micro !== null && (
                              <span>
                                今日限额:{' '}
                                {formatNativeBalance(
                                  balanceSnap.daily_quota_used_micro ?? 0,
                                  balanceSnap.currency
                                )}{' '}
                                /{' '}
                                {formatNativeBalance(
                                  balanceSnap.daily_quota_limit_micro,
                                  balanceSnap.currency
                                )}
                              </span>
                            )}
                          </>
                        )}
                        <span style={{ color: 'var(--text-muted)' }}>
                          更新于 {formatRelativeTime(balanceSnap.fetched_at_ms)}
                        </span>
                        {balanceOutcome?.from_cache && (
                          <span style={{ color: 'var(--accent-amber)' }}>
                            冷却 {balanceOutcome.cooldown_remaining_secs}s
                          </span>
                        )}
                      </div>
                    )}
                  </div>
                )}

                {cfg.provider_id === 'antigravity' && (
                  <div
                    className="pl-4.5 pt-2 flex flex-wrap items-center justify-between gap-3 text-[11.5px]"
                    style={{ borderTop: '1px dashed var(--border-subtle)' }}
                  >
                    <div className="flex items-center gap-2">
                      <RefreshCw className="w-3.5 h-3.5" style={{ color: 'var(--text-muted)' }} />
                      <span style={{ color: 'var(--text-secondary)' }}>
                        Antigravity 官方 CLI 配额
                      </span>
                      <span
                        className="px-1.5 py-0.5 rounded-md text-[10.5px]"
                        style={{
                          backgroundColor: 'var(--bg-hover)',
                          border: '1px solid var(--border-hairline)',
                          color: 'var(--accent-emerald)',
                        }}
                      >
                        agy CLI 授权
                      </span>
                      <span
                        className="px-1.5 py-0.5 rounded-md text-[10.5px]"
                        style={{
                          backgroundColor: 'var(--bg-hover)',
                          border: '1px solid var(--border-hairline)',
                          color: (bootstrap?.preferences.background_cli_quota_poll ?? true)
                            ? 'var(--accent-emerald)'
                            : 'var(--text-muted)',
                        }}
                      >
                        {(bootstrap?.preferences.background_cli_quota_poll ?? true)
                          ? '后台静默轮询'
                          : '手动按需刷新'}
                      </span>
                    </div>
                    <button
                      type="button"
                      onClick={() => refreshBalanceMutation.mutate(cfg.provider_id)}
                      disabled={refreshBalanceMutation.isPending}
                      className="btn-primary inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg cursor-pointer disabled:opacity-50"
                      style={{
                        backgroundColor: 'var(--accent-primary)',
                        color: '#ffffff',
                      }}
                    >
                      <RefreshCw className="w-3 h-3" />
                      <span>刷新 CLI 配额</span>
                    </button>

                    {balanceSnap && (
                      <div
                        className="w-full px-2.5 py-1.5 rounded-lg text-[11px] tabular-nums flex flex-wrap items-center gap-4"
                        style={{
                          backgroundColor: 'var(--bg-elevated)',
                          border: '1px solid var(--border-hairline)',
                          color: 'var(--text-secondary)',
                        }}
                      >
                        <span>
                          官方 CLI:{' '}
                          <strong style={{ color: 'var(--text-primary)' }}>
                            Antigravity 官方 CLI 配额已同步至「用量」
                          </strong>
                        </span>
                        <span style={{ color: 'var(--text-muted)' }}>
                          更新于 {formatRelativeTime(balanceSnap.fetched_at_ms)}
                        </span>
                        {balanceOutcome?.from_cache && (
                          <span style={{ color: 'var(--accent-amber)' }}>
                            冷却 {balanceOutcome.cooldown_remaining_secs}s
                          </span>
                        )}
                      </div>
                    )}
                  </div>
                )}

                {cfg.last_error && (
                  <div className="pl-4.5 text-[11.5px]" style={{ color: 'var(--accent-rose)' }}>
                    最近采集告警：{cfg.last_error}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </section>

      {/* Section 2: Loopback-Only OTLP Receiver */}
      <section
        className="py-5 space-y-3"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div className="space-y-0.5">
            <div className="flex items-center gap-2">
              <h2 className="text-[14px] font-semibold">
                本地 OTLP 接收器
              </h2>
              <span
                className="text-[11px] px-1.5 py-0.5 rounded-md tabular-nums"
                style={{
                  backgroundColor: 'var(--bg-hover)',
                  border: '1px solid var(--border-hairline)',
                  color: bootstrap?.otlp_running
                    ? 'var(--accent-emerald)'
                    : 'var(--text-muted)',
                }}
              >
                {bootstrap?.otlp_running
                  ? `运行中 · 127.0.0.1:${bootstrap.otlp_port}`
                  : '待机'}
              </span>
            </div>
          </div>

          <div className="flex items-center gap-2.5">
            <label className="flex items-center gap-1.5 text-[12px] tabular-nums">
              <span style={{ color: 'var(--text-secondary)' }}>端口</span>
              <input
                type="number"
                min={1024}
                max={65535}
                value={activeOtlpPort}
                onChange={(e) => setOtlpPortDraft(e.target.value)}
                className="w-20 px-2 py-1 text-[12px] rounded-lg outline-none tabular-nums"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--text-primary)',
                }}
              />
            </label>
            {bootstrap?.preferences.otlp_enabled ? (
              <button
                type="button"
                onClick={() => handleToggleOtlp(false)}
                disabled={updateOtlpPrefsMutation.isPending}
                className="btn-danger bg-[var(--bg-elevated)] px-3 py-1 rounded-lg text-[12px] font-medium cursor-pointer disabled:opacity-50"
                style={{
                  color: 'var(--accent-rose)',
                  border: '1px solid var(--border-subtle)',
                }}
              >
                停止接收
              </button>
            ) : (
              <button
                type="button"
                onClick={() => handleToggleOtlp(true)}
                disabled={updateOtlpPrefsMutation.isPending}
                className="btn-primary px-3 py-1 rounded-lg text-[12px] font-medium cursor-pointer disabled:opacity-50"
                style={{
                  backgroundColor: 'var(--accent-primary)',
                  color: '#ffffff',
                }}
              >
                开启接收
              </button>
            )}
          </div>
        </div>

        <div
          className="p-3 rounded-lg flex items-start justify-between gap-4"
          style={{
            backgroundColor: 'var(--bg-elevated)',
            border: '1px solid var(--border-subtle)',
          }}
        >
          <pre
            className="m-0 text-[11.5px] font-mono select-text whitespace-pre-wrap"
            style={{ color: 'var(--text-secondary)' }}
          >
            {otlpEnvSnippet}
          </pre>
          <button
            type="button"
            onClick={handleCopyOtlpSnippet}
            className="btn-interactive bg-[var(--bg-canvas)] inline-flex items-center gap-1 px-2 py-1 text-[11px] rounded-lg shrink-0 cursor-pointer"
            style={{
              color: 'var(--text-primary)',
              border: '1px solid var(--border-subtle)',
            }}
          >
            {copiedSnippet ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />}
            <span>{copiedSnippet ? '已复制' : '复制配置'}</span>
          </button>
        </div>
      </section>

      {/* Section 3: Custom Compatible & Declarative JSONL Rules */}
      <section
        className="pt-5 space-y-4"
        style={{ borderTop: '1px solid var(--border-subtle)' }}
      >
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div>
            <h2
              className="text-[12px] font-semibold uppercase tracking-wider"
              style={{ color: 'var(--text-muted)' }}
            >
              自定义接入规则
            </h2>
          </div>
          <button
            type="button"
            onClick={() => {
              setRuleDraft(createEmptyCustomRule());
              setRootsTextDraft('');
              setExtTextDraft('jsonl, json');
              setSampleTestResult(null);
            }}
            className="btn-interactive bg-[var(--bg-elevated)] inline-flex items-center gap-1 px-2.5 py-1 text-[11.5px] rounded-lg cursor-pointer"
            style={{
              color: 'var(--text-primary)',
              border: '1px solid var(--border-subtle)',
            }}
          >
            <Plus className="w-3.5 h-3.5" />
            <span>重置新建表单</span>
          </button>
        </div>

        <div style={{ borderTop: '1px solid var(--border-subtle)' }}>
          {customRules.length === 0 ? (
            <div
              className="py-4 text-[11.5px]"
              style={{
                borderBottom: '1px solid var(--border-hairline)',
                color: 'var(--text-muted)',
              }}
            >
              可添加规则
            </div>
          ) : (
            customRules.map((rule) => {
              const targetProviderId =
                rule.mapping_spec.provider_id_override?.trim() ||
                (rule.rule_id.startsWith('custom_')
                  ? rule.rule_id
                  : `custom_${rule.rule_id}`);
              return (
                <div
                  key={rule.rule_id}
                  className="py-3 flex flex-wrap items-center justify-between gap-3 text-[12px]"
                  style={{ borderBottom: '1px solid var(--border-hairline)' }}
                >
                  <div className="space-y-0.5 min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <span
                        className="inline-block w-2 h-2 rounded-full"
                        style={{ backgroundColor: providerAccentColor(targetProviderId) }}
                      />
                      <span className="font-medium">{rule.display_name}</span>
                      <code className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                        {rule.rule_id} → {targetProviderId}
                      </code>
                      <span
                        className="px-1.5 py-0.5 rounded-md text-[10.5px]"
                        style={{
                          backgroundColor: 'var(--bg-hover)',
                          border: '1px solid var(--border-hairline)',
                          color: 'var(--text-secondary)',
                        }}
                      >
                        {rule.protocol_kind}
                      </span>
                    </div>
                    <div
                      className="text-[11px] font-mono truncate"
                      style={{ color: 'var(--text-muted)' }}
                    >
                      目录: {rule.log_roots.length > 0 ? rule.log_roots.join(' ; ') : '待指定目录'} ·
                      后缀: {rule.mapping_spec.file_extensions.join(', ') || 'jsonl'}
                    </div>
                  </div>

                  <div className="flex items-center gap-2">
                    <label className="flex items-center gap-1 text-[11.5px] cursor-pointer">
                      <input
                        type="checkbox"
                        checked={rule.enabled}
                        onChange={(e) =>
                          upsertCustomRuleMutation.mutate({
                            ...rule,
                            enabled: e.target.checked,
                            updated_at_ms: Date.now(),
                          })
                        }
                      />
                      <span>启用</span>
                    </label>
                    <button
                      type="button"
                      onClick={() => {
                        setRuleDraft(rule);
                        setRootsTextDraft(rule.log_roots.join('\n'));
                        setExtTextDraft(
                          (rule.mapping_spec.file_extensions ?? ['jsonl']).join(', ')
                        );
                        setSampleTestResult(null);
                      }}
                      className="btn-interactive bg-[var(--bg-elevated)] px-2 py-1 text-[11px] rounded-lg cursor-pointer"
                      style={{
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-primary)',
                      }}
                    >
                      编辑
                    </button>
                    <button
                      type="button"
                      aria-label={`删除自定义规则 ${rule.rule_id}`}
                      onClick={() => deleteCustomRuleMutation.mutate(rule.rule_id)}
                      className="btn-danger inline-flex items-center gap-1 px-2 py-1 text-[11px] rounded-lg cursor-pointer"
                      style={{ color: 'var(--accent-rose)' }}
                    >
                      <Trash2 className="w-3 h-3" />
                      <span>删除</span>
                    </button>
                  </div>
                </div>
              );
            })
          )}
        </div>

        <div
          className="pt-3 space-y-3 text-[12px] pb-4"
          style={{ borderBottom: '1px solid var(--border-hairline)' }}
        >
          <div className="grid grid-cols-1 md:grid-cols-4 gap-3">
            <label className="space-y-1">
              <span className="block text-[11px]" style={{ color: 'var(--text-muted)' }}>
                规则 ID
              </span>
              <input
                type="text"
                aria-label="自定义规则 ID"
                placeholder="例如 my_gateway"
                value={ruleDraft.rule_id}
                onChange={(e) =>
                  setRuleDraft((prev) => ({ ...prev, rule_id: e.target.value }))
                }
                className="w-full px-2.5 py-1.5 text-[12px] rounded-lg outline-none"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--text-primary)',
                }}
              />
            </label>

            <label className="space-y-1">
              <span className="block text-[11px]" style={{ color: 'var(--text-muted)' }}>
                显示名称
              </span>
              <input
                type="text"
                aria-label="自定义规则显示名称"
                placeholder="例如 OpenRouter 本地代理"
                value={ruleDraft.display_name}
                onChange={(e) =>
                  setRuleDraft((prev) => ({ ...prev, display_name: e.target.value }))
                }
                className="w-full px-2.5 py-1.5 text-[12px] rounded-lg outline-none"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--text-primary)',
                }}
              />
            </label>

            <label className="space-y-1">
              <span className="block text-[11px]" style={{ color: 'var(--text-muted)' }}>
                协议格式预设
              </span>
              <select
                aria-label="自定义规则协议格式"
                value={ruleDraft.protocol_kind}
                onChange={(e) =>
                  setRuleDraft((prev) => ({
                    ...prev,
                    protocol_kind: e.target.value as CustomProtocolKind,
                  }))
                }
                className="w-full px-2.5 py-1.5 text-[12px] rounded-lg outline-none cursor-pointer"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--text-primary)',
                }}
              >
                {PROTOCOL_KINDS.map((pk) => (
                  <option key={pk.id} value={pk.id}>
                    {pk.label}
                  </option>
                ))}
              </select>
            </label>

            <label className="space-y-1">
              <span className="block text-[11px]" style={{ color: 'var(--text-muted)' }}>
                文件扩展名
              </span>
              <input
                type="text"
                placeholder="jsonl, json"
                value={extTextDraft}
                onChange={(e) => setExtTextDraft(e.target.value)}
                className="w-full px-2.5 py-1.5 text-[12px] rounded-lg outline-none"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--text-primary)',
                }}
              />
            </label>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
            <label className="md:col-span-2 space-y-1">
              <span className="block text-[11px]" style={{ color: 'var(--text-muted)' }}>
                日志目录
              </span>
              <textarea
                rows={2}
                placeholder="~/.my-cli/logs"
                value={rootsTextDraft}
                onChange={(e) => setRootsTextDraft(e.target.value)}
                className="w-full px-2.5 py-1.5 text-[11.5px] font-mono rounded-lg outline-none"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--text-primary)',
                }}
              />
            </label>

            <label className="space-y-1">
              <span className="block text-[11px]" style={{ color: 'var(--text-muted)' }}>
                归属提供商 ID
              </span>
              <input
                type="text"
                placeholder="custom_my_gateway"
                value={ruleDraft.mapping_spec.provider_id_override ?? ''}
                onChange={(e) =>
                  setRuleDraft((prev) => ({
                    ...prev,
                    mapping_spec: {
                      ...prev.mapping_spec,
                      provider_id_override: e.target.value || null,
                    },
                  }))
                }
                className="w-full px-2.5 py-1.5 text-[12px] rounded-lg outline-none"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--text-primary)',
                }}
              />
            </label>
          </div>

          {ruleDraft.protocol_kind === 'declarative_jsonl' && (
            <div className="grid grid-cols-2 md:grid-cols-4 gap-2.5 pt-1">
              {(
                [
                  ['timestamp_path', '时间戳'],
                  ['model_path', '模型'],
                  ['input_tokens_path', '输入 Token'],
                  ['output_tokens_path', '输出 Token'],
                  ['cache_read_tokens_path', '缓存读'],
                  ['cache_write_tokens_path', '缓存写'],
                  ['reasoning_tokens_path', '推理 Token'],
                  ['duration_ms_path', '耗时 ms'],
                ] as const
              ).map(([fieldKey, label]) => (
                <label key={fieldKey} className="space-y-1">
                  <span className="block text-[10.5px]" style={{ color: 'var(--text-muted)' }}>
                    {label}
                  </span>
                  <input
                    type="text"
                    value={ruleDraft.mapping_spec[fieldKey] ?? ''}
                    onChange={(e) =>
                      setRuleDraft((prev) => ({
                        ...prev,
                        mapping_spec: {
                          ...prev.mapping_spec,
                          [fieldKey]: e.target.value || null,
                        },
                      }))
                    }
                    className="w-full px-2 py-1 text-[11px] font-mono rounded-lg outline-none"
                    style={{
                      backgroundColor: 'var(--bg-input)',
                      border: '1px solid var(--border-subtle)',
                      color: 'var(--text-primary)',
                    }}
                  />
                </label>
              ))}
            </div>
          )}

          <div className="space-y-2 pt-1">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span className="text-[11px]" style={{ color: 'var(--text-muted)' }}>
                样本校验
              </span>
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  disabled={testCustomRuleMutation.isPending}
                  onClick={() =>
                    testCustomRuleMutation.mutate({
                      rule: buildNormalizedDraftRule(ruleDraft),
                      sampleLine: sampleLineDraft,
                    })
                  }
                  className="btn-interactive bg-[var(--bg-elevated)] inline-flex items-center gap-1.5 px-2.5 py-1 text-[11.5px] rounded-lg cursor-pointer disabled:opacity-50"
                  style={{
                    color: 'var(--text-primary)',
                    border: '1px solid var(--border-subtle)',
                  }}
                >
                  <FlaskConical className="w-3.5 h-3.5" />
                  <span>试跑验证样本</span>
                </button>
                <button
                  type="button"
                  disabled={upsertCustomRuleMutation.isPending}
                  onClick={() =>
                    upsertCustomRuleMutation.mutate(buildNormalizedDraftRule(ruleDraft))
                  }
                  className="btn-primary px-3 py-1 text-[11.5px] font-medium rounded-lg cursor-pointer disabled:opacity-50"
                  style={{
                    backgroundColor: 'var(--accent-primary)',
                    color: '#ffffff',
                  }}
                >
                  保存规则
                </button>
              </div>
            </div>

            <textarea
              rows={2}
              aria-label="脱敏 JSONL 样本行"
              value={sampleLineDraft}
              onChange={(e) => setSampleLineDraft(e.target.value)}
              className="w-full px-2.5 py-1.5 text-[11px] font-mono rounded-lg outline-none"
              style={{
                backgroundColor: 'var(--bg-input)',
                border: '1px solid var(--border-subtle)',
                color: 'var(--text-primary)',
              }}
            />

            {sampleTestResult && (
              <div
                className="px-3 py-2 rounded-lg text-[11.5px] tabular-nums flex flex-wrap items-center gap-4"
                style={{
                  backgroundColor: 'var(--bg-elevated)',
                  border: '1px solid var(--border-hairline)',
                  color: sampleTestResult.matched
                    ? 'var(--text-primary)'
                    : 'var(--accent-rose)',
                }}
              >
                <span className="font-semibold">
                  {sampleTestResult.matched ? '匹配成功' : '需调整规则'}
                </span>
                <span>模型提供商: {sampleTestResult.provider_id}</span>
                <span>模型: {sampleTestResult.model_id ?? '—'}</span>
                <span>输入: {formatExactInt(sampleTestResult.input_tokens)}</span>
                <span>输出: {formatExactInt(sampleTestResult.output_tokens)}</span>
                <span>缓存读: {formatExactInt(sampleTestResult.cache_read_tokens)}</span>
                <span>
                  等效成本:{' '}
                  {formatCost(
                    sampleTestResult.estimated_cost_micro_usd,
                    currency,
                    usdToCnyRate
                  )}
                </span>
                <span style={{ color: 'var(--text-muted)' }}>
                  {sampleTestResult.diagnostic}
                </span>
              </div>
            )}
          </div>
        </div>
      </section>

      {/* Section 4: Ingestion Cursors High-Density Table */}
      <section className="space-y-3 pb-4">
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-[13px] font-semibold">读取游标</h2>
          </div>
          <span className="text-[11.5px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
            共 {cursors.length} 个已跟踪文件
          </span>
        </div>

        <div className="overflow-x-auto">
          <table className="workspace-table w-full border-collapse text-[12px]">
            <thead>
              <tr
                style={{
                  borderBottom: '1px solid var(--border-strong)',
                  color: 'var(--text-muted)',
                }}
              >
                <th className="py-2 pr-3 text-left font-medium">模型提供商</th>
                <th className="py-2 px-3 text-left font-medium">文件路径</th>
                <th className="py-2 px-3 text-left font-medium">文件标识</th>
                <th className="py-2 px-3 text-right font-medium">已读偏移 (Bytes)</th>
                <th className="py-2 px-3 text-left font-medium">末行指纹</th>
                <th className="py-2 pl-3 pr-5 text-right font-medium">更新时间</th>
              </tr>
            </thead>
            <tbody>
              {cursors.length === 0 ? (
                <tr>
                  <td
                    colSpan={6}
                    className="py-8 text-center text-[12px]"
                    style={{ color: 'var(--text-muted)' }}
                  >
                    等待日志文件扫描记录
                  </td>
                </tr>
              ) : (
                cursors.map((c) => (
                  <tr
                    key={c.source_path}
                    className="row-interactive"
                    style={{ borderBottom: '1px solid var(--border-hairline)' }}
                  >
                    <td className="py-2 pr-3 font-medium whitespace-nowrap">
                      {providerDisplayName(c.provider_id)}
                    </td>
                    <td
                      className="py-2 px-3 font-mono text-[11px] max-w-[360px] truncate select-text"
                      title={c.source_path}
                      style={{ color: 'var(--text-secondary)' }}
                    >
                      {c.source_path}
                    </td>
                    <td
                      className="py-2 px-3 font-mono text-[11px] tabular-nums"
                      style={{ color: 'var(--text-muted)' }}
                    >
                      {c.file_identity}
                    </td>
                    <td className="py-2 px-3 text-right font-mono tabular-nums">
                      {formatExactInt(c.byte_offset)}
                    </td>
                    <td
                      className="py-2 px-3 font-mono text-[11px]"
                      style={{ color: 'var(--text-muted)' }}
                    >
                      {c.last_line_hash.slice(0, 12)}
                    </td>
                    <td
                      className="py-2 pl-3 text-right tabular-nums whitespace-nowrap"
                      style={{ color: 'var(--text-secondary)' }}
                    >
                      {formatRelativeTime(c.updated_at_ms)}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}
