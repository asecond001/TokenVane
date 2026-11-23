//! Activity-triggered official quota refresh scheduling.
//!
//! When the log watcher ingests new usage events for a provider whose quota is only available
//! through an official CLI (`agy`, `claude`) or official quota/balance API (Kimi Code, MiniMax,
//! QCode, DeepSeek), the desktop shell asks this throttle whether to refresh now. Each provider is
//! refreshed at most once per [`ACTIVITY_REFRESH_MIN_INTERVAL_MS`]; activity arriving inside the
//! interval is coalesced into one trailing refresh so the final reading after a burst is captured.
//! Providers whose quota already streams through local logs (Codex `rate_limits`) are excluded.

use std::collections::{HashMap, HashSet};

use tokvane_core::domain::provider_ids;

/// Minimum spacing between two activity-triggered refreshes for API-key providers.
pub const ACTIVITY_REFRESH_MIN_INTERVAL_MS: i64 = 60_000;
/// Longer minimum spacing for official CLI subprocess probes (opt-in background polling).
pub const CLI_ACTIVITY_REFRESH_MIN_INTERVAL_MS: i64 = 300_000;

/// How a provider's quota can be refreshed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaRefreshKind {
    /// Official public CLI subprocess (no API key required).
    OfficialCli,
    /// Official public quota/balance REST API (requires a stored API key).
    OfficialApiKey,
}

/// Classify providers eligible for activity-triggered quota refresh.
pub fn activity_refresh_kind(provider_id: &str) -> Option<QuotaRefreshKind> {
    match provider_id {
        provider_ids::ANTIGRAVITY | provider_ids::CLAUDE_CODE => Some(QuotaRefreshKind::OfficialCli),
        provider_ids::KIMI_CODE
        | provider_ids::MINIMAX_CODE
        | provider_ids::QCODE
        | provider_ids::DEEPSEEK_HARNESS => Some(QuotaRefreshKind::OfficialApiKey),
        _ => None,
    }
}

/// Minimum activity-refresh spacing for `provider_id`.
pub fn min_interval_ms_for_provider(provider_id: &str) -> i64 {
    match activity_refresh_kind(provider_id) {
        Some(QuotaRefreshKind::OfficialCli) => CLI_ACTIVITY_REFRESH_MIN_INTERVAL_MS,
        _ => ACTIVITY_REFRESH_MIN_INTERVAL_MS,
    }
}

/// Per-provider leading-edge + trailing-edge throttle.
#[derive(Debug, Default)]
pub struct ActivityRefreshThrottle {
    last_started_ms: HashMap<String, i64>,
    pending: HashSet<String>,
    in_flight: HashSet<String>,
}

impl ActivityRefreshThrottle {
    pub fn new() -> Self {
        Self::default()
    }

    fn interval_elapsed(&self, provider_id: &str, now_ms: i64) -> bool {
        let min_interval = min_interval_ms_for_provider(provider_id);
        self.last_started_ms
            .get(provider_id)
            .is_none_or(|last| now_ms.saturating_sub(*last) >= min_interval)
    }

    /// Record new activity. Returns `true` when a refresh should start immediately; otherwise the
    /// provider is queued for a trailing refresh returned later by [`Self::take_due`].
    pub fn on_activity(&mut self, provider_id: &str, now_ms: i64) -> bool {
        if !self.in_flight.contains(provider_id) && self.interval_elapsed(provider_id, now_ms) {
            self.pending.remove(provider_id);
            self.last_started_ms.insert(provider_id.to_string(), now_ms);
            self.in_flight.insert(provider_id.to_string());
            return true;
        }
        self.pending.insert(provider_id.to_string());
        false
    }

    /// Drain queued providers whose interval has elapsed and that are not currently refreshing.
    /// Returned providers are marked as started.
    pub fn take_due(&mut self, now_ms: i64) -> Vec<String> {
        let mut due: Vec<String> = self
            .pending
            .iter()
            .filter(|pid| !self.in_flight.contains(*pid) && self.interval_elapsed(pid, now_ms))
            .cloned()
            .collect();
        due.sort();
        for pid in &due {
            self.pending.remove(pid);
            self.last_started_ms.insert(pid.clone(), now_ms);
            self.in_flight.insert(pid.clone());
        }
        due
    }

    /// Mark a refresh as finished so later activity can schedule the next one.
    pub fn finish(&mut self, provider_id: &str) {
        self.in_flight.remove(provider_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligible_providers_exclude_log_streamed_quota() {
        assert_eq!(
            activity_refresh_kind(provider_ids::CLAUDE_CODE),
            Some(QuotaRefreshKind::OfficialCli)
        );
        assert_eq!(
            activity_refresh_kind(provider_ids::ANTIGRAVITY),
            Some(QuotaRefreshKind::OfficialCli)
        );
        for pid in [
            provider_ids::KIMI_CODE,
            provider_ids::MINIMAX_CODE,
            provider_ids::QCODE,
            provider_ids::DEEPSEEK_HARNESS,
        ] {
            assert_eq!(activity_refresh_kind(pid), Some(QuotaRefreshKind::OfficialApiKey));
        }
        assert_eq!(activity_refresh_kind(provider_ids::CODEX), None);
        assert_eq!(activity_refresh_kind(provider_ids::WORKBUDDY), None);
    }

    #[test]
    fn leading_edge_then_single_trailing_refresh() {
        let mut t = ActivityRefreshThrottle::new();
        let pid = provider_ids::KIMI_CODE;

        assert!(t.on_activity(pid, 0), "first activity starts immediately");
        assert!(!t.on_activity(pid, 1_000), "in-flight activity is queued");
        t.finish(pid);
        assert!(!t.on_activity(pid, 20_000), "inside interval is queued");
        assert!(!t.on_activity(pid, 30_000));
        assert!(t.take_due(59_999).is_empty(), "trailing waits for the 60s API interval");
        assert_eq!(t.take_due(60_000), vec![pid.to_string()]);
        assert!(t.take_due(200_000).is_empty(), "burst coalesces into one trailing refresh");

        let cli_pid = provider_ids::ANTIGRAVITY;
        assert!(t.on_activity(cli_pid, 0), "first CLI activity starts immediately");
        t.finish(cli_pid);
        assert!(!t.on_activity(cli_pid, 60_000), "CLI cooldown is 300s");
        assert!(t.take_due(299_999).is_empty(), "CLI trailing waits for 300s");
        assert_eq!(t.take_due(300_000), vec![cli_pid.to_string()]);
    }

    #[test]
    fn in_flight_refresh_delays_trailing_until_finished() {
        let mut t = ActivityRefreshThrottle::new();
        let pid = provider_ids::ANTIGRAVITY;
        assert!(t.on_activity(pid, 0));
        assert!(!t.on_activity(pid, 10_000));
        assert!(t.take_due(310_000).is_empty(), "still running");
        t.finish(pid);
        assert_eq!(t.take_due(310_000), vec![pid.to_string()]);
        t.finish(pid);
        assert!(t.on_activity(pid, 610_000), "next 300s interval starts immediately again");
    }

    #[test]
    fn providers_are_throttled_independently() {
        let mut t = ActivityRefreshThrottle::new();
        assert!(t.on_activity(provider_ids::KIMI_CODE, 0));
        assert!(t.on_activity(provider_ids::QCODE, 1_000));
        assert!(!t.on_activity(provider_ids::KIMI_CODE, 2_000));
    }
}
