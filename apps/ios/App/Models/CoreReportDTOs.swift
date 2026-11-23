import Foundation

public struct AnalyticsFilterDTO: Codable, Equatable, Sendable {
    public let startMs: Int64
    public let endMs: Int64
    public let providerId: String?
    public let deviceId: String?
    public let tzOffsetMinutes: Int32
    public let allTime: Bool

    enum CodingKeys: String, CodingKey {
        case startMs = "start_ms"
        case endMs = "end_ms"
        case providerId = "provider_id"
        case deviceId = "device_id"
        case tzOffsetMinutes = "tz_offset_minutes"
        case allTime = "all_time"
    }

    public init(
        startMs: Int64,
        endMs: Int64,
        providerId: String? = nil,
        deviceId: String? = nil,
        tzOffsetMinutes: Int32 = Int32(TimeZone.current.secondsFromGMT() / 60),
        allTime: Bool = false
    ) {
        self.startMs = startMs
        self.endMs = endMs
        self.providerId = providerId
        self.deviceId = deviceId
        self.tzOffsetMinutes = tzOffsetMinutes
        self.allTime = allTime
    }
}

public enum TrendGranularityDTO: String, Codable, CaseIterable, Sendable {
    case auto
    case fifteenMinutes = "fifteen_minutes"
    case hourly
    case daily
    case weekly

    public var label: String {
        switch self {
        case .auto: return L10n.tr("gran.auto", default: "自动")
        case .fifteenMinutes: return L10n.tr("gran.15m", default: "15分")
        case .hourly: return L10n.tr("gran.hourly", default: "小时")
        case .daily: return L10n.tr("gran.daily", default: "日")
        case .weekly: return L10n.tr("gran.weekly", default: "周")
        }
    }
}

public enum StackDimensionDTO: String, Codable, CaseIterable, Sendable {
    case provider
    case model
    case tokenType = "token_type"

    public var label: String {
        switch self {
        case .provider: return L10n.tr("dim.provider", default: "服务商")
        case .model: return L10n.tr("dim.model", default: "模型")
        case .tokenType: return L10n.tr("dim.token_type", default: "类型")
        }
    }
}

public struct TrendsQueryDTO: Codable, Equatable, Sendable {
    public let filter: AnalyticsFilterDTO
    public let granularity: TrendGranularityDTO
    public let dimension: StackDimensionDTO

    public init(
        filter: AnalyticsFilterDTO,
        granularity: TrendGranularityDTO,
        dimension: StackDimensionDTO
    ) {
        self.filter = filter
        self.granularity = granularity
        self.dimension = dimension
    }
}

public struct MonthlyQueryDTO: Codable, Equatable, Sendable {
    public let year: Int32
    public let month: UInt32
    public let providerId: String?
    public let deviceId: String?
    public let tzOffsetMinutes: Int32
    public let startMs: Int64?
    public let endMs: Int64?

    enum CodingKeys: String, CodingKey {
        case year
        case month
        case providerId = "provider_id"
        case deviceId = "device_id"
        case tzOffsetMinutes = "tz_offset_minutes"
        case startMs = "start_ms"
        case endMs = "end_ms"
    }

    public init(
        year: Int32,
        month: UInt32,
        providerId: String? = nil,
        deviceId: String? = nil,
        tzOffsetMinutes: Int32 = Int32(TimeZone.current.secondsFromGMT() / 60),
        startMs: Int64? = nil,
        endMs: Int64? = nil
    ) {
        self.year = year
        self.month = month
        self.providerId = providerId
        self.deviceId = deviceId
        self.tzOffsetMinutes = tzOffsetMinutes
        self.startMs = startMs
        self.endMs = endMs
    }
}

public struct SyncedDevicePulseDTO: Codable, Equatable, Identifiable, Sendable {
    public let deviceId: String
    public let deviceName: String
    public let platform: String
    public let lastActivityMs: Int64?
    public let todayTokens: Int64
    public let todayCostMicroUsd: Int64
    public let totalTokens: Int64
    public let totalCostMicroUsd: Int64

    public var id: String { deviceId }

    enum CodingKeys: String, CodingKey {
        case deviceId = "device_id"
        case deviceName = "device_name"
        case platform
        case lastActivityMs = "last_activity_ms"
        case todayTokens = "today_tokens"
        case todayCostMicroUsd = "today_cost_micro_usd"
        case totalTokens = "total_tokens"
        case totalCostMicroUsd = "total_cost_micro_usd"
    }
}

public struct AppPreferencesDTO: Codable, Equatable, Sendable {
    public var currency: String
    public var usdToCnyRate: String
    public var theme: String
    public var minimizeToTray: Bool
    public var launchAtStartup: Bool
    public var otlpEnabled: Bool
    public var otlpPort: UInt16
    public var deviceDisplayName: String
    public var bgMode: String
    public var bgImagePath: String?
    public var bgPresetId: String
    public var bgOpacity: Double
    public var bgBlurPx: Int32
    public var autoSyncPricing: Bool
    public var exchangeRateAuto: Bool
    public var lastRateSyncAtMs: Int64?
    public var lastPricingSyncAtMs: Int64?
    public var lastSubscriptionSyncAtMs: Int64?
    public var autoCheckUpdates: Bool
    public var floatingWindowEnabled: Bool
    public var backgroundCliQuotaPoll: Bool
    public var updatedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case currency
        case usdToCnyRate = "usd_to_cny_rate"
        case theme
        case minimizeToTray = "minimize_to_tray"
        case launchAtStartup = "launch_at_startup"
        case otlpEnabled = "otlp_enabled"
        case otlpPort = "otlp_port"
        case deviceDisplayName = "device_display_name"
        case bgMode = "bg_mode"
        case bgImagePath = "bg_image_path"
        case bgPresetId = "bg_preset_id"
        case bgOpacity = "bg_opacity"
        case bgBlurPx = "bg_blur_px"
        case autoSyncPricing = "auto_sync_pricing"
        case exchangeRateAuto = "exchange_rate_auto"
        case lastRateSyncAtMs = "last_rate_sync_at_ms"
        case lastPricingSyncAtMs = "last_pricing_sync_at_ms"
        case lastSubscriptionSyncAtMs = "last_subscription_sync_at_ms"
        case autoCheckUpdates = "auto_check_updates"
        case floatingWindowEnabled = "floating_window_enabled"
        case backgroundCliQuotaPoll = "background_cli_quota_poll"
        case updatedAtMs = "updated_at_ms"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.currency = try container.decode(String.self, forKey: .currency)
        self.usdToCnyRate = try container.decode(String.self, forKey: .usdToCnyRate)
        self.theme = try container.decode(String.self, forKey: .theme)
        self.minimizeToTray = try container.decode(Bool.self, forKey: .minimizeToTray)
        self.launchAtStartup = try container.decode(Bool.self, forKey: .launchAtStartup)
        self.otlpEnabled = try container.decode(Bool.self, forKey: .otlpEnabled)
        self.otlpPort = try container.decode(UInt16.self, forKey: .otlpPort)
        self.deviceDisplayName = try container.decode(String.self, forKey: .deviceDisplayName)
        self.bgMode = try container.decode(String.self, forKey: .bgMode)
        self.bgImagePath = try container.decodeIfPresent(String.self, forKey: .bgImagePath)
        self.bgPresetId = try container.decode(String.self, forKey: .bgPresetId)
        self.bgOpacity = try container.decode(Double.self, forKey: .bgOpacity)
        self.bgBlurPx = try container.decode(Int32.self, forKey: .bgBlurPx)
        self.autoSyncPricing = try container.decode(Bool.self, forKey: .autoSyncPricing)
        self.exchangeRateAuto = try container.decode(Bool.self, forKey: .exchangeRateAuto)
        self.lastRateSyncAtMs = try container.decodeIfPresent(Int64.self, forKey: .lastRateSyncAtMs)
        self.lastPricingSyncAtMs = try container.decodeIfPresent(Int64.self, forKey: .lastPricingSyncAtMs)
        self.lastSubscriptionSyncAtMs = try container.decodeIfPresent(Int64.self, forKey: .lastSubscriptionSyncAtMs)
        self.autoCheckUpdates = try container.decode(Bool.self, forKey: .autoCheckUpdates)
        self.floatingWindowEnabled = try container.decodeIfPresent(Bool.self, forKey: .floatingWindowEnabled) ?? false
        self.backgroundCliQuotaPoll = try container.decodeIfPresent(Bool.self, forKey: .backgroundCliQuotaPoll) ?? true
        self.updatedAtMs = try container.decodeIfPresent(Int64.self, forKey: .updatedAtMs) ?? 0
    }
}

public struct SyncConfigStateDTO: Codable, Equatable, Sendable {
    public let enabled: Bool
    public let workerUrl: String?
    public let e2eeEnabled: Bool
    public let e2eeKdfSaltHex: String?
    public let lastPushWatermarkMs: Int64
    public let lastPullWatermarkMs: Int64
    public let lastSyncAtMs: Int64?
    public let lastSyncError: String?
    public let updatedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case enabled
        case workerUrl = "worker_url"
        case e2eeEnabled = "e2ee_enabled"
        case e2eeKdfSaltHex = "e2ee_kdf_salt_hex"
        case lastPushWatermarkMs = "last_push_watermark_ms"
        case lastPullWatermarkMs = "last_pull_watermark_ms"
        case lastSyncAtMs = "last_sync_at_ms"
        case lastSyncError = "last_sync_error"
        case updatedAtMs = "updated_at_ms"
    }
}

public struct RemoteSubscriptionSnapshotDTO: Codable, Equatable, Identifiable, Sendable {
    public let deviceId: String
    public let deviceDisplayName: String
    public let planId: String
    public let windowId: String
    public let providerId: String
    public let planName: String
    public let windowLabel: String
    public let usedPercent: Double
    public let resetsAtMs: Int64?
    public let source: String
    public let monthlyFeeMicroUsd: Int64
    public let updatedAtMs: Int64

    public var id: String { "\(deviceId)::\(planId)::\(windowId)" }

    enum CodingKeys: String, CodingKey {
        case deviceId = "device_id"
        case deviceDisplayName = "device_display_name"
        case planId = "plan_id"
        case windowId = "window_id"
        case providerId = "provider_id"
        case planName = "plan_name"
        case windowLabel = "window_label"
        case usedPercent = "used_percent"
        case resetsAtMs = "resets_at_ms"
        case source
        case monthlyFeeMicroUsd = "monthly_fee_micro_usd"
        case updatedAtMs = "updated_at_ms"
    }

    public init(
        deviceId: String,
        deviceDisplayName: String,
        planId: String,
        windowId: String,
        providerId: String,
        planName: String,
        windowLabel: String,
        usedPercent: Double,
        resetsAtMs: Int64?,
        source: String,
        monthlyFeeMicroUsd: Int64 = 0,
        updatedAtMs: Int64
    ) {
        self.deviceId = deviceId
        self.deviceDisplayName = deviceDisplayName
        self.planId = planId
        self.windowId = windowId
        self.providerId = providerId
        self.planName = planName
        self.windowLabel = windowLabel
        self.usedPercent = usedPercent
        self.resetsAtMs = resetsAtMs
        self.source = source
        self.monthlyFeeMicroUsd = monthlyFeeMicroUsd
        self.updatedAtMs = updatedAtMs
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.deviceId = try container.decode(String.self, forKey: .deviceId)
        self.deviceDisplayName = try container.decode(String.self, forKey: .deviceDisplayName)
        self.planId = try container.decode(String.self, forKey: .planId)
        self.windowId = try container.decode(String.self, forKey: .windowId)
        self.providerId = try container.decode(String.self, forKey: .providerId)
        self.planName = try container.decode(String.self, forKey: .planName)
        self.windowLabel = try container.decode(String.self, forKey: .windowLabel)
        self.usedPercent = try container.decode(Double.self, forKey: .usedPercent)
        self.resetsAtMs = try container.decodeIfPresent(Int64.self, forKey: .resetsAtMs)
        self.source = try container.decode(String.self, forKey: .source)
        self.monthlyFeeMicroUsd = try container.decodeIfPresent(Int64.self, forKey: .monthlyFeeMicroUsd) ?? 0
        self.updatedAtMs = try container.decode(Int64.self, forKey: .updatedAtMs)
    }
}

public struct RemotePaygSnapshotDTO: Codable, Equatable, Identifiable, Sendable {
    public let deviceId: String
    public let deviceDisplayName: String
    public let accountId: String
    public let providerId: String
    public let accountName: String
    public let enabled: Bool
    public let currency: String?
    public let totalBalanceMicro: Int64?
    public let grantedBalanceMicro: Int64?
    public let toppedUpBalanceMicro: Int64?
    public let remainingManualBalanceMicroUsd: Int64?
    public let monthlyBudgetMicroUsd: Int64?
    public let todayCostMicroUsd: Int64
    public let monthCostMicroUsd: Int64
    public let fetchedAtMs: Int64?
    public let updatedAtMs: Int64

    public var id: String { "\(deviceId)::\(accountId)" }

    enum CodingKeys: String, CodingKey {
        case deviceId = "device_id"
        case deviceDisplayName = "device_display_name"
        case accountId = "account_id"
        case providerId = "provider_id"
        case accountName = "account_name"
        case enabled
        case currency
        case totalBalanceMicro = "total_balance_micro"
        case grantedBalanceMicro = "granted_balance_micro"
        case toppedUpBalanceMicro = "topped_up_balance_micro"
        case remainingManualBalanceMicroUsd = "remaining_manual_balance_micro_usd"
        case monthlyBudgetMicroUsd = "monthly_budget_micro_usd"
        case todayCostMicroUsd = "today_cost_micro_usd"
        case monthCostMicroUsd = "month_cost_micro_usd"
        case fetchedAtMs = "fetched_at_ms"
        case updatedAtMs = "updated_at_ms"
    }
}

public struct MobileBootstrapStateDTO: Codable, Equatable, Sendable {
    public let schemaVersion: UInt32
    public let viewerDeviceId: String
    public let preferences: AppPreferencesDTO
    public let syncConfig: SyncConfigStateDTO
    public let syncedDevices: [SyncedDevicePulseDTO]
    public let remoteSubscriptions: [RemoteSubscriptionSnapshotDTO]
    public let remotePaygAccounts: [RemotePaygSnapshotDTO]

    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case viewerDeviceId = "viewer_device_id"
        case preferences
        case syncConfig = "sync_config"
        case syncedDevices = "synced_devices"
        case remoteSubscriptions = "remote_subscriptions"
        case remotePaygAccounts = "remote_payg_accounts"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.schemaVersion = try container.decode(UInt32.self, forKey: .schemaVersion)
        self.viewerDeviceId = try container.decode(String.self, forKey: .viewerDeviceId)
        self.preferences = try container.decode(AppPreferencesDTO.self, forKey: .preferences)
        self.syncConfig = try container.decode(SyncConfigStateDTO.self, forKey: .syncConfig)
        self.syncedDevices = try container.decode([SyncedDevicePulseDTO].self, forKey: .syncedDevices)
        self.remoteSubscriptions = try container.decode([RemoteSubscriptionSnapshotDTO].self, forKey: .remoteSubscriptions)
        self.remotePaygAccounts = try container.decodeIfPresent([RemotePaygSnapshotDTO].self, forKey: .remotePaygAccounts) ?? []
    }
}

public struct MobilePreferencesPatchDTO: Codable, Equatable, Sendable {
    public let currency: String?
    public let usdToCnyRate: String?
    public let theme: String?
    public let exchangeRateAuto: Bool?
    public let autoSyncPricing: Bool?
    public let deviceDisplayName: String?
    public let bgMode: String?
    public let bgPresetId: String?
    public let bgOpacity: Double?

    enum CodingKeys: String, CodingKey {
        case currency
        case usdToCnyRate = "usd_to_cny_rate"
        case theme
        case exchangeRateAuto = "exchange_rate_auto"
        case autoSyncPricing = "auto_sync_pricing"
        case deviceDisplayName = "device_display_name"
        case bgMode = "bg_mode"
        case bgPresetId = "bg_preset_id"
        case bgOpacity = "bg_opacity"
    }

    public init(
        currency: String? = nil,
        usdToCnyRate: String? = nil,
        theme: String? = nil,
        exchangeRateAuto: Bool? = nil,
        autoSyncPricing: Bool? = nil,
        deviceDisplayName: String? = nil,
        bgMode: String? = nil,
        bgPresetId: String? = nil,
        bgOpacity: Double? = nil
    ) {
        self.currency = currency
        self.usdToCnyRate = usdToCnyRate
        self.theme = theme
        self.exchangeRateAuto = exchangeRateAuto
        self.autoSyncPricing = autoSyncPricing
        self.deviceDisplayName = deviceDisplayName
        self.bgMode = bgMode
        self.bgPresetId = bgPresetId
        self.bgOpacity = bgOpacity
    }
}

public struct TimeBucketPointDTO: Codable, Equatable, Identifiable, Sendable {
    public let bucketStartMs: Int64
    public let inputTokens: Int64
    public let outputTokens: Int64
    public let cacheReadTokens: Int64
    public let cacheWriteTokens: Int64
    public let reasoningTokens: Int64
    public let costMicroUsd: Int64
    public let requests: Int64

    public var id: Int64 { bucketStartMs }

    enum CodingKeys: String, CodingKey {
        case bucketStartMs = "bucket_start_ms"
        case inputTokens = "input_tokens"
        case outputTokens = "output_tokens"
        case cacheReadTokens = "cache_read_tokens"
        case cacheWriteTokens = "cache_write_tokens"
        case reasoningTokens = "reasoning_tokens"
        case costMicroUsd = "cost_micro_usd"
        case requests
    }
}

public struct UsageShareItemDTO: Codable, Equatable, Identifiable, Sendable {
    public let key: String
    public let label: String
    public let tokens: Int64
    public let tokenSharePercent: Double
    public let costMicroUsd: Int64
    public let costSharePercent: Double

    public var id: String { key }

    enum CodingKeys: String, CodingKey {
        case key
        case label
        case tokens
        case tokenSharePercent = "token_share_percent"
        case costMicroUsd = "cost_micro_usd"
        case costSharePercent = "cost_share_percent"
    }
}

public struct ProviderSummaryRowDTO: Codable, Equatable, Identifiable, Sendable {
    public let providerId: String
    public let enabled: Bool
    public let periodTokens: Int64
    public let periodCostMicroUsd: Int64
    public let todayTokens: Int64
    public let todayCostMicroUsd: Int64
    public let monthTokens: Int64
    public let monthCostMicroUsd: Int64
    public let cacheHitRatio: Double
    public let observableSteps: Int64
    public let activeWindowLabel: String?
    public let activeWindowUsedPercent: Double?
    public let lastSyncAtMs: Int64?
    public let lastError: String?

    public var id: String { providerId }

    enum CodingKeys: String, CodingKey {
        case providerId = "provider_id"
        case enabled
        case periodTokens = "period_tokens"
        case periodCostMicroUsd = "period_cost_micro_usd"
        case todayTokens = "today_tokens"
        case todayCostMicroUsd = "today_cost_micro_usd"
        case monthTokens = "month_tokens"
        case monthCostMicroUsd = "month_cost_micro_usd"
        case cacheHitRatio = "cache_hit_ratio"
        case observableSteps = "observable_steps"
        case activeWindowLabel = "active_window_label"
        case activeWindowUsedPercent = "active_window_used_percent"
        case lastSyncAtMs = "last_sync_at_ms"
        case lastError = "last_error"
    }
}

public struct TrendPointDTO: Codable, Equatable, Identifiable, Sendable {
    public let bucketStartMs: Int64
    public let tokens: Int64
    public let costMicroUsd: Int64
    public let requests: Int64
    public let cacheHitRatio: Double

    public var id: Int64 { bucketStartMs }

    enum CodingKeys: String, CodingKey {
        case bucketStartMs = "bucket_start_ms"
        case tokens
        case costMicroUsd = "cost_micro_usd"
        case requests
        case cacheHitRatio = "cache_hit_ratio"
    }

    public init(
        bucketStartMs: Int64,
        tokens: Int64,
        costMicroUsd: Int64,
        requests: Int64,
        cacheHitRatio: Double = 0
    ) {
        self.bucketStartMs = bucketStartMs
        self.tokens = tokens
        self.costMicroUsd = costMicroUsd
        self.requests = requests
        self.cacheHitRatio = cacheHitRatio
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.bucketStartMs = try container.decode(Int64.self, forKey: .bucketStartMs)
        self.tokens = try container.decode(Int64.self, forKey: .tokens)
        self.costMicroUsd = try container.decode(Int64.self, forKey: .costMicroUsd)
        self.requests = try container.decode(Int64.self, forKey: .requests)
        self.cacheHitRatio = try container.decodeIfPresent(Double.self, forKey: .cacheHitRatio) ?? 0
    }
}

public struct TrendSeriesDTO: Codable, Equatable, Identifiable, Sendable {
    public let key: String
    public let label: String
    public let totalTokens: Int64
    public let totalCostMicroUsd: Int64
    public let points: [TrendPointDTO]

    public var id: String { key }

    enum CodingKeys: String, CodingKey {
        case key
        case label
        case totalTokens = "total_tokens"
        case totalCostMicroUsd = "total_cost_micro_usd"
        case points
    }
}

public struct TrendComparisonRowDTO: Codable, Equatable, Identifiable, Sendable {
    public let key: String
    public let label: String
    public let currentTokens: Int64
    public let previousTokens: Int64
    public let deltaPct: Double?
    public let currentCostMicroUsd: Int64
    public let previousCostMicroUsd: Int64
    public let costDeltaPct: Double?
    public let cacheHitRatio: Double
    public let requests: Int64

    public var id: String { key }

    enum CodingKeys: String, CodingKey {
        case key
        case label
        case currentTokens = "current_tokens"
        case previousTokens = "previous_tokens"
        case deltaPct = "delta_pct"
        case currentCostMicroUsd = "current_cost_micro_usd"
        case previousCostMicroUsd = "previous_cost_micro_usd"
        case costDeltaPct = "cost_delta_pct"
        case cacheHitRatio = "cache_hit_ratio"
        case requests
    }

    public init(
        key: String,
        label: String,
        currentTokens: Int64,
        previousTokens: Int64,
        deltaPct: Double?,
        currentCostMicroUsd: Int64,
        previousCostMicroUsd: Int64,
        costDeltaPct: Double? = nil,
        cacheHitRatio: Double = 0,
        requests: Int64
    ) {
        self.key = key
        self.label = label
        self.currentTokens = currentTokens
        self.previousTokens = previousTokens
        self.deltaPct = deltaPct
        self.currentCostMicroUsd = currentCostMicroUsd
        self.previousCostMicroUsd = previousCostMicroUsd
        self.costDeltaPct = costDeltaPct
        self.cacheHitRatio = cacheHitRatio
        self.requests = requests
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.key = try container.decode(String.self, forKey: .key)
        self.label = try container.decode(String.self, forKey: .label)
        self.currentTokens = try container.decode(Int64.self, forKey: .currentTokens)
        self.previousTokens = try container.decode(Int64.self, forKey: .previousTokens)
        self.deltaPct = try container.decodeIfPresent(Double.self, forKey: .deltaPct)
        self.currentCostMicroUsd = try container.decode(Int64.self, forKey: .currentCostMicroUsd)
        self.previousCostMicroUsd = try container.decode(Int64.self, forKey: .previousCostMicroUsd)
        self.costDeltaPct = try container.decodeIfPresent(Double.self, forKey: .costDeltaPct)
        self.cacheHitRatio = try container.decodeIfPresent(Double.self, forKey: .cacheHitRatio) ?? 0
        self.requests = try container.decode(Int64.self, forKey: .requests)
    }
}

public struct LiveModelSpeedDTO: Codable, Equatable, Identifiable, Sendable {
    public let providerId: String
    public let modelId: String
    public let outputTps: Double
    public let activityState: String

    public var id: String { "\(providerId)::\(modelId)" }

    enum CodingKeys: String, CodingKey {
        case providerId = "provider_id"
        case modelId = "model_id"
        case outputTps = "output_tps"
        case activityState = "activity_state"
    }
}

public struct ActiveSessionSummaryDTO: Codable, Equatable, Identifiable, Sendable {
    public let sessionKey: String
    public let sessionTitle: String?
    public let providerId: String
    public let modelId: String
    public let projectLabel: String
    public let activityState: String
    public let startedAtMs: Int64
    public let lastActiveAtMs: Int64
    public let requestCount: Int64
    public let totalTokens: Int64
    public let inputTokens: Int64
    public let outputTokens: Int64
    public let cacheReadTokens: Int64
    public let estimatedCostMicroUsd: Int64
    public let latestOutputTps: Double?

    public var id: String { sessionKey }

    enum CodingKeys: String, CodingKey {
        case sessionKey = "session_key"
        case sessionTitle = "session_title"
        case providerId = "provider_id"
        case modelId = "model_id"
        case projectLabel = "project_label"
        case activityState = "activity_state"
        case startedAtMs = "started_at_ms"
        case lastActiveAtMs = "last_active_at_ms"
        case requestCount = "request_count"
        case totalTokens = "total_tokens"
        case inputTokens = "input_tokens"
        case outputTokens = "output_tokens"
        case cacheReadTokens = "cache_read_tokens"
        case estimatedCostMicroUsd = "estimated_cost_micro_usd"
        case latestOutputTps = "latest_output_tps"
    }
}

public struct OverviewReportDTO: Codable, Equatable, Sendable {
    public let totalTokens: Int64
    public let totalCostMicroUsd: Int64
    public let todayTokens: Int64
    public let todayCostMicroUsd: Int64
    public let todayRequests: Int64
    public let todayCacheHitRatio: Double
    public let todayCacheSavedMicroUsd: Int64
    public let todayTokensDeltaPct: Double?
    public let todayCostDeltaPct: Double?
    public let todayHourlyPulse: [TimeBucketPointDTO]
    public let cacheHitRatio: Double
    public let avgOutputTps: Double
    public let peakOutputTps: Double
    public let latestOutputTps: Double?
    public let liveModelSpeeds: [LiveModelSpeedDTO]
    public let totalRequests: Int64
    public let observableSteps: Int64
    public let tokensDeltaPct: Double?
    public let costDeltaPct: Double?
    public let cacheDeltaPct: Double?
    public let tpsDeltaPct: Double?
    public let seriesGranularity: String
    public let series: [TimeBucketPointDTO]
    public let providerSeries: [TrendSeriesDTO]
    public let providers: [ProviderSummaryRowDTO]
    public let providerShares: [UsageShareItemDTO]
    public let topModelShares: [UsageShareItemDTO]
    public let activeSessions: [ActiveSessionSummaryDTO]

    enum CodingKeys: String, CodingKey {
        case totalTokens = "total_tokens"
        case totalCostMicroUsd = "total_cost_micro_usd"
        case todayTokens = "today_tokens"
        case todayCostMicroUsd = "today_cost_micro_usd"
        case todayRequests = "today_requests"
        case todayCacheHitRatio = "today_cache_hit_ratio"
        case todayCacheSavedMicroUsd = "today_cache_saved_micro_usd"
        case todayTokensDeltaPct = "today_tokens_delta_pct"
        case todayCostDeltaPct = "today_cost_delta_pct"
        case todayHourlyPulse = "today_hourly_pulse"
        case cacheHitRatio = "cache_hit_ratio"
        case avgOutputTps = "avg_output_tps"
        case peakOutputTps = "peak_output_tps"
        case latestOutputTps = "latest_output_tps"
        case liveModelSpeeds = "live_model_speeds"
        case totalRequests = "total_requests"
        case observableSteps = "observable_steps"
        case tokensDeltaPct = "tokens_delta_pct"
        case costDeltaPct = "cost_delta_pct"
        case cacheDeltaPct = "cache_delta_pct"
        case tpsDeltaPct = "tps_delta_pct"
        case seriesGranularity = "series_granularity"
        case series
        case providerSeries = "provider_series"
        case providers
        case providerShares = "provider_shares"
        case topModelShares = "top_model_shares"
        case activeSessions = "active_sessions"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.totalTokens = try container.decode(Int64.self, forKey: .totalTokens)
        self.totalCostMicroUsd = try container.decode(Int64.self, forKey: .totalCostMicroUsd)
        self.todayTokens = try container.decode(Int64.self, forKey: .todayTokens)
        self.todayCostMicroUsd = try container.decode(Int64.self, forKey: .todayCostMicroUsd)
        let decodedTotalRequests = try container.decode(Int64.self, forKey: .totalRequests)
        self.totalRequests = decodedTotalRequests
        self.todayRequests = try container.decodeIfPresent(Int64.self, forKey: .todayRequests) ?? decodedTotalRequests
        let decodedCacheRatio = try container.decode(Double.self, forKey: .cacheHitRatio)
        self.cacheHitRatio = decodedCacheRatio
        self.todayCacheHitRatio = try container.decodeIfPresent(Double.self, forKey: .todayCacheHitRatio) ?? decodedCacheRatio
        self.todayCacheSavedMicroUsd = try container.decodeIfPresent(Int64.self, forKey: .todayCacheSavedMicroUsd) ?? 0
        self.todayTokensDeltaPct = try container.decodeIfPresent(Double.self, forKey: .todayTokensDeltaPct)
        self.todayCostDeltaPct = try container.decodeIfPresent(Double.self, forKey: .todayCostDeltaPct)
        self.todayHourlyPulse = try container.decodeIfPresent([TimeBucketPointDTO].self, forKey: .todayHourlyPulse) ?? []
        self.avgOutputTps = try container.decode(Double.self, forKey: .avgOutputTps)
        self.peakOutputTps = try container.decode(Double.self, forKey: .peakOutputTps)
        self.latestOutputTps = try container.decodeIfPresent(Double.self, forKey: .latestOutputTps)
        self.liveModelSpeeds = try container.decodeIfPresent([LiveModelSpeedDTO].self, forKey: .liveModelSpeeds) ?? []
        self.observableSteps = try container.decode(Int64.self, forKey: .observableSteps)
        self.tokensDeltaPct = try container.decodeIfPresent(Double.self, forKey: .tokensDeltaPct)
        self.costDeltaPct = try container.decodeIfPresent(Double.self, forKey: .costDeltaPct)
        self.cacheDeltaPct = try container.decodeIfPresent(Double.self, forKey: .cacheDeltaPct)
        self.tpsDeltaPct = try container.decodeIfPresent(Double.self, forKey: .tpsDeltaPct)
        self.seriesGranularity = try container.decode(String.self, forKey: .seriesGranularity)
        self.series = try container.decode([TimeBucketPointDTO].self, forKey: .series)
        self.providerSeries = try container.decode([TrendSeriesDTO].self, forKey: .providerSeries)
        self.providers = try container.decode([ProviderSummaryRowDTO].self, forKey: .providers)
        self.providerShares = try container.decode([UsageShareItemDTO].self, forKey: .providerShares)
        self.topModelShares = try container.decode([UsageShareItemDTO].self, forKey: .topModelShares)
        self.activeSessions = try container.decodeIfPresent([ActiveSessionSummaryDTO].self, forKey: .activeSessions) ?? []
    }
}

public struct TrendsReportDTO: Codable, Equatable, Sendable {
    public let effectiveGranularity: TrendGranularityDTO
    public let buckets: [Int64]
    public let series: [TrendSeriesDTO]
    public let comparisons: [TrendComparisonRowDTO]

    enum CodingKeys: String, CodingKey {
        case effectiveGranularity = "effective_granularity"
        case buckets
        case series
        case comparisons
    }
}

public struct MonthlyDailyLedgerRowDTO: Codable, Equatable, Identifiable, Sendable {
    public let date: String
    public let requests: Int64
    public let observableSteps: Int64
    public let inputTokens: Int64
    public let outputTokens: Int64
    public let cacheReadTokens: Int64
    public let cacheWriteTokens: Int64
    public let reasoningTokens: Int64
    public let totalTokens: Int64
    public let costMicroUsd: Int64
    public let cacheSavedMicroUsd: Int64
    public let topModel: String?

    public var id: String { date }

    enum CodingKeys: String, CodingKey {
        case date
        case requests
        case observableSteps = "observable_steps"
        case inputTokens = "input_tokens"
        case outputTokens = "output_tokens"
        case cacheReadTokens = "cache_read_tokens"
        case cacheWriteTokens = "cache_write_tokens"
        case reasoningTokens = "reasoning_tokens"
        case totalTokens = "total_tokens"
        case costMicroUsd = "cost_micro_usd"
        case cacheSavedMicroUsd = "cache_saved_micro_usd"
        case topModel = "top_model"
    }
}

public struct HeatmapDayCellDTO: Codable, Equatable, Identifiable, Sendable {
    public let date: String
    public let totalTokens: Int64
    public let costMicroUsd: Int64
    public let requests: Int64
    public let topModel: String?

    public var id: String { date }

    enum CodingKeys: String, CodingKey {
        case date
        case totalTokens = "total_tokens"
        case costMicroUsd = "cost_micro_usd"
        case requests
        case topModel = "top_model"
    }
}

public struct MonthlyReportDTO: Codable, Equatable, Sendable {
    public let year: Int32
    public let month: UInt32
    public let apiEquivalentCostMicroUsd: Int64
    public let subscriptionMonthlyFeeMicroUsd: Int64
    public let roiMultiplier: Double?
    public let monthTotalTokens: Int64
    public let monthTotalRequests: Int64
    public let monthCacheSavedMicroUsd: Int64
    public let providerShares: [UsageShareItemDTO]
    public let modelShares: [UsageShareItemDTO]
    public let dailyLedger: [MonthlyDailyLedgerRowDTO]
    public let heatmap365d: [HeatmapDayCellDTO]
    public let rangeStartMs: Int64?
    public let rangeEndMs: Int64?
    public let subscriptionPeriodFeeMicroUsd: Int64?

    enum CodingKeys: String, CodingKey {
        case year
        case month
        case apiEquivalentCostMicroUsd = "api_equivalent_cost_micro_usd"
        case subscriptionMonthlyFeeMicroUsd = "subscription_monthly_fee_micro_usd"
        case roiMultiplier = "roi_multiplier"
        case monthTotalTokens = "month_total_tokens"
        case monthTotalRequests = "month_total_requests"
        case monthCacheSavedMicroUsd = "month_cache_saved_micro_usd"
        case providerShares = "provider_shares"
        case modelShares = "model_shares"
        case dailyLedger = "daily_ledger"
        case heatmap365d = "heatmap_365d"
        case rangeStartMs = "range_start_ms"
        case rangeEndMs = "range_end_ms"
        case subscriptionPeriodFeeMicroUsd = "subscription_period_fee_micro_usd"
    }

    public init(
        year: Int32,
        month: UInt32,
        apiEquivalentCostMicroUsd: Int64,
        subscriptionMonthlyFeeMicroUsd: Int64,
        roiMultiplier: Double?,
        monthTotalTokens: Int64,
        monthTotalRequests: Int64,
        monthCacheSavedMicroUsd: Int64,
        providerShares: [UsageShareItemDTO] = [],
        modelShares: [UsageShareItemDTO] = [],
        dailyLedger: [MonthlyDailyLedgerRowDTO],
        heatmap365d: [HeatmapDayCellDTO],
        rangeStartMs: Int64? = nil,
        rangeEndMs: Int64? = nil,
        subscriptionPeriodFeeMicroUsd: Int64? = nil
    ) {
        self.year = year
        self.month = month
        self.apiEquivalentCostMicroUsd = apiEquivalentCostMicroUsd
        self.subscriptionMonthlyFeeMicroUsd = subscriptionMonthlyFeeMicroUsd
        self.roiMultiplier = roiMultiplier
        self.monthTotalTokens = monthTotalTokens
        self.monthTotalRequests = monthTotalRequests
        self.monthCacheSavedMicroUsd = monthCacheSavedMicroUsd
        self.providerShares = providerShares
        self.modelShares = modelShares
        self.dailyLedger = dailyLedger
        self.heatmap365d = heatmap365d
        self.rangeStartMs = rangeStartMs
        self.rangeEndMs = rangeEndMs
        self.subscriptionPeriodFeeMicroUsd = subscriptionPeriodFeeMicroUsd
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.year = try container.decode(Int32.self, forKey: .year)
        self.month = try container.decode(UInt32.self, forKey: .month)
        self.apiEquivalentCostMicroUsd = try container.decode(Int64.self, forKey: .apiEquivalentCostMicroUsd)
        self.subscriptionMonthlyFeeMicroUsd = try container.decode(Int64.self, forKey: .subscriptionMonthlyFeeMicroUsd)
        self.roiMultiplier = try container.decodeIfPresent(Double.self, forKey: .roiMultiplier)
        self.monthTotalTokens = try container.decode(Int64.self, forKey: .monthTotalTokens)
        self.monthTotalRequests = try container.decode(Int64.self, forKey: .monthTotalRequests)
        self.monthCacheSavedMicroUsd = try container.decode(Int64.self, forKey: .monthCacheSavedMicroUsd)
        self.providerShares = try container.decodeIfPresent([UsageShareItemDTO].self, forKey: .providerShares) ?? []
        self.modelShares = try container.decodeIfPresent([UsageShareItemDTO].self, forKey: .modelShares) ?? []
        self.dailyLedger = try container.decode([MonthlyDailyLedgerRowDTO].self, forKey: .dailyLedger)
        self.heatmap365d = try container.decode([HeatmapDayCellDTO].self, forKey: .heatmap365d)
        self.rangeStartMs = try container.decodeIfPresent(Int64.self, forKey: .rangeStartMs)
        self.rangeEndMs = try container.decodeIfPresent(Int64.self, forKey: .rangeEndMs)
        self.subscriptionPeriodFeeMicroUsd = try container.decodeIfPresent(Int64.self, forKey: .subscriptionPeriodFeeMicroUsd)
    }
}

public struct ModelPricingEntryDTO: Codable, Equatable, Identifiable, Sendable {
    public let modelPattern: String
    public let providerId: String
    public let inputPerMMicroUsd: Int64
    public let outputPerMMicroUsd: Int64
    public let cacheReadPerMMicroUsd: Int64
    public let cacheWritePerMMicroUsd: Int64
    public let reasoningPerMMicroUsd: Int64
    public let isUserOverride: Bool
    public let effectiveFrom: String

    public var id: String { "\(providerId)::\(modelPattern)" }

    enum CodingKeys: String, CodingKey {
        case modelPattern = "model_pattern"
        case providerId = "provider_id"
        case inputPerMMicroUsd = "input_per_m_micro_usd"
        case outputPerMMicroUsd = "output_per_m_micro_usd"
        case cacheReadPerMMicroUsd = "cache_read_per_m_micro_usd"
        case cacheWritePerMMicroUsd = "cache_write_per_m_micro_usd"
        case reasoningPerMMicroUsd = "reasoning_per_m_micro_usd"
        case isUserOverride = "is_user_override"
        case effectiveFrom = "effective_from"
    }

    public init(
        modelPattern: String,
        providerId: String,
        inputPerMMicroUsd: Int64,
        outputPerMMicroUsd: Int64,
        cacheReadPerMMicroUsd: Int64,
        cacheWritePerMMicroUsd: Int64,
        reasoningPerMMicroUsd: Int64,
        isUserOverride: Bool,
        effectiveFrom: String
    ) {
        self.modelPattern = modelPattern
        self.providerId = providerId
        self.inputPerMMicroUsd = inputPerMMicroUsd
        self.outputPerMMicroUsd = outputPerMMicroUsd
        self.cacheReadPerMMicroUsd = cacheReadPerMMicroUsd
        self.cacheWritePerMMicroUsd = cacheWritePerMMicroUsd
        self.reasoningPerMMicroUsd = reasoningPerMMicroUsd
        self.isUserOverride = isUserOverride
        self.effectiveFrom = effectiveFrom
    }
}

public enum SpeedBasisDTO: String, Codable, Equatable, Sendable {
    case decode
    case endToEnd = "end_to_end"
}

public struct ProviderSubscriptionAllocationDTO: Codable, Equatable, Identifiable, Sendable {
    public let providerId: String
    public let planNames: [String]
    public let monthlyFeeMicroUsd: Int64
    public let periodFeeMicroUsd: Int64

    public var id: String { providerId }

    enum CodingKeys: String, CodingKey {
        case providerId = "provider_id"
        case planNames = "plan_names"
        case monthlyFeeMicroUsd = "monthly_fee_micro_usd"
        case periodFeeMicroUsd = "period_fee_micro_usd"
    }

    public init(
        providerId: String,
        planNames: [String],
        monthlyFeeMicroUsd: Int64,
        periodFeeMicroUsd: Int64
    ) {
        self.providerId = providerId
        self.planNames = planNames
        self.monthlyFeeMicroUsd = monthlyFeeMicroUsd
        self.periodFeeMicroUsd = periodFeeMicroUsd
    }
}

public struct ModelStatRowDTO: Codable, Equatable, Identifiable, Sendable {
    public let modelId: String
    public let providerId: String
    public let requests: Int64
    public let observableSteps: Int64
    public let inputTokens: Int64
    public let outputTokens: Int64
    public let cacheReadTokens: Int64
    public let cacheWriteTokens: Int64
    public let reasoningTokens: Int64
    public let totalTokens: Int64
    public let cacheSavedMicroUsd: Int64
    public let estimatedCostMicroUsd: Int64
    public let costShareRatio: Double
    public let avgOutputTps: Double
    public let effectivePricing: ModelPricingEntryDTO?
    public let listCostMicroUsd: Int64
    public let subscriptionCostMicroUsd: Int64?

    public var id: String { "\(providerId)::\(modelId)" }

    enum CodingKeys: String, CodingKey {
        case modelId = "model_id"
        case providerId = "provider_id"
        case requests
        case observableSteps = "observable_steps"
        case inputTokens = "input_tokens"
        case outputTokens = "output_tokens"
        case cacheReadTokens = "cache_read_tokens"
        case cacheWriteTokens = "cache_write_tokens"
        case reasoningTokens = "reasoning_tokens"
        case totalTokens = "total_tokens"
        case cacheSavedMicroUsd = "cache_saved_micro_usd"
        case estimatedCostMicroUsd = "estimated_cost_micro_usd"
        case costShareRatio = "cost_share_ratio"
        case avgOutputTps = "avg_output_tps"
        case effectivePricing = "effective_pricing"
        case listCostMicroUsd = "list_cost_micro_usd"
        case subscriptionCostMicroUsd = "subscription_cost_micro_usd"
    }

    public init(
        modelId: String,
        providerId: String,
        requests: Int64,
        observableSteps: Int64,
        inputTokens: Int64,
        outputTokens: Int64,
        cacheReadTokens: Int64,
        cacheWriteTokens: Int64,
        reasoningTokens: Int64,
        totalTokens: Int64,
        cacheSavedMicroUsd: Int64,
        estimatedCostMicroUsd: Int64,
        costShareRatio: Double,
        avgOutputTps: Double,
        effectivePricing: ModelPricingEntryDTO? = nil,
        listCostMicroUsd: Int64 = 0,
        subscriptionCostMicroUsd: Int64? = nil
    ) {
        self.modelId = modelId
        self.providerId = providerId
        self.requests = requests
        self.observableSteps = observableSteps
        self.inputTokens = inputTokens
        self.outputTokens = outputTokens
        self.cacheReadTokens = cacheReadTokens
        self.cacheWriteTokens = cacheWriteTokens
        self.reasoningTokens = reasoningTokens
        self.totalTokens = totalTokens
        self.cacheSavedMicroUsd = cacheSavedMicroUsd
        self.estimatedCostMicroUsd = estimatedCostMicroUsd
        self.costShareRatio = costShareRatio
        self.avgOutputTps = avgOutputTps
        self.effectivePricing = effectivePricing
        self.listCostMicroUsd = listCostMicroUsd
        self.subscriptionCostMicroUsd = subscriptionCostMicroUsd
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.modelId = try container.decode(String.self, forKey: .modelId)
        self.providerId = try container.decode(String.self, forKey: .providerId)
        self.requests = try container.decode(Int64.self, forKey: .requests)
        self.observableSteps = try container.decode(Int64.self, forKey: .observableSteps)
        self.inputTokens = try container.decode(Int64.self, forKey: .inputTokens)
        self.outputTokens = try container.decode(Int64.self, forKey: .outputTokens)
        self.cacheReadTokens = try container.decode(Int64.self, forKey: .cacheReadTokens)
        self.cacheWriteTokens = try container.decode(Int64.self, forKey: .cacheWriteTokens)
        self.reasoningTokens = try container.decode(Int64.self, forKey: .reasoningTokens)
        self.totalTokens = try container.decode(Int64.self, forKey: .totalTokens)
        self.cacheSavedMicroUsd = try container.decode(Int64.self, forKey: .cacheSavedMicroUsd)
        self.estimatedCostMicroUsd = try container.decode(Int64.self, forKey: .estimatedCostMicroUsd)
        self.costShareRatio = try container.decode(Double.self, forKey: .costShareRatio)
        self.avgOutputTps = try container.decode(Double.self, forKey: .avgOutputTps)
        self.effectivePricing = try container.decodeIfPresent(ModelPricingEntryDTO.self, forKey: .effectivePricing)
        self.listCostMicroUsd = try container.decodeIfPresent(Int64.self, forKey: .listCostMicroUsd) ?? 0
        self.subscriptionCostMicroUsd = try container.decodeIfPresent(Int64.self, forKey: .subscriptionCostMicroUsd)
    }

    public var cacheHitPercent: Double {
        let denom = inputTokens + cacheReadTokens + cacheWriteTokens
        guard denom > 0 else { return 0 }
        return (Double(cacheReadTokens) / Double(denom)) * 100.0
    }
}

public struct ModelStatsReportDTO: Codable, Equatable, Sendable {
    public let rows: [ModelStatRowDTO]
    public let totalCostMicroUsd: Int64
    public let totalCacheSavedMicroUsd: Int64
    public let catalog: [ModelPricingEntryDTO]
    public let subscriptionAllocations: [ProviderSubscriptionAllocationDTO]

    enum CodingKeys: String, CodingKey {
        case rows
        case totalCostMicroUsd = "total_cost_micro_usd"
        case totalCacheSavedMicroUsd = "total_cache_saved_micro_usd"
        case catalog
        case subscriptionAllocations = "subscription_allocations"
    }

    public init(
        rows: [ModelStatRowDTO],
        totalCostMicroUsd: Int64,
        totalCacheSavedMicroUsd: Int64,
        catalog: [ModelPricingEntryDTO] = [],
        subscriptionAllocations: [ProviderSubscriptionAllocationDTO] = []
    ) {
        self.rows = rows
        self.totalCostMicroUsd = totalCostMicroUsd
        self.totalCacheSavedMicroUsd = totalCacheSavedMicroUsd
        self.catalog = catalog
        self.subscriptionAllocations = subscriptionAllocations
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.rows = try container.decode([ModelStatRowDTO].self, forKey: .rows)
        self.totalCostMicroUsd = try container.decode(Int64.self, forKey: .totalCostMicroUsd)
        self.totalCacheSavedMicroUsd = try container.decode(Int64.self, forKey: .totalCacheSavedMicroUsd)
        self.catalog = try container.decodeIfPresent([ModelPricingEntryDTO].self, forKey: .catalog) ?? []
        self.subscriptionAllocations = try container.decodeIfPresent([ProviderSubscriptionAllocationDTO].self, forKey: .subscriptionAllocations) ?? []
    }
}

public struct ModelSpeedStatsDTO: Codable, Equatable, Identifiable, Sendable {
    public let modelId: String
    public let providerId: String
    public let sampleCount: Int64
    public let meanTps: Double
    public let p50Tps: Double
    public let p95Tps: Double
    public let maxTps: Double
    public let minTps: Double
    public let latestTps: Double
    public let avgDurationMs: Double
    public let avgTtftMs: Double?
    public let speedBasis: SpeedBasisDTO?

    public var id: String { "\(providerId)::\(modelId)" }

    enum CodingKeys: String, CodingKey {
        case modelId = "model_id"
        case providerId = "provider_id"
        case sampleCount = "sample_count"
        case meanTps = "mean_tps"
        case p50Tps = "p50_tps"
        case p95Tps = "p95_tps"
        case maxTps = "max_tps"
        case minTps = "min_tps"
        case latestTps = "latest_tps"
        case avgDurationMs = "avg_duration_ms"
        case avgTtftMs = "avg_ttft_ms"
        case speedBasis = "speed_basis"
    }

    public init(
        modelId: String,
        providerId: String,
        sampleCount: Int64,
        meanTps: Double,
        p50Tps: Double,
        p95Tps: Double,
        maxTps: Double,
        minTps: Double,
        latestTps: Double,
        avgDurationMs: Double,
        avgTtftMs: Double?,
        speedBasis: SpeedBasisDTO? = nil
    ) {
        self.modelId = modelId
        self.providerId = providerId
        self.sampleCount = sampleCount
        self.meanTps = meanTps
        self.p50Tps = p50Tps
        self.p95Tps = p95Tps
        self.maxTps = maxTps
        self.minTps = minTps
        self.latestTps = latestTps
        self.avgDurationMs = avgDurationMs
        self.avgTtftMs = avgTtftMs
        self.speedBasis = speedBasis
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.modelId = try container.decode(String.self, forKey: .modelId)
        self.providerId = try container.decode(String.self, forKey: .providerId)
        self.sampleCount = try container.decode(Int64.self, forKey: .sampleCount)
        self.meanTps = try container.decode(Double.self, forKey: .meanTps)
        self.p50Tps = try container.decode(Double.self, forKey: .p50Tps)
        self.p95Tps = try container.decode(Double.self, forKey: .p95Tps)
        self.maxTps = try container.decode(Double.self, forKey: .maxTps)
        self.minTps = try container.decode(Double.self, forKey: .minTps)
        self.latestTps = try container.decode(Double.self, forKey: .latestTps)
        self.avgDurationMs = try container.decode(Double.self, forKey: .avgDurationMs)
        self.avgTtftMs = try container.decodeIfPresent(Double.self, forKey: .avgTtftMs)
        self.speedBasis = try container.decodeIfPresent(String.self, forKey: .speedBasis).flatMap(SpeedBasisDTO.init(rawValue:))
    }
}

public struct ProviderSpeedStatsDTO: Codable, Equatable, Identifiable, Sendable {
    public let providerId: String
    public let sampleCount: Int64
    public let meanTps: Double
    public let p50Tps: Double
    public let p95Tps: Double
    public let maxTps: Double
    public let minTps: Double
    public let avgDurationMs: Double
    public let avgTtftMs: Double?
    public let speedBasis: SpeedBasisDTO?

    public var id: String { providerId }

    enum CodingKeys: String, CodingKey {
        case providerId = "provider_id"
        case sampleCount = "sample_count"
        case meanTps = "mean_tps"
        case p50Tps = "p50_tps"
        case p95Tps = "p95_tps"
        case maxTps = "max_tps"
        case minTps = "min_tps"
        case avgDurationMs = "avg_duration_ms"
        case avgTtftMs = "avg_ttft_ms"
        case speedBasis = "speed_basis"
    }

    public init(
        providerId: String,
        sampleCount: Int64,
        meanTps: Double,
        p50Tps: Double,
        p95Tps: Double,
        maxTps: Double,
        minTps: Double,
        avgDurationMs: Double,
        avgTtftMs: Double?,
        speedBasis: SpeedBasisDTO? = nil
    ) {
        self.providerId = providerId
        self.sampleCount = sampleCount
        self.meanTps = meanTps
        self.p50Tps = p50Tps
        self.p95Tps = p95Tps
        self.maxTps = maxTps
        self.minTps = minTps
        self.avgDurationMs = avgDurationMs
        self.avgTtftMs = avgTtftMs
        self.speedBasis = speedBasis
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.providerId = try container.decode(String.self, forKey: .providerId)
        self.sampleCount = try container.decode(Int64.self, forKey: .sampleCount)
        self.meanTps = try container.decode(Double.self, forKey: .meanTps)
        self.p50Tps = try container.decode(Double.self, forKey: .p50Tps)
        self.p95Tps = try container.decode(Double.self, forKey: .p95Tps)
        self.maxTps = try container.decode(Double.self, forKey: .maxTps)
        self.minTps = try container.decode(Double.self, forKey: .minTps)
        self.avgDurationMs = try container.decode(Double.self, forKey: .avgDurationMs)
        self.avgTtftMs = try container.decodeIfPresent(Double.self, forKey: .avgTtftMs)
        self.speedBasis = try container.decodeIfPresent(String.self, forKey: .speedBasis).flatMap(SpeedBasisDTO.init(rawValue:))
    }
}

public struct SpeedLatencyReportDTO: Codable, Equatable, Sendable {
    public let providers: [ProviderSpeedStatsDTO]
    public let models: [ModelSpeedStatsDTO]
    public let fromHourlyRollups: Bool

    enum CodingKeys: String, CodingKey {
        case providers
        case models
        case fromHourlyRollups = "from_hourly_rollups"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.providers = try container.decode([ProviderSpeedStatsDTO].self, forKey: .providers)
        self.models = try container.decode([ModelSpeedStatsDTO].self, forKey: .models)
        self.fromHourlyRollups = try container.decodeIfPresent(Bool.self, forKey: .fromHourlyRollups) ?? false
    }

    public init(
        providers: [ProviderSpeedStatsDTO],
        models: [ModelSpeedStatsDTO],
        fromHourlyRollups: Bool
    ) {
        self.providers = providers
        self.models = models
        self.fromHourlyRollups = fromHourlyRollups
    }
}

public struct DesktopPairingPayload: Codable, Equatable, Sendable {
    public let v: Int
    public let url: String
    public let token: String
    public let e2ee: Bool
    public let key: String?

    public init(v: Int, url: String, token: String, e2ee: Bool, key: String? = nil) {
        self.v = v
        self.url = url
        self.token = token
        self.e2ee = e2ee
        self.key = key
    }

    public enum PairingError: LocalizedError, Equatable {
        case invalidJSON
        case unsupportedVersion(Int)
        case invalidURL
        case emptyToken

        public var errorDescription: String? {
            switch self {
            case .invalidJSON: return L10n.tr("pairing.error.invalid", default: "配对格式无效")
            case .unsupportedVersion(let v): return L10n.trFormat("pairing.error.version", default: "需更新 App 以支持 v%lld", Int64(v))
            case .invalidURL: return L10n.tr("pairing.error.url", default: "Worker 地址需为 https:// 或本地 http://")
            case .emptyToken: return L10n.tr("pairing.error.token", default: "Sync Token 为必填项")
            }
        }
    }

    public static func parse(_ raw: String) throws -> DesktopPairingPayload {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        let decoded: DesktopPairingPayload
        if trimmed.lowercased().hasPrefix("tokenvane://") || trimmed.lowercased().hasPrefix("tokentracker://") {
            guard let components = URLComponents(string: trimmed),
                  (components.host?.lowercased() == "pair" || components.path.lowercased().contains("pair")),
                  let items = components.queryItems else {
                throw PairingError.invalidJSON
            }
            let map = Dictionary(items.compactMap { item -> (String, String)? in
                guard let value = item.value else { return nil }
                return (item.name.lowercased(), value)
            }, uniquingKeysWith: { _, latest in latest })

            let version = Int(map["v"] ?? "1") ?? 0
            let urlVal = map["url"] ?? map["u"] ?? ""
            let tokenVal = map["token"] ?? map["t"] ?? ""
            let e2eeRaw = (map["e2ee"] ?? map["e"] ?? "0").lowercased()
            let e2eeVal = e2eeRaw == "1" || e2eeRaw == "true" || e2eeRaw == "yes"
            let keyVal = (map["key"] ?? map["k"])?.trimmingCharacters(in: .whitespacesAndNewlines)
            decoded = DesktopPairingPayload(
                v: version,
                url: urlVal,
                token: tokenVal,
                e2ee: e2eeVal,
                key: keyVal?.isEmpty == true ? nil : keyVal
            )
        } else {
            guard let data = trimmed.data(using: .utf8),
                  let jsonPayload = try? JSONDecoder().decode(DesktopPairingPayload.self, from: data) else {
                throw PairingError.invalidJSON
            }
            decoded = jsonPayload
        }
        guard decoded.v == 1 else {
            throw PairingError.unsupportedVersion(decoded.v)
        }
        let cleanedURL = decoded.url.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let parsed = URL(string: cleanedURL),
              let scheme = parsed.scheme?.lowercased(),
              (scheme == "https" || (scheme == "http" && (parsed.host == "127.0.0.1" || parsed.host == "localhost"))),
              parsed.host != nil else {
            throw PairingError.invalidURL
        }
        let cleanedToken = decoded.token.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !cleanedToken.isEmpty else {
            throw PairingError.emptyToken
        }
        let cleanedKey = decoded.key?.trimmingCharacters(in: .whitespacesAndNewlines)
        return DesktopPairingPayload(
            v: decoded.v,
            url: cleanedURL.hasSuffix("/") ? String(cleanedURL.dropLast()) : cleanedURL,
            token: cleanedToken,
            e2ee: decoded.e2ee,
            key: cleanedKey?.isEmpty == true ? nil : cleanedKey
        )
    }

    /// Builds a `tokenvane://pair?v=1...` pairing link matching desktop/Rust `build_sync_pairing_uri`.
    public static func makePairingURI(
        url: String,
        token: String,
        e2ee: Bool,
        key: String? = nil
    ) -> String? {
        let trimmedURL = url.trimmingCharacters(in: .whitespacesAndNewlines)
        let normalizedURL = trimmedURL.hasSuffix("/") ? String(trimmedURL.dropLast()) : trimmedURL
        let trimmedToken = token.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedURL.isEmpty,
              let parsed = URL(string: normalizedURL),
              parsed.scheme != nil,
              parsed.host != nil,
              !trimmedToken.isEmpty else {
            return nil
        }

        var items: [URLQueryItem] = [
            URLQueryItem(name: "v", value: "1"),
            URLQueryItem(name: "url", value: normalizedURL),
            URLQueryItem(name: "token", value: trimmedToken),
            URLQueryItem(name: "e2ee", value: e2ee ? "1" : "0")
        ]
        if e2ee, let trimmedKey = key?.trimmingCharacters(in: .whitespacesAndNewlines), !trimmedKey.isEmpty {
            items.append(URLQueryItem(name: "key", value: trimmedKey))
        }

        var components = URLComponents()
        components.scheme = "tokenvane"
        components.host = "pair"
        components.queryItems = items
        return components.string
    }
}

public struct SubscriptionWindowRuleDTO: Codable, Equatable, Identifiable, Sendable {
    public let windowId: String
    public let label: String
    public let durationHours: Int64
    public let rolling: Bool
    public let unit: String
    public let limitValue: Int64
    public let warningPercent: UInt8
    public let criticalPercent: UInt8

    public var id: String { windowId }

    enum CodingKeys: String, CodingKey {
        case windowId = "window_id"
        case label
        case durationHours = "duration_hours"
        case rolling
        case unit
        case limitValue = "limit_value"
        case warningPercent = "warning_percent"
        case criticalPercent = "critical_percent"
    }

    public init(
        windowId: String,
        label: String,
        durationHours: Int64,
        rolling: Bool,
        unit: String,
        limitValue: Int64,
        warningPercent: UInt8 = 80,
        criticalPercent: UInt8 = 95
    ) {
        self.windowId = windowId
        self.label = label
        self.durationHours = durationHours
        self.rolling = rolling
        self.unit = unit
        self.limitValue = limitValue
        self.warningPercent = warningPercent
        self.criticalPercent = criticalPercent
    }
}

public struct SubscriptionWindowStatusDTO: Codable, Equatable, Identifiable, Sendable {
    public let windowId: String
    public let label: String
    public let durationHours: Int64
    public let rolling: Bool
    public let unit: String
    public let usedValue: Int64
    public let limitValue: Int64
    public let usedPercent: Double
    public let warningPercent: UInt8
    public let criticalPercent: UInt8
    public let burnRatePerHour: Double
    public let projectedExhaustionHours: Double?
    public let resetsAtMs: Int64?
    public let resetsInSeconds: Int64
    public let source: String
    public let alertLevel: String

    public var id: String { windowId }

    enum CodingKeys: String, CodingKey {
        case windowId = "window_id"
        case label
        case durationHours = "duration_hours"
        case rolling
        case unit
        case usedValue = "used_value"
        case limitValue = "limit_value"
        case usedPercent = "used_percent"
        case warningPercent = "warning_percent"
        case criticalPercent = "critical_percent"
        case burnRatePerHour = "burn_rate_per_hour"
        case projectedExhaustionHours = "projected_exhaustion_hours"
        case resetsAtMs = "resets_at_ms"
        case resetsInSeconds = "resets_in_seconds"
        case source
        case alertLevel = "alert_level"
    }
}

public struct SubscriptionPlanDTO: Codable, Equatable, Identifiable, Sendable {
    public let planId: String
    public let providerId: String
    public let planName: String
    public let enabled: Bool
    public let monthlyFeeMicroUsd: Int64
    public let windows: [SubscriptionWindowRuleDTO]
    public let isUserConfigured: Bool
    public let autoSource: String?
    public let updatedAtMs: Int64

    public var id: String { planId }

    enum CodingKeys: String, CodingKey {
        case planId = "plan_id"
        case providerId = "provider_id"
        case planName = "plan_name"
        case enabled
        case monthlyFeeMicroUsd = "monthly_fee_micro_usd"
        case windows
        case isUserConfigured = "is_user_configured"
        case autoSource = "auto_source"
        case updatedAtMs = "updated_at_ms"
    }

    public init(
        planId: String,
        providerId: String,
        planName: String,
        enabled: Bool = true,
        monthlyFeeMicroUsd: Int64,
        windows: [SubscriptionWindowRuleDTO],
        isUserConfigured: Bool = true,
        autoSource: String? = nil,
        updatedAtMs: Int64
    ) {
        self.planId = planId
        self.providerId = providerId
        self.planName = planName
        self.enabled = enabled
        self.monthlyFeeMicroUsd = monthlyFeeMicroUsd
        self.windows = windows
        self.isUserConfigured = isUserConfigured
        self.autoSource = autoSource
        self.updatedAtMs = updatedAtMs
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.planId = try container.decode(String.self, forKey: .planId)
        self.providerId = try container.decode(String.self, forKey: .providerId)
        self.planName = try container.decode(String.self, forKey: .planName)
        self.enabled = try container.decode(Bool.self, forKey: .enabled)
        self.monthlyFeeMicroUsd = try container.decode(Int64.self, forKey: .monthlyFeeMicroUsd)
        self.windows = try container.decode([SubscriptionWindowRuleDTO].self, forKey: .windows)
        self.isUserConfigured = try container.decodeIfPresent(Bool.self, forKey: .isUserConfigured) ?? false
        self.autoSource = try container.decodeIfPresent(String.self, forKey: .autoSource)
        self.updatedAtMs = try container.decode(Int64.self, forKey: .updatedAtMs)
    }
}

public struct SubscriptionPlanTemplateDTO: Codable, Equatable, Identifiable, Sendable {
    public let templateId: String
    public let providerId: String
    public let planName: String
    public let monthlyFeeMicroUsd: Int64
    public let windows: [SubscriptionWindowRuleDTO]
    public let description: String
    public let liveWindows: [SubscriptionWindowStatusDTO]

    public var id: String { templateId }

    enum CodingKeys: String, CodingKey {
        case templateId = "template_id"
        case providerId = "provider_id"
        case planName = "plan_name"
        case monthlyFeeMicroUsd = "monthly_fee_micro_usd"
        case windows
        case description
        case liveWindows = "live_windows"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.templateId = try container.decode(String.self, forKey: .templateId)
        self.providerId = try container.decode(String.self, forKey: .providerId)
        self.planName = try container.decode(String.self, forKey: .planName)
        self.monthlyFeeMicroUsd = try container.decode(Int64.self, forKey: .monthlyFeeMicroUsd)
        self.windows = try container.decode([SubscriptionWindowRuleDTO].self, forKey: .windows)
        self.description = try container.decode(String.self, forKey: .description)
        self.liveWindows = try container.decodeIfPresent([SubscriptionWindowStatusDTO].self, forKey: .liveWindows) ?? []
    }
}

public struct SubscriptionPlanStatusDTO: Codable, Equatable, Identifiable, Sendable {
    public let plan: SubscriptionPlanDTO
    public let windows: [SubscriptionWindowStatusDTO]
    public let todayCostMicroUsd: Int64
    public let weekCostMicroUsd: Int64
    public let monthCostMicroUsd: Int64
    public let monthTokens: Int64

    public var id: String { plan.planId }

    enum CodingKeys: String, CodingKey {
        case plan
        case windows
        case todayCostMicroUsd = "today_cost_micro_usd"
        case weekCostMicroUsd = "week_cost_micro_usd"
        case monthCostMicroUsd = "month_cost_micro_usd"
        case monthTokens = "month_tokens"
    }
}

public struct ProviderBalanceSnapshotDTO: Codable, Equatable, Sendable {
    public let providerId: String
    public let currency: String
    public let totalBalanceMicro: Int64?
    public let grantedBalanceMicro: Int64?
    public let toppedUpBalanceMicro: Int64?
    public let dailyQuotaUsedMicro: Int64?
    public let dailyQuotaLimitMicro: Int64?
    public let fetchedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case providerId = "provider_id"
        case currency
        case totalBalanceMicro = "total_balance_micro"
        case grantedBalanceMicro = "granted_balance_micro"
        case toppedUpBalanceMicro = "topped_up_balance_micro"
        case dailyQuotaUsedMicro = "daily_quota_used_micro"
        case dailyQuotaLimitMicro = "daily_quota_limit_micro"
        case fetchedAtMs = "fetched_at_ms"
    }
}

public struct PaygAccountDTO: Codable, Equatable, Identifiable, Sendable {
    public let accountId: String
    public let providerId: String
    public let accountName: String
    public let enabled: Bool
    public let manualBalanceMicroUsd: Int64?
    public let balanceAnchorMs: Int64?
    public let monthlyBudgetMicroUsd: Int64?
    public let isAuto: Bool
    public let updatedAtMs: Int64

    public var id: String { accountId }

    enum CodingKeys: String, CodingKey {
        case accountId = "account_id"
        case providerId = "provider_id"
        case accountName = "account_name"
        case enabled
        case manualBalanceMicroUsd = "manual_balance_micro_usd"
        case balanceAnchorMs = "balance_anchor_ms"
        case monthlyBudgetMicroUsd = "monthly_budget_micro_usd"
        case isAuto = "is_auto"
        case updatedAtMs = "updated_at_ms"
    }

    public init(
        accountId: String,
        providerId: String,
        accountName: String,
        enabled: Bool = true,
        manualBalanceMicroUsd: Int64? = nil,
        balanceAnchorMs: Int64? = nil,
        monthlyBudgetMicroUsd: Int64? = nil,
        isAuto: Bool = false,
        updatedAtMs: Int64
    ) {
        self.accountId = accountId
        self.providerId = providerId
        self.accountName = accountName
        self.enabled = enabled
        self.manualBalanceMicroUsd = manualBalanceMicroUsd
        self.balanceAnchorMs = balanceAnchorMs
        self.monthlyBudgetMicroUsd = monthlyBudgetMicroUsd
        self.isAuto = isAuto
        self.updatedAtMs = updatedAtMs
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.accountId = try container.decode(String.self, forKey: .accountId)
        self.providerId = try container.decode(String.self, forKey: .providerId)
        self.accountName = try container.decode(String.self, forKey: .accountName)
        self.enabled = try container.decode(Bool.self, forKey: .enabled)
        self.manualBalanceMicroUsd = try container.decodeIfPresent(Int64.self, forKey: .manualBalanceMicroUsd)
        self.balanceAnchorMs = try container.decodeIfPresent(Int64.self, forKey: .balanceAnchorMs)
        self.monthlyBudgetMicroUsd = try container.decodeIfPresent(Int64.self, forKey: .monthlyBudgetMicroUsd)
        self.isAuto = try container.decodeIfPresent(Bool.self, forKey: .isAuto) ?? false
        self.updatedAtMs = try container.decode(Int64.self, forKey: .updatedAtMs)
    }
}

public struct PaygAccountStatusDTO: Codable, Equatable, Identifiable, Sendable {
    public let account: PaygAccountDTO
    public let officialBalance: ProviderBalanceSnapshotDTO?
    public let remainingManualBalanceMicroUsd: Int64?
    public let todayCostMicroUsd: Int64
    public let monthCostMicroUsd: Int64
    public let budgetUsedPercent: Double?

    public var id: String { account.accountId }

    enum CodingKeys: String, CodingKey {
        case account
        case officialBalance = "official_balance"
        case remainingManualBalanceMicroUsd = "remaining_manual_balance_micro_usd"
        case todayCostMicroUsd = "today_cost_micro_usd"
        case monthCostMicroUsd = "month_cost_micro_usd"
        case budgetUsedPercent = "budget_used_percent"
    }
}

public struct OfficialBalanceApplyResultDTO: Codable, Equatable, Sendable {
    public let balance: ProviderBalanceSnapshotDTO
    public let observedWindowsUpdated: Int

    enum CodingKeys: String, CodingKey {
        case balance
        case observedWindowsUpdated = "observed_windows_updated"
    }
}

public struct CloudflareWorkerUploadBundleDTO: Codable, Equatable, Sendable {
    public let workerName: String
    public let bearerToken: String
    public let e2eeEnabled: Bool
    public let e2eePassphrase: String?
    public let contentType: String
    public let multipartBodyBase64: String

    enum CodingKeys: String, CodingKey {
        case workerName = "worker_name"
        case bearerToken = "bearer_token"
        case e2eeEnabled = "e2ee_enabled"
        case e2eePassphrase = "e2ee_passphrase"
        case contentType = "content_type"
        case multipartBodyBase64 = "multipart_body_base64"
    }
}
