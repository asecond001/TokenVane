export type BuiltInProviderId =
  | 'claude_code'
  | 'codex'
  | 'antigravity'
  | 'kimi_code'
  | 'workbuddy'
  | 'deepseek_harness'
  | 'qcode'
  | 'minimax_code';
export type ProviderId = BuiltInProviderId | (string & {});
export type CollectionMode = 'local_log' | 'otlp' | 'official_api';
export type CurrencyCode = 'USD' | 'CNY';
export type ThemeMode = 'system' | 'dark' | 'light';
export type QuotaUnit = 'micro_usd' | 'tokens';
export type TrendGranularity = 'auto' | 'fifteen_minutes' | 'hourly' | 'daily' | 'weekly';
export type StackDimension = 'provider' | 'model' | 'token_type';

export interface DeviceRecord {
  device_id: string;
  device_name: string;
  platform: string;
  created_at_ms: number;
}

export interface ProviderConfig {
  provider_id: string;
  enabled: boolean;
  collection_mode: CollectionMode;
  custom_log_paths: string[];
  has_credential: boolean;
  last_sync_at_ms: number | null;
  last_error: string | null;
  updated_at_ms: number;
}

export interface IngestionCursor {
  source_path: string;
  provider_id: string;
  file_identity: string;
  byte_offset: number;
  last_line_hash: string;
  updated_at_ms: number;
}

export interface AppPreferences {
  currency: CurrencyCode;
  usd_to_cny_rate: string;
  theme: ThemeMode | string;
  minimize_to_tray: boolean;
  launch_at_startup: boolean;
  otlp_enabled: boolean;
  otlp_port: number;
  device_display_name: string;
  bg_mode: 'none' | 'image' | 'gradient' | string;
  bg_image_path: string | null;
  bg_preset_id: string;
  bg_opacity: number;
  bg_blur_px: number;
  auto_sync_pricing: boolean;
  exchange_rate_auto: boolean;
  auto_check_updates: boolean;
  floating_window_enabled: boolean;
  background_cli_quota_poll: boolean;
  last_rate_sync_at_ms: number | null;
  last_pricing_sync_at_ms: number | null;
  last_subscription_sync_at_ms: number | null;
  updated_at_ms: number;
}

export interface ScanReport {
  files_scanned: number;
  events_inserted: number;
  events_deduplicated: number;
  rate_limits_updated?: number;
  active_providers?: string[];
  errors: string[];
  scanned_at_ms: number;
}

export interface BootstrapState {
  db_path: string;
  device: DeviceRecord;
  preferences: AppPreferences;
  providers: ProviderConfig[];
  collector_watching: boolean;
  otlp_running: boolean;
  otlp_port: number | null;
  last_scan: ScanReport | null;
}

export interface AnalyticsFilter {
  start_ms: number;
  end_ms: number;
  provider_id: string | null;
  device_id?: string | null;
  tz_offset_minutes: number;
  all_time?: boolean;
}

export interface TimeBucketPoint {
  bucket_start_ms: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  reasoning_tokens: number;
  cost_micro_usd: number;
  requests: number;
}

export interface ProviderSummaryRow {
  provider_id: string;
  enabled: boolean;
  period_tokens: number;
  period_cost_micro_usd: number;
  today_tokens: number;
  today_cost_micro_usd: number;
  month_tokens: number;
  month_cost_micro_usd: number;
  cache_hit_ratio: number;
  observable_steps: number;
  active_window_label: string | null;
  active_window_used_percent: number | null;
  last_sync_at_ms: number | null;
  last_error: string | null;
}

export interface UsageShareItem {
  key: string;
  label: string;
  tokens: number;
  token_share_percent: number;
  cost_micro_usd: number;
  cost_share_percent: number;
}

export interface ActiveSessionSummary {
  session_key: string;
  session_title?: string | null;
  provider_id: string;
  model_id: string;
  project_label: string;
  activity_state: 'active' | 'recent' | string;
  started_at_ms: number;
  last_active_at_ms: number;
  request_count: number;
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  estimated_cost_micro_usd: number;
  latest_output_tps: number | null;
}

export interface LiveModelSpeed {
  provider_id: string;
  model_id: string;
  output_tps: number;
  activity_state: 'active' | 'recent' | string;
}

export interface WorkspaceNavigatePayload {
  page?: string | null;
  provider_id?: string | null;
}

export interface OverviewReport {
  total_tokens: number;
  total_cost_micro_usd: number;
  today_tokens: number;
  today_cost_micro_usd: number;
  today_requests?: number;
  today_cache_hit_ratio?: number;
  today_cache_saved_micro_usd?: number;
  today_tokens_delta_pct?: number | null;
  today_cost_delta_pct?: number | null;
  today_hourly_pulse?: TimeBucketPoint[];
  cache_hit_ratio: number;
  avg_output_tps: number;
  peak_output_tps: number;
  latest_output_tps: number | null;
  live_model_speeds?: LiveModelSpeed[];
  total_requests: number;
  observable_steps: number;
  tokens_delta_pct: number | null;
  cost_delta_pct: number | null;
  cache_delta_pct: number | null;
  tps_delta_pct: number | null;
  series_granularity: string;
  series: TimeBucketPoint[];
  provider_series: TrendSeries[];
  providers: ProviderSummaryRow[];
  provider_shares: UsageShareItem[];
  top_model_shares: UsageShareItem[];
  active_sessions: ActiveSessionSummary[];
}

export interface TrendsQuery {
  filter: AnalyticsFilter;
  granularity: TrendGranularity;
  dimension: StackDimension;
}

export interface TrendPoint {
  bucket_start_ms: number;
  tokens: number;
  cost_micro_usd: number;
  requests: number;
  cache_hit_ratio?: number;
}

export interface TrendSeries {
  key: string;
  label: string;
  total_tokens: number;
  total_cost_micro_usd: number;
  points: TrendPoint[];
}

export interface TrendComparisonRow {
  key: string;
  label: string;
  current_tokens: number;
  previous_tokens: number;
  delta_pct: number | null;
  current_cost_micro_usd: number;
  previous_cost_micro_usd: number;
  cost_delta_pct?: number | null;
  cache_hit_ratio?: number;
  requests: number;
}

export interface TrendsReport {
  buckets: number[];
  series: TrendSeries[];
  comparisons: TrendComparisonRow[];
  effective_granularity: TrendGranularity;
}

export interface MonthlyQuery {
  year: number;
  month: number;
  provider_id: string | null;
  device_id?: string | null;
  tz_offset_minutes: number;
  /** Optional inclusive statistics range; overrides year/month when both are set. */
  start_ms?: number | null;
  end_ms?: number | null;
}

export interface MonthlyDailyLedgerRow {
  date: string;
  requests: number;
  observable_steps: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  reasoning_tokens: number;
  total_tokens: number;
  cost_micro_usd: number;
  cache_saved_micro_usd: number;
  top_model: string | null;
}

export interface HeatmapDayCell {
  date: string;
  total_tokens: number;
  cost_micro_usd: number;
  requests: number;
  top_model: string | null;
}

export interface MonthlyReport {
  year: number;
  month: number;
  api_equivalent_cost_micro_usd: number;
  subscription_monthly_fee_micro_usd: number;
  roi_multiplier: number | null;
  month_total_tokens: number;
  month_total_requests: number;
  month_cache_saved_micro_usd: number;
  provider_shares?: UsageShareItem[];
  model_shares?: UsageShareItem[];
  daily_ledger: MonthlyDailyLedgerRow[];
  heatmap_365d: HeatmapDayCell[];
  range_start_ms?: number;
  range_end_ms?: number;
  /** Subscription fee prorated to the statistics range; ROI is based on this. */
  subscription_period_fee_micro_usd?: number;
}

export interface ModelPricingEntry {
  model_pattern: string;
  provider_id: string;
  input_per_m_micro_usd: number;
  output_per_m_micro_usd: number;
  cache_read_per_m_micro_usd: number;
  cache_write_per_m_micro_usd: number;
  reasoning_per_m_micro_usd: number;
  is_user_override: boolean;
  effective_from: string;
}

export interface ModelStatRow {
  model_id: string;
  provider_id: string;
  requests: number;
  observable_steps: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  reasoning_tokens: number;
  total_tokens: number;
  cache_saved_micro_usd: number;
  estimated_cost_micro_usd: number;
  cost_share_ratio: number;
  avg_output_tps: number;
  effective_pricing: ModelPricingEntry | null;
  /** API list-price cost with prompt caching ignored (all prompt tokens at the input rate). */
  list_cost_micro_usd: number;
  /** Prorated subscription fee allocated by API-equivalent cost within the provider. */
  subscription_cost_micro_usd: number | null;
}

export interface ProviderSubscriptionAllocation {
  provider_id: string;
  plan_names: string[];
  monthly_fee_micro_usd: number;
  period_fee_micro_usd: number;
}

export interface ModelStatsReport {
  rows: ModelStatRow[];
  total_cost_micro_usd: number;
  total_cache_saved_micro_usd: number;
  catalog: ModelPricingEntry[];
  subscription_allocations: ProviderSubscriptionAllocation[];
}

export interface SubscriptionWindowRule {
  window_id: string;
  label: string;
  duration_hours: number;
  rolling: boolean;
  unit: QuotaUnit;
  limit_value: number;
  warning_percent: number;
  critical_percent: number;
}

export interface SubscriptionPlan {
  plan_id: string;
  provider_id: string;
  plan_name: string;
  enabled: boolean;
  monthly_fee_micro_usd: number;
  windows: SubscriptionWindowRule[];
  is_user_configured: boolean;
  auto_source?: string | null;
  updated_at_ms: number;
}

export interface SubscriptionPlanTemplate {
  template_id: string;
  provider_id: string;
  plan_name: string;
  monthly_fee_micro_usd: number;
  windows: SubscriptionWindowRule[];
  description: string;
  live_windows?: SubscriptionWindowStatus[];
}

export interface ObservedRateLimitWindow {
  provider_id: string;
  window_name: string;
  used_percent: number;
  window_minutes: number | null;
  resets_at_ms: number | null;
  observed_at_ms: number;
  source?: string | null;
  plan_hint?: string | null;
}

export interface SubscriptionWindowStatus {
  window_id: string;
  label: string;
  duration_hours: number;
  rolling: boolean;
  unit: QuotaUnit;
  used_value: number;
  limit_value: number;
  used_percent: number;
  warning_percent: number;
  critical_percent: number;
  burn_rate_per_hour: number;
  projected_exhaustion_hours: number | null;
  resets_at_ms: number | null;
  resets_in_seconds: number;
  source: string;
  alert_level: string;
}

export interface SubscriptionPlanStatus {
  plan: SubscriptionPlan;
  windows: SubscriptionWindowStatus[];
  today_cost_micro_usd: number;
  week_cost_micro_usd: number;
  month_cost_micro_usd: number;
  month_tokens: number;
}

/**
 * Caliber behind `*_tps`: `decode` divides by the streaming window after the first
 * token; `end_to_end` includes queueing / prefill / TTFT and reads systematically lower.
 */
export type SpeedBasis = 'decode' | 'end_to_end';

export interface ModelSpeedStats {
  model_id: string;
  provider_id: string;
  sample_count: number;
  mean_tps: number;
  p50_tps: number;
  p95_tps: number;
  max_tps: number;
  min_tps: number;
  latest_tps: number;
  avg_duration_ms: number;
  avg_ttft_ms: number | null;
  speed_basis?: SpeedBasis;
}

export interface ProviderSpeedStats {
  provider_id: string;
  sample_count: number;
  mean_tps: number;
  p50_tps: number;
  p95_tps: number;
  max_tps: number;
  min_tps: number;
  avg_duration_ms: number;
  avg_ttft_ms: number | null;
  speed_basis?: SpeedBasis;
}

export interface SpeedLatencyReport {
  providers: ProviderSpeedStats[];
  models: ModelSpeedStats[];
  from_hourly_rollups?: boolean;
}

export interface TraySummary {
  today_tokens: number;
  today_cost_micro_usd: number;
  today_requests: number;
  latest_output_tps: number | null;
  live_model_speeds?: LiveModelSpeed[];
  subscriptions: SubscriptionPlanStatus[];
  provider_shares: UsageShareItem[];
  active_sessions: ActiveSessionSummary[];
  currency: CurrencyCode;
  usd_to_cny_rate: string;
  active_providers_count: number;
  total_ingested_events: number;
}

export interface OfficialApiReconcileResult {
  provider_id: string;
  endpoint: string;
  status_code: number;
  buckets_received: number;
  total_input_tokens: number;
  total_output_tokens: number;
  reported_cost_usd: number | null;
  checked_at_ms: number;
}

export interface ExportReportResult {
  path: string | null;
  content: string;
  bytes: number;
}

export interface SyncHourlyDelta {
  provider_id: string;
  model_id: string;
  bucket_start_ms: number;
  request_count: number;
  observable_step_count: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  reasoning_tokens: number;
  estimated_cost_micro_usd: number;
  cache_saved_micro_usd: number;
  total_duration_ms: number;
  tps_sample_count: number;
  tps_sum: number;
  max_output_tps: number;
  updated_at_ms: number;
}

export interface SyncSubscriptionSnapshot {
  plan_id: string;
  provider_id: string;
  plan_name: string;
  window_id: string;
  window_label: string;
  used_percent: number;
  resets_at_ms: number | null;
  source: string;
  monthly_fee_micro_usd?: number;
}

export interface SyncPaygSnapshot {
  account_id: string;
  provider_id: string;
  account_name: string;
  enabled: boolean;
  currency: string | null;
  total_balance_micro: number | null;
  granted_balance_micro: number | null;
  topped_up_balance_micro: number | null;
  remaining_manual_balance_micro_usd: number | null;
  monthly_budget_micro_usd: number | null;
  today_cost_micro_usd: number;
  month_cost_micro_usd: number;
  fetched_at_ms: number | null;
  updated_at_ms: number;
}

export interface SyncEnvelopeV1 {
  schema_version: number;
  device_id: string;
  device_display_name: string;
  generated_at_ms: number;
  since_watermark_ms: number;
  hourly_deltas: SyncHourlyDelta[];
  subscription_snapshots: SyncSubscriptionSnapshot[];
  payg_snapshots?: SyncPaygSnapshot[];
}

export interface CatalogSyncResult {
  rate_updated: boolean;
  usd_to_cny_rate: string;
  pricing_updated: boolean;
  synced_models_count: number;
  recomputed_events_count: number;
  subscriptions_updated: boolean;
  synced_plans_count: number;
  synced_at_ms: number;
  rate_source: string | null;
  pricing_source: string | null;
  subscription_source: string | null;
  warning: string | null;
}

export interface ProviderBalanceSnapshot {
  provider_id: string;
  currency: string;
  total_balance_micro: number | null;
  granted_balance_micro: number | null;
  topped_up_balance_micro: number | null;
  daily_quota_used_micro: number | null;
  daily_quota_limit_micro: number | null;
  fetched_at_ms: number;
}

export interface BalanceRefreshOutcome {
  snapshot: ProviderBalanceSnapshot;
  rate_limits_updated: number;
  endpoint: string;
  from_cache: boolean;
  cooldown_remaining_secs: number;
}

export interface PaygAccount {
  account_id: string;
  provider_id: string;
  account_name: string;
  enabled: boolean;
  manual_balance_micro_usd: number | null;
  balance_anchor_ms: number | null;
  monthly_budget_micro_usd: number | null;
  is_auto: boolean;
  updated_at_ms: number;
}

export interface PaygAccountStatus {
  account: PaygAccount;
  official_balance: ProviderBalanceSnapshot | null;
  remaining_manual_balance_micro_usd: number | null;
  today_cost_micro_usd: number;
  month_cost_micro_usd: number;
  budget_used_percent: number | null;
}

export type CustomProtocolKind =
  | 'openai_jsonl'
  | 'anthropic_jsonl'
  | 'declarative_jsonl';

export interface DeclarativeMappingSpec {
  provider_id_override: string | null;
  file_extensions: string[];
  timestamp_path: string | null;
  model_path: string | null;
  session_id_path: string | null;
  request_id_path: string | null;
  input_tokens_path: string | null;
  output_tokens_path: string | null;
  cache_read_tokens_path: string | null;
  cache_write_tokens_path: string | null;
  reasoning_tokens_path: string | null;
  duration_ms_path: string | null;
  ttft_ms_path: string | null;
}

export interface CustomProviderRule {
  rule_id: string;
  display_name: string;
  enabled: boolean;
  protocol_kind: CustomProtocolKind;
  log_roots: string[];
  mapping_spec: DeclarativeMappingSpec;
  updated_at_ms: number;
}

export interface CustomRuleTestResult {
  matched: boolean;
  provider_id: string;
  model_id: string | null;
  timestamp_ms: number | null;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  reasoning_tokens: number;
  estimated_cost_micro_usd: number;
  diagnostic: string;
}

export interface SyncStatusView {
  enabled: boolean;
  worker_url: string | null;
  e2ee_enabled: boolean;
  e2ee_kdf_salt_hex: string | null;
  has_bearer_token: boolean;
  has_e2ee_passphrase: boolean;
  has_cf_api_token?: boolean;
  sse_connected: boolean;
  last_push_watermark_ms: number;
  last_pull_watermark_ms: number;
  last_sync_at_ms: number | null;
  last_sync_error: string | null;
  updated_at_ms: number;
}

export type CloudSyncDeployMode = 'wrangler_cli' | 'cloudflare_api';

/** Mirrors Rust `ProvisionStep` emitted on `cloud-sync-provision-progress`. */
export type CloudSyncProvisionStep =
  | 'preparing'
  | 'checking_auth'
  | 'browser_login'
  | 'resolving_account'
  | 'deploying'
  | 'writing_secret'
  | 'saving'
  | 'initial_sync';

export interface CloudSyncProvisionInput {
  mode: CloudSyncDeployMode;
  worker_name: string | null;
  bearer_token: string | null;
  e2ee_enabled: boolean;
  e2ee_passphrase: string | null;
  cf_api_token: string | null;
  cf_account_id: string | null;
}

export interface CloudSyncProvisionResult {
  mode: CloudSyncDeployMode;
  worker_name: string;
  worker_url: string;
  bearer_token: string;
  e2ee_passphrase: string | null;
  initial_sync_ok: boolean;
  status: SyncStatusView;
  pairing: SyncPairingBundle;
}

export interface SyncPairingBundle {
  pairing_uri: string;
  pairing_json: string;
  worker_url: string;
  e2ee_enabled: boolean;
  includes_passphrase: boolean;
}

export interface SyncRunReport {
  pushed_hourly_deltas: number;
  pushed_subscriptions: number;
  pulled_items: number;
  applied_hourly_buckets: number;
  applied_subscriptions: number;
  server_seq_ms: number | null;
  synced_at_ms: number;
}

export interface RemoteSubscriptionSnapshot {
  device_id: string;
  device_display_name: string;
  plan_id: string;
  window_id: string;
  provider_id: string;
  plan_name: string;
  window_label: string;
  used_percent: number;
  resets_at_ms: number | null;
  source: string;
  monthly_fee_micro_usd?: number;
  updated_at_ms: number;
}
