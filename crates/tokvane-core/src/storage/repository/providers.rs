//! Provider configs, credential flags, balances and custom provider rules.

use super::*;

impl Storage {
    pub fn list_provider_configs(&self) -> CoreResult<Vec<ProviderConfig>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT provider_id, enabled, collection_mode, custom_log_paths_json,
                       has_credential, last_sync_at_ms, last_error, updated_at_ms
                FROM provider_configs
                ORDER BY provider_id ASC
                "#,
            )?;
            let rows = stmt.query_map([], |row| {
                let mode_str: String = row.get(2)?;
                let paths_json: String = row.get(3)?;
                let paths = serde_json::from_str::<Vec<String>>(&paths_json).unwrap_or_default();
                Ok(ProviderConfig {
                    provider_id: row.get(0)?,
                    enabled: row.get::<_, i64>(1)? != 0,
                    collection_mode: CollectionMode::parse(&mode_str),
                    custom_log_paths: paths,
                    has_credential: row.get::<_, i64>(4)? != 0,
                    last_sync_at_ms: row.get(5)?,
                    last_error: row.get(6)?,
                    updated_at_ms: row.get(7)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn upsert_provider_config(&self, config: &ProviderConfig) -> CoreResult<()> {
        let now_ms = Utc::now().timestamp_millis();
        let paths_json = serde_json::to_string(&config.custom_log_paths)?;
        self.with_conn(|conn| {
            conn.execute(
                r#"
                INSERT INTO provider_configs
                    (provider_id, enabled, collection_mode, custom_log_paths_json, has_credential, last_sync_at_ms, last_error, updated_at_ms)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                ON CONFLICT(provider_id) DO UPDATE SET
                    enabled = excluded.enabled,
                    collection_mode = excluded.collection_mode,
                    custom_log_paths_json = excluded.custom_log_paths_json,
                    has_credential = excluded.has_credential,
                    last_sync_at_ms = COALESCE(excluded.last_sync_at_ms, provider_configs.last_sync_at_ms),
                    last_error = excluded.last_error,
                    updated_at_ms = excluded.updated_at_ms
                "#,
                params![
                    config.provider_id,
                    i64::from(config.enabled),
                    config.collection_mode.as_str(),
                    paths_json,
                    i64::from(config.has_credential),
                    config.last_sync_at_ms,
                    config.last_error,
                    now_ms,
                ],
            )?;
            Ok(())
        })
    }

    pub fn set_provider_sync_status(
        &self,
        provider_id: &str,
        last_sync_at_ms: Option<i64>,
        last_error: Option<&str>,
    ) -> CoreResult<()> {
        let now_ms = Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            conn.execute(
                r#"
                UPDATE provider_configs
                SET last_sync_at_ms = COALESCE(?2, last_sync_at_ms),
                    last_error = ?3,
                    updated_at_ms = ?4
                WHERE provider_id = ?1
                "#,
                params![provider_id, last_sync_at_ms, last_error, now_ms],
            )?;
            Ok(())
        })
    }

    pub fn get_cli_auth_paused_reason(&self, provider_id: &str) -> CoreResult<Option<String>> {
        self.with_conn(|conn| {
            let raw: Option<Option<String>> = conn
                .query_row(
                    "SELECT cli_auth_paused_reason FROM provider_configs WHERE provider_id = ?1",
                    params![provider_id],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(raw
                .flatten()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()))
        })
    }

    pub fn set_cli_auth_paused_reason(
        &self,
        provider_id: &str,
        reason: Option<&str>,
    ) -> CoreResult<()> {
        let now_ms = Utc::now().timestamp_millis();
        let cleaned = reason.map(str::trim).filter(|s| !s.is_empty());
        self.with_conn(|conn| {
            conn.execute(
                r#"
                UPDATE provider_configs
                SET cli_auth_paused_reason = ?2,
                    updated_at_ms = ?3
                WHERE provider_id = ?1
                "#,
                params![provider_id, cleaned, now_ms],
            )?;
            Ok(())
        })
    }

    pub fn set_provider_has_credential(
        &self,
        provider_id: &str,
        has_credential: bool,
    ) -> CoreResult<()> {
        let now_ms = Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            conn.execute(
                r#"
                UPDATE provider_configs
                SET has_credential = ?2, updated_at_ms = ?3
                WHERE provider_id = ?1
                "#,
                params![provider_id, i64::from(has_credential), now_ms],
            )?;
            Ok(())
        })
    }

    pub fn clear_provider_api_artifacts(&self, provider_id: &str) -> CoreResult<()> {
        self.with_conn_mut(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "DELETE FROM provider_balances WHERE provider_id = ?1",
                params![provider_id],
            )?;
            tx.execute(
                "DELETE FROM observed_rate_limits WHERE provider_id = ?1 AND source = 'official_api'",
                params![provider_id],
            )?;
            tx.execute(
                "DELETE FROM subscription_plans WHERE provider_id = ?1 AND auto_source = 'official_api' AND user_customized = 0",
                params![provider_id],
            )?;
            tx.execute(
                "DELETE FROM payg_accounts WHERE provider_id = ?1 AND is_auto = 1",
                params![provider_id],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn list_provider_balances(&self) -> CoreResult<Vec<ProviderBalanceSnapshot>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT provider_id, currency, total_balance_micro, granted_balance_micro,
                       topped_up_balance_micro, daily_quota_used_micro, daily_quota_limit_micro,
                       fetched_at_ms
                FROM provider_balances
                ORDER BY provider_id ASC
                "#,
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(ProviderBalanceSnapshot {
                    provider_id: row.get(0)?,
                    currency: row.get(1)?,
                    total_balance_micro: row.get(2)?,
                    granted_balance_micro: row.get(3)?,
                    topped_up_balance_micro: row.get(4)?,
                    daily_quota_used_micro: row.get(5)?,
                    daily_quota_limit_micro: row.get(6)?,
                    fetched_at_ms: row.get(7)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn get_provider_balance(
        &self,
        provider_id: &str,
    ) -> CoreResult<Option<ProviderBalanceSnapshot>> {
        self.with_conn(|conn| {
            conn.query_row(
                r#"
                SELECT provider_id, currency, total_balance_micro, granted_balance_micro,
                       topped_up_balance_micro, daily_quota_used_micro, daily_quota_limit_micro,
                       fetched_at_ms
                FROM provider_balances
                WHERE provider_id = ?1
                "#,
                params![provider_id],
                |row| {
                    Ok(ProviderBalanceSnapshot {
                        provider_id: row.get(0)?,
                        currency: row.get(1)?,
                        total_balance_micro: row.get(2)?,
                        granted_balance_micro: row.get(3)?,
                        topped_up_balance_micro: row.get(4)?,
                        daily_quota_used_micro: row.get(5)?,
                        daily_quota_limit_micro: row.get(6)?,
                        fetched_at_ms: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(CoreError::from)
        })
    }

    pub fn apply_official_balance_payload(
        &self,
        provider_id: &str,
        endpoint: &str,
        body_json: &str,
        now_ms: i64,
    ) -> CoreResult<crate::pricing::OfficialBalanceApplyResult> {
        let parsed: serde_json::Value = serde_json::from_str(body_json)?;
        let (snapshot, rate_limits) = crate::pricing::parse_official_balance_payload(
            provider_id,
            endpoint,
            &parsed,
            now_ms,
        )?;
        self.upsert_provider_balance(&snapshot)?;
        let observed_windows_updated = rate_limits.len();
        if !rate_limits.is_empty() {
            self.upsert_observed_rate_limits(&rate_limits)?;
        }
        Ok(crate::pricing::OfficialBalanceApplyResult {
            balance: snapshot,
            observed_windows_updated,
        })
    }

    pub fn upsert_provider_balance(&self, balance: &ProviderBalanceSnapshot) -> CoreResult<()> {
        self.with_conn_mut(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                r#"
                INSERT INTO provider_balances (
                    provider_id, currency, total_balance_micro, granted_balance_micro,
                    topped_up_balance_micro, daily_quota_used_micro, daily_quota_limit_micro,
                    fetched_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                ON CONFLICT(provider_id) DO UPDATE SET
                    currency = excluded.currency,
                    total_balance_micro = excluded.total_balance_micro,
                    granted_balance_micro = excluded.granted_balance_micro,
                    topped_up_balance_micro = excluded.topped_up_balance_micro,
                    daily_quota_used_micro = excluded.daily_quota_used_micro,
                    daily_quota_limit_micro = excluded.daily_quota_limit_micro,
                    fetched_at_ms = excluded.fetched_at_ms
                "#,
                params![
                    balance.provider_id,
                    balance.currency,
                    balance.total_balance_micro,
                    balance.granted_balance_micro,
                    balance.topped_up_balance_micro,
                    balance.daily_quota_used_micro,
                    balance.daily_quota_limit_micro,
                    balance.fetched_at_ms,
                ],
            )?;
            sync_auto_payg_for_balance_conn(&tx, balance)?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn upsert_provider_balance_if_newer(
        &self,
        balance: &ProviderBalanceSnapshot,
    ) -> CoreResult<()> {
        self.with_conn_mut(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                r#"
                INSERT INTO provider_balances (
                    provider_id, currency, total_balance_micro, granted_balance_micro,
                    topped_up_balance_micro, daily_quota_used_micro, daily_quota_limit_micro,
                    fetched_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                ON CONFLICT(provider_id) DO UPDATE SET
                    currency = excluded.currency,
                    total_balance_micro = excluded.total_balance_micro,
                    granted_balance_micro = excluded.granted_balance_micro,
                    topped_up_balance_micro = excluded.topped_up_balance_micro,
                    daily_quota_used_micro = excluded.daily_quota_used_micro,
                    daily_quota_limit_micro = excluded.daily_quota_limit_micro,
                    fetched_at_ms = excluded.fetched_at_ms
                WHERE excluded.fetched_at_ms >= provider_balances.fetched_at_ms
                "#,
                params![
                    balance.provider_id,
                    balance.currency,
                    balance.total_balance_micro,
                    balance.granted_balance_micro,
                    balance.topped_up_balance_micro,
                    balance.daily_quota_used_micro,
                    balance.daily_quota_limit_micro,
                    balance.fetched_at_ms,
                ],
            )?;
            sync_auto_payg_for_balance_conn(&tx, balance)?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn list_custom_provider_rules(&self) -> CoreResult<Vec<CustomProviderRule>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT rule_id, display_name, enabled, protocol_kind, log_roots_json, mapping_spec_json, updated_at_ms
                FROM custom_provider_rules
                ORDER BY display_name ASC, rule_id ASC
                "#,
            )?;
            let rows = stmt.query_map([], |row| {
                let kind_str: String = row.get(3)?;
                let roots_json: String = row.get(4)?;
                let spec_json: String = row.get(5)?;
                let log_roots = serde_json::from_str::<Vec<String>>(&roots_json).unwrap_or_default();
                let mapping_spec =
                    serde_json::from_str::<DeclarativeMappingSpec>(&spec_json).unwrap_or_default();
                Ok(CustomProviderRule {
                    rule_id: row.get(0)?,
                    display_name: row.get(1)?,
                    enabled: row.get::<_, i64>(2)? != 0,
                    protocol_kind: CustomProtocolKind::parse(&kind_str),
                    log_roots,
                    mapping_spec,
                    updated_at_ms: row.get(6)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn upsert_custom_provider_rule(&self, rule: &CustomProviderRule) -> CoreResult<()> {
        let trimmed_id = rule.rule_id.trim();
        let trimmed_name = rule.display_name.trim();
        if trimmed_id.is_empty() {
            return Err(CoreError::Validation(
                "Custom provider rule_id cannot be empty".to_string(),
            ));
        }
        if trimmed_name.is_empty() {
            return Err(CoreError::Validation(
                "Custom provider display_name cannot be empty".to_string(),
            ));
        }
        let now_ms = Utc::now().timestamp_millis();
        let roots_json = serde_json::to_string(&rule.log_roots)?;
        let spec_json = serde_json::to_string(&rule.mapping_spec)?;
        self.with_conn(|conn| {
            conn.execute(
                r#"
                INSERT INTO custom_provider_rules (
                    rule_id, display_name, enabled, protocol_kind, log_roots_json, mapping_spec_json, updated_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ON CONFLICT(rule_id) DO UPDATE SET
                    display_name = excluded.display_name,
                    enabled = excluded.enabled,
                    protocol_kind = excluded.protocol_kind,
                    log_roots_json = excluded.log_roots_json,
                    mapping_spec_json = excluded.mapping_spec_json,
                    updated_at_ms = excluded.updated_at_ms
                "#,
                params![
                    trimmed_id,
                    trimmed_name,
                    i64::from(rule.enabled),
                    rule.protocol_kind.as_str(),
                    roots_json,
                    spec_json,
                    now_ms,
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete_custom_provider_rule(&self, rule_id: &str) -> CoreResult<()> {
        let trimmed = rule_id.trim();
        if trimmed.is_empty() {
            return Err(CoreError::Validation(
                "Custom provider rule_id cannot be empty".to_string(),
            ));
        }
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM custom_provider_rules WHERE rule_id = ?1",
                params![trimmed],
            )?;
            Ok(())
        })
    }
}
