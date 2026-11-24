//! Subscription plans, observed rate-limit windows and pay-as-you-go accounts.

use super::*;

impl Storage {
    pub fn list_observed_rate_limits(&self) -> CoreResult<Vec<ObservedRateLimitWindow>> {
        self.with_conn(|conn| {
            Ok(load_observed_rate_limits(conn)?
                .into_iter()
                .filter(|o| {
                    o.source.as_deref() != Some("local_log_hint") && o.window_name != "tier_hint"
                })
                .collect())
        })
    }

    /// Replace the synced `AI Subscription Catalog` template table atomically and refresh any
    /// untouched auto-discovered subscription rows if their catalog tier name or fee changed.
    /// Returns `(catalog_updated, synced_plans_count)`.
    pub fn apply_synced_subscription_catalog(
        &self,
        remote_templates: &[SubscriptionPlanTemplate],
        synced_at_ms: i64,
    ) -> CoreResult<(bool, usize)> {
        if remote_templates.is_empty() {
            return Err(CoreError::Validation(
                "Synced subscription catalog cannot be empty".to_string(),
            ));
        }

        let new_hash = compute_subscription_catalog_hash(remote_templates);
        let plan_count = remote_templates.len();

        self.with_conn_mut(|conn| {
            let existing_hash: Option<String> = conn
                .query_row(
                    "SELECT subscription_catalog_hash FROM app_preferences WHERE id = 1",
                    [],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            let existing_rows: i64 =
                conn.query_row("SELECT COUNT(*) FROM subscription_catalog", [], |r| {
                    r.get(0)
                })?;

            if existing_hash.as_deref() == Some(new_hash.as_str()) && existing_rows > 0 {
                conn.execute(
                    "UPDATE app_preferences SET last_subscription_sync_at_ms = ?1, updated_at_ms = ?1 WHERE id = 1",
                    params![synced_at_ms],
                )?;
                return Ok((false, existing_rows as usize));
            }

            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute("DELETE FROM subscription_catalog", [])?;

            {
                let mut stmt = tx.prepare(
                    r#"
                    INSERT INTO subscription_catalog (
                        template_id, provider_id, plan_name, monthly_fee_micro_usd,
                        windows_json, description, sort_order, updated_at_ms
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                    "#,
                )?;

                for (idx, tpl) in remote_templates.iter().enumerate() {
                    let windows_json = serde_json::to_string(&tpl.windows)?;
                    stmt.execute(params![
                        tpl.template_id,
                        tpl.provider_id,
                        tpl.plan_name,
                        tpl.monthly_fee_micro_usd,
                        windows_json,
                        tpl.description,
                        idx as i64,
                        synced_at_ms,
                    ])?;
                }
            }

            tx.execute(
                r#"
                UPDATE app_preferences
                SET last_subscription_sync_at_ms = ?1,
                    subscription_catalog_hash = ?2,
                    updated_at_ms = ?1
                WHERE id = 1
                "#,
                params![synced_at_ms, new_hash],
            )?;

            let auto_providers: Vec<String> = {
                let mut stmt = tx.prepare(
                    r#"
                    SELECT DISTINCT provider_id
                    FROM subscription_plans
                    WHERE (plan_id LIKE 'auto_%' OR auto_source IS NOT NULL)
                      AND user_customized = 0
                      AND user_dismissed = 0
                    "#,
                )?;
                let mapped = stmt.query_map([], |row| row.get::<_, String>(0))?;
                mapped.collect::<Result<Vec<_>, _>>()?
            };

            for pid in auto_providers {
                sync_auto_subscription_for_provider_conn(&tx, &pid, synced_at_ms)?;
            }

            tx.commit()?;
            Ok((true, plan_count))
        })
    }

    pub fn list_subscription_plans(&self) -> CoreResult<Vec<SubscriptionPlan>> {
        self.with_conn(load_subscription_plans)
    }

    pub fn upsert_subscription_plan(&self, plan: &SubscriptionPlan) -> CoreResult<()> {
        let now_ms = Utc::now().timestamp_millis();
        let windows_json = serde_json::to_string(&plan.windows)?;
        let is_auto = plan.auto_source.is_some();
        self.with_conn_mut(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if !is_auto {
                tx.execute(
                    r#"
                    DELETE FROM subscription_plans
                    WHERE provider_id = ?1
                      AND plan_id != ?2
                      AND (plan_id LIKE 'auto_%' OR auto_source IS NOT NULL)
                      AND user_customized = 0
                    "#,
                    params![plan.provider_id, plan.plan_id],
                )?;
            }
            tx.execute(
                r#"
                INSERT INTO subscription_plans
                    (plan_id, provider_id, plan_name, enabled, monthly_fee_micro_usd, windows_json, is_user_configured, auto_source, user_customized, user_dismissed, updated_at_ms)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, 1, 0, ?8)
                ON CONFLICT(plan_id) DO UPDATE SET
                    provider_id = excluded.provider_id,
                    plan_name = excluded.plan_name,
                    enabled = excluded.enabled,
                    monthly_fee_micro_usd = excluded.monthly_fee_micro_usd,
                    windows_json = excluded.windows_json,
                    is_user_configured = 1,
                    auto_source = excluded.auto_source,
                    user_customized = 1,
                    user_dismissed = 0,
                    updated_at_ms = excluded.updated_at_ms
                "#,
                params![
                    plan.plan_id,
                    plan.provider_id,
                    plan.plan_name,
                    i64::from(plan.enabled),
                    plan.monthly_fee_micro_usd,
                    windows_json,
                    plan.auto_source,
                    now_ms,
                ],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Flip a plan's `enabled` flag only. Unlike [`Self::upsert_subscription_plan`] this does not
    /// mark an auto-discovered plan as user-customized, so its tier name, fee and windows keep
    /// following official observations after the user pauses or resumes it.
    pub fn set_subscription_plan_enabled(&self, plan_id: &str, enabled: bool) -> CoreResult<()> {
        let trimmed = plan_id.trim();
        if trimmed.is_empty() {
            return Err(CoreError::Validation("plan_id cannot be empty".to_string()));
        }
        let now_ms = Utc::now().timestamp_millis();
        self.with_conn_mut(|conn| {
            let changed = conn.execute(
                "UPDATE subscription_plans SET enabled = ?2, updated_at_ms = ?3 WHERE plan_id = ?1",
                params![trimmed, i64::from(enabled), now_ms],
            )?;
            if changed == 0 {
                return Err(CoreError::Validation(format!(
                    "subscription plan {trimmed} not found"
                )));
            }
            Ok(())
        })
    }

    pub fn delete_subscription_plan(&self, plan_id: &str) -> CoreResult<()> {
        let trimmed = plan_id.trim();
        if trimmed.is_empty() {
            return Err(CoreError::Validation(
                "Subscription plan_id cannot be empty".to_string(),
            ));
        }
        let now_ms = Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            let auto_source: Option<Option<String>> = conn
                .query_row(
                    "SELECT auto_source FROM subscription_plans WHERE plan_id = ?1",
                    params![trimmed],
                    |row| row.get(0),
                )
                .optional()?;

            if auto_source.flatten().is_some() || trimmed.starts_with("auto_") {
                conn.execute(
                    r#"
                    UPDATE subscription_plans
                    SET enabled = 0, is_user_configured = 0, user_dismissed = 1, updated_at_ms = ?2
                    WHERE plan_id = ?1
                    "#,
                    params![trimmed, now_ms],
                )?;
            } else {
                conn.execute(
                    "DELETE FROM subscription_plans WHERE plan_id = ?1",
                    params![trimmed],
                )?;
            }
            Ok(())
        })
    }

    pub fn reset_auto_subscription_dismissal(&self, provider_id: &str) -> CoreResult<()> {
        let now_ms = Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            conn.execute(
                r#"
                UPDATE subscription_plans
                SET user_dismissed = 0, updated_at_ms = ?2
                WHERE provider_id = ?1 AND (plan_id LIKE 'auto_%' OR auto_source IS NOT NULL)
                "#,
                params![provider_id, now_ms],
            )?;
            conn.execute(
                r#"
                UPDATE payg_accounts
                SET user_dismissed = 0, enabled = 1, updated_at_ms = ?2
                WHERE provider_id = ?1 AND is_auto = 1
                "#,
                params![provider_id, now_ms],
            )?;
            Ok(())
        })
    }

    pub fn upsert_payg_account(&self, account: &PaygAccount) -> CoreResult<()> {
        let account_id = account.account_id.trim();
        let provider_id = account.provider_id.trim();
        let account_name = account.account_name.trim();
        if account_id.is_empty() || provider_id.is_empty() || account_name.is_empty() {
            return Err(CoreError::Validation(
                "PAYG account_id, provider_id, and account_name cannot be empty".to_string(),
            ));
        }
        let now_ms = Utc::now().timestamp_millis();
        let updated_at_ms = if account.updated_at_ms > 0 {
            account.updated_at_ms
        } else {
            now_ms
        };
        let is_auto = account.is_auto || account_id.starts_with("auto_");
        self.with_conn_mut(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if !is_auto {
                tx.execute(
                    "DELETE FROM payg_accounts WHERE provider_id = ?1 AND account_id != ?2 AND is_auto = 1",
                    params![provider_id, account_id],
                )?;
            }
            tx.execute(
                r#"
                INSERT INTO payg_accounts (
                    account_id, provider_id, account_name, enabled,
                    manual_balance_micro_usd, balance_anchor_ms, monthly_budget_micro_usd,
                    is_auto, user_dismissed, updated_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9)
                ON CONFLICT(account_id) DO UPDATE SET
                    provider_id = excluded.provider_id,
                    account_name = excluded.account_name,
                    enabled = excluded.enabled,
                    manual_balance_micro_usd = excluded.manual_balance_micro_usd,
                    balance_anchor_ms = excluded.balance_anchor_ms,
                    monthly_budget_micro_usd = excluded.monthly_budget_micro_usd,
                    is_auto = CASE
                        WHEN payg_accounts.account_id LIKE 'auto_%' THEN payg_accounts.is_auto
                        ELSE excluded.is_auto
                    END,
                    user_dismissed = 0,
                    updated_at_ms = excluded.updated_at_ms
                "#,
                params![
                    account_id,
                    provider_id,
                    account_name,
                    i64::from(account.enabled),
                    account.manual_balance_micro_usd,
                    account.balance_anchor_ms,
                    account.monthly_budget_micro_usd,
                    i64::from(account.is_auto),
                    updated_at_ms,
                ],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn delete_payg_account(&self, account_id: &str) -> CoreResult<()> {
        let trimmed = account_id.trim();
        if trimmed.is_empty() {
            return Err(CoreError::Validation(
                "PAYG account_id cannot be empty".to_string(),
            ));
        }
        let now_ms = Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            let is_auto: Option<i64> = conn
                .query_row(
                    "SELECT is_auto FROM payg_accounts WHERE account_id = ?1",
                    params![trimmed],
                    |row| row.get(0),
                )
                .optional()?;
            if is_auto == Some(1) || trimmed.starts_with("auto_") {
                conn.execute(
                    "UPDATE payg_accounts SET enabled = 0, user_dismissed = 1, updated_at_ms = ?2 WHERE account_id = ?1",
                    params![trimmed, now_ms],
                )?;
            } else {
                conn.execute(
                    "DELETE FROM payg_accounts WHERE account_id = ?1",
                    params![trimmed],
                )?;
            }
            Ok(())
        })
    }

    pub fn query_payg_accounts(
        &self,
        now_ms: i64,
        tz_offset_minutes: i32,
    ) -> CoreResult<Vec<PaygAccountStatus>> {
        self.with_conn_mut(|conn| {
            let balances = {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT provider_id, currency, total_balance_micro, granted_balance_micro,
                           topped_up_balance_micro, daily_quota_used_micro, daily_quota_limit_micro,
                           fetched_at_ms
                    FROM provider_balances
                    WHERE UPPER(currency) != 'QUOTA'
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
                let mut map: HashMap<String, ProviderBalanceSnapshot> = HashMap::new();
                for row in rows {
                    let bal = row?;
                    sync_auto_payg_for_balance_conn(conn, &bal)?;
                    map.insert(bal.provider_id.to_ascii_lowercase(), bal);
                }
                map
            };

            let accounts = {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT account_id, provider_id, account_name, enabled,
                           manual_balance_micro_usd, balance_anchor_ms, monthly_budget_micro_usd,
                           is_auto, updated_at_ms
                    FROM payg_accounts
                    WHERE user_dismissed = 0
                    ORDER BY enabled DESC, provider_id ASC, updated_at_ms DESC
                    "#,
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok(PaygAccount {
                        account_id: row.get(0)?,
                        provider_id: row.get(1)?,
                        account_name: row.get(2)?,
                        enabled: row.get::<_, i64>(3)? != 0,
                        manual_balance_micro_usd: row.get(4)?,
                        balance_anchor_ms: row.get(5)?,
                        monthly_budget_micro_usd: row.get(6)?,
                        is_auto: row.get::<_, i64>(7)? != 0,
                        updated_at_ms: row.get(8)?,
                    })
                })?;
                rows.collect::<Result<Vec<_>, _>>()?
            };

            let (today_start_ms, month_start_ms) =
                crate::analytics::local_day_bounds_ms(now_ms, tz_offset_minutes);
            let today_bucket_start = today_start_ms.div_euclid(HOUR_MS) * HOUR_MS;
            let month_bucket_start = month_start_ms.div_euclid(HOUR_MS) * HOUR_MS;

            let mut today_costs: HashMap<String, i64> = HashMap::new();
            {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT LOWER(provider_id), COALESCE(SUM(estimated_cost_micro_usd), 0)
                    FROM usage_hourly_rollups
                    WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2
                    GROUP BY LOWER(provider_id)
                    "#,
                )?;
                let rows = stmt.query_map(params![today_bucket_start, now_ms], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })?;
                for row in rows {
                    let (pid, cost) = row?;
                    today_costs.insert(pid, cost);
                }
            }

            let mut month_costs: HashMap<String, i64> = HashMap::new();
            {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT LOWER(provider_id), COALESCE(SUM(estimated_cost_micro_usd), 0)
                    FROM usage_hourly_rollups
                    WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2
                    GROUP BY LOWER(provider_id)
                    "#,
                )?;
                let rows = stmt.query_map(params![month_bucket_start, now_ms], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })?;
                for row in rows {
                    let (pid, cost) = row?;
                    month_costs.insert(pid, cost);
                }
            }

            let has_raw_events: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM usage_events LIMIT 1)",
                [],
                |r| r.get(0),
            )?;

            let mut statuses = Vec::with_capacity(accounts.len());
            for account in accounts {
                let key = account.provider_id.to_ascii_lowercase();
                let official_balance = balances.get(&key).cloned();
                let today_cost_micro_usd = *today_costs.get(&key).unwrap_or(&0);
                let month_cost_micro_usd = *month_costs.get(&key).unwrap_or(&0);

                let remaining_manual_balance_micro_usd =
                    if let Some(initial) = account.manual_balance_micro_usd {
                        let anchor = account.balance_anchor_ms.unwrap_or(0);
                        let spent_since: i64 = if has_raw_events {
                            conn.query_row(
                                r#"
                                SELECT COALESCE(SUM(estimated_cost_micro_usd), 0)
                                FROM usage_events
                                WHERE LOWER(provider_id) = ?1 AND timestamp_ms >= ?2 AND timestamp_ms <= ?3
                                "#,
                                params![key, anchor, now_ms],
                                |r| r.get(0),
                            )?
                        } else {
                            let aligned_anchor = anchor.div_euclid(HOUR_MS) * HOUR_MS;
                            conn.query_row(
                                r#"
                                SELECT COALESCE(SUM(estimated_cost_micro_usd), 0)
                                FROM usage_hourly_rollups
                                WHERE LOWER(provider_id) = ?1 AND bucket_start_ms >= ?2 AND bucket_start_ms <= ?3
                                "#,
                                params![key, aligned_anchor, now_ms],
                                |r| r.get(0),
                            )?
                        };
                        Some(initial - spent_since)
                    } else {
                        None
                    };

                let budget_used_percent = account
                    .monthly_budget_micro_usd
                    .filter(|&budget| budget > 0)
                    .map(|budget| (month_cost_micro_usd as f64 / budget as f64) * 100.0);

                statuses.push(PaygAccountStatus {
                    account,
                    official_balance,
                    remaining_manual_balance_micro_usd,
                    today_cost_micro_usd,
                    month_cost_micro_usd,
                    budget_used_percent,
                });
            }

            Ok(statuses)
        })
    }

    pub fn upsert_observed_rate_limits(
        &self,
        rate_limits: &[ObservedRateLimitWindow],
    ) -> CoreResult<()> {
        if rate_limits.is_empty() {
            return Ok(());
        }
        let now_ms = Utc::now().timestamp_millis();
        self.with_conn_mut(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut rate_limit_providers: BTreeSet<String> = BTreeSet::new();
            for obs in rate_limits {
                rate_limit_providers.insert(obs.provider_id.clone());
                tx.execute(
                    r#"
                    INSERT INTO observed_rate_limits
                        (provider_id, window_name, used_percent, window_minutes, resets_at_ms, observed_at_ms, source, plan_hint)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                    ON CONFLICT(provider_id, window_name) DO UPDATE SET
                        used_percent = excluded.used_percent,
                        window_minutes = excluded.window_minutes,
                        resets_at_ms = excluded.resets_at_ms,
                        observed_at_ms = excluded.observed_at_ms,
                        source = COALESCE(excluded.source, observed_rate_limits.source),
                        plan_hint = COALESCE(excluded.plan_hint, observed_rate_limits.plan_hint)
                    WHERE excluded.observed_at_ms >= observed_rate_limits.observed_at_ms
                    "#,
                    params![
                        obs.provider_id,
                        obs.window_name,
                        obs.used_percent,
                        obs.window_minutes,
                        obs.resets_at_ms,
                        obs.observed_at_ms,
                        obs.source,
                        obs.plan_hint,
                    ],
                )?;
            }

            for pid in rate_limit_providers {
                sync_auto_subscription_for_provider_conn(&tx, &pid, now_ms)?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Atomically replace all observed rate-limit windows for `(provider_id, source)` and refresh
    /// any auto-discovered subscription record. Useful for CLI / API snapshots that report a
    /// canonical active bucket list where vanished buckets should not linger.
    pub fn replace_provider_source_rate_limits(
        &self,
        provider_id: &str,
        source: &str,
        rate_limits: &[ObservedRateLimitWindow],
    ) -> CoreResult<()> {
        let trimmed_provider = provider_id.trim();
        let trimmed_source = source.trim();
        if trimmed_provider.is_empty() || trimmed_source.is_empty() {
            return Ok(());
        }
        let now_ms = Utc::now().timestamp_millis();
        self.with_conn_mut(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "DELETE FROM observed_rate_limits WHERE provider_id = ?1 AND source = ?2",
                params![trimmed_provider, trimmed_source],
            )?;
            for obs in rate_limits {
                tx.execute(
                    r#"
                    INSERT INTO observed_rate_limits
                        (provider_id, window_name, used_percent, window_minutes, resets_at_ms, observed_at_ms, source, plan_hint)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                    ON CONFLICT(provider_id, window_name) DO UPDATE SET
                        used_percent = excluded.used_percent,
                        window_minutes = excluded.window_minutes,
                        resets_at_ms = excluded.resets_at_ms,
                        observed_at_ms = excluded.observed_at_ms,
                        source = COALESCE(excluded.source, observed_rate_limits.source),
                        plan_hint = COALESCE(excluded.plan_hint, observed_rate_limits.plan_hint)
                    WHERE excluded.observed_at_ms >= observed_rate_limits.observed_at_ms
                    "#,
                    params![
                        trimmed_provider,
                        obs.window_name,
                        obs.used_percent,
                        obs.window_minutes,
                        obs.resets_at_ms,
                        obs.observed_at_ms,
                        obs.source.as_deref().unwrap_or(trimmed_source),
                        obs.plan_hint,
                    ],
                )?;
            }
            sync_auto_subscription_for_provider_conn(&tx, trimmed_provider, now_ms)?;
            tx.commit()?;
            Ok(())
        })
    }
}

pub(crate) fn load_subscription_plans(conn: &Connection) -> CoreResult<Vec<SubscriptionPlan>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT plan_id, provider_id, plan_name, enabled, monthly_fee_micro_usd,
               windows_json, is_user_configured, auto_source, updated_at_ms
        FROM subscription_plans
        ORDER BY provider_id ASC, monthly_fee_micro_usd ASC
        "#,
    )?;
    let rows = stmt.query_map([], |row| {
        let windows_json: String = row.get(5)?;
        let windows =
            serde_json::from_str::<Vec<SubscriptionWindowRule>>(&windows_json).unwrap_or_default();
        Ok(SubscriptionPlan {
            plan_id: row.get(0)?,
            provider_id: row.get(1)?,
            plan_name: row.get(2)?,
            enabled: row.get::<_, i64>(3)? != 0,
            monthly_fee_micro_usd: row.get(4)?,
            windows,
            is_user_configured: row.get::<_, i64>(6)? != 0,
            auto_source: row.get(7)?,
            updated_at_ms: row.get(8)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub(crate) fn load_observed_rate_limits(
    conn: &Connection,
) -> CoreResult<Vec<ObservedRateLimitWindow>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT provider_id, window_name, used_percent, window_minutes, resets_at_ms,
               observed_at_ms, source, plan_hint
        FROM observed_rate_limits
        "#,
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ObservedRateLimitWindow {
            provider_id: row.get(0)?,
            window_name: row.get(1)?,
            used_percent: row.get(2)?,
            window_minutes: row.get(3)?,
            resets_at_ms: row.get(4)?,
            observed_at_ms: row.get(5)?,
            source: row.get(6)?,
            plan_hint: row.get(7)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub(super) fn infer_window_rule_from_observed(
    provider_id: &str,
    obs: &ObservedRateLimitWindow,
    template_windows: &[SubscriptionWindowRule],
    daily_quota_limit_micro: Option<i64>,
    balance_currency: Option<&str>,
) -> SubscriptionWindowRule {
    let name = obs.window_name.to_ascii_lowercase();
    if obs.source.as_deref() == Some("official_cli")
        || crate::subscription::is_pool_qualified_window_name(&obs.window_name)
    {
        let duration_hours = obs
            .window_minutes
            .map(|m| (m / 60).max(1))
            .unwrap_or_else(|| {
                if name.contains("week") || name.contains("7d") {
                    168
                } else if name.contains("daily") || name.contains("24h") {
                    24
                } else if name.contains("month") || name.contains("30d") {
                    720
                } else {
                    5
                }
            });
        return SubscriptionWindowRule {
            window_id: obs.window_name.clone(),
            label: crate::subscription::format_authoritative_window_label(
                &obs.window_name,
                obs.window_minutes,
            ),
            duration_hours,
            rolling: duration_hours <= 168,
            unit: QuotaUnit::Tokens,
            limit_value: 100,
            warning_percent: 80,
            critical_percent: 95,
        };
    }

    let (window_id, default_label, duration_hours, rolling) = match obs.window_minutes {
        Some(m) if (240..=360).contains(&m) => ("5h".to_string(), "5h 滚动窗口".to_string(), 5, true),
        Some(m) if (1_200..=1_680).contains(&m) => {
            ("24h".to_string(), "24h 日额度".to_string(), 24, false)
        }
        Some(m) if (9_600..=10_560).contains(&m) => {
            ("7d".to_string(), "7d 周额度".to_string(), 168, true)
        }
        Some(m) if m >= 40_000 => ("30d".to_string(), "30d 月度限额".to_string(), 720, false),
        Some(m) => {
            let hours = (m / 60).max(1);
            (
                format!("{hours}h"),
                format!("{hours}h 窗口"),
                hours,
                hours <= 168,
            )
        }
        None if name.contains("primary") || name.contains("5h") => {
            ("5h".to_string(), "5h 滚动窗口".to_string(), 5, true)
        }
        None if name.contains("secondary") || name.contains("7d") || name.contains("week") => {
            ("7d".to_string(), "7d 周额度".to_string(), 168, true)
        }
        None if name.contains("30d") || name.contains("month") => {
            ("30d".to_string(), "30d 月度限额".to_string(), 720, false)
        }
        None if name.contains("daily") || name.contains("24h") || name.contains("day") => {
            ("24h".to_string(), "24h 日额度".to_string(), 24, false)
        }
        None => ("5h".to_string(), "5h 滚动窗口".to_string(), 5, true),
    };

    if provider_id == provider_ids::QCODE && window_id == "24h" {
        if let Some(limit) = daily_quota_limit_micro.filter(|&v| v > 0) {
            return SubscriptionWindowRule {
                window_id,
                label: default_label,
                duration_hours,
                rolling,
                unit: QuotaUnit::MicroUsd,
                limit_value: limit,
                warning_percent: 80,
                critical_percent: 95,
            };
        }
    }

    if let Some(tpl_win) = template_windows.iter().find(|w| w.window_id == window_id) {
        let mut cloned = tpl_win.clone();
        if provider_id == provider_ids::KIMI_CODE && balance_currency == Some("QUOTA") {
            cloned.label = match window_id.as_str() {
                "5h" => "5h 请求/频次窗口".to_string(),
                "7d" => "7d 周额度窗口".to_string(),
                "30d" => "30d 月度限额".to_string(),
                _ => cloned.label,
            };
        }
        return cloned;
    }

    SubscriptionWindowRule {
        window_id: window_id.clone(),
        label: default_label,
        duration_hours,
        rolling,
        unit: QuotaUnit::MicroUsd,
        limit_value: if window_id == "30d" {
            120_000_000
        } else {
            20_000_000
        },
        warning_percent: 80,
        critical_percent: 95,
    }
}

/// Map a Kimi Code membership level (from the official `/coding/v1/usages` response) onto the
/// catalog tier it corresponds to. Returns `None` for an empty or unrecognised level.
fn kimi_template_for_hint(hint_lower: &str) -> Option<&'static str> {
    match hint_lower.trim() {
        "level_basic" | "basic" | "moderato" | "plus" => Some("kimi_code_plus"),
        "level_intermediate" | "intermediate" | "allegretto" | "pro" => {
            Some("kimi_code_allegretto")
        }
        "level_advanced" | "advanced" | "allegro" | "max" => Some("kimi_code_max"),
        "level_max" | "vivace" | "ultra" => Some("kimi_code_ultra"),
        _ => None,
    }
}

pub(crate) fn sync_auto_subscription_for_provider_conn(
    conn: &Connection,
    provider_id: &str,
    now_ms: i64,
) -> CoreResult<()> {
    let trimmed = provider_id.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    // Do not auto-seed if the user already maintains an active manually created plan for this provider.
    let has_manual_plan: Option<i64> = conn
        .query_row(
            r#"
            SELECT 1
            FROM subscription_plans
            WHERE provider_id = ?1
              AND is_user_configured = 1
              AND auto_source IS NULL
              AND plan_id NOT LIKE 'auto_%'
            LIMIT 1
            "#,
            params![trimmed],
            |row| row.get(0),
        )
        .optional()?;
    if has_manual_plan.is_some() {
        return Ok(());
    }

    let auto_plan_id = format!("auto_{trimmed}");
    let existing_flags: Option<(i64, i64)> = conn
        .query_row(
            "SELECT user_dismissed, user_customized FROM subscription_plans WHERE plan_id = ?1",
            params![auto_plan_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((dismissed, customized)) = existing_flags {
        if dismissed != 0 || customized != 0 {
            return Ok(());
        }
    }

    let obs_rows: Vec<ObservedRateLimitWindow> = load_observed_rate_limits(conn)?
        .into_iter()
        .filter(|o| o.provider_id.eq_ignore_ascii_case(trimmed))
        .collect();
    if obs_rows.is_empty() && trimmed != provider_ids::ANTIGRAVITY {
        return Ok(());
    }

    let bal_info: Option<(String, Option<i64>)> = conn
        .query_row(
            "SELECT currency, daily_quota_limit_micro FROM provider_balances WHERE provider_id = ?1",
            params![trimmed],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let balance_currency = bal_info.as_ref().map(|(c, _)| c.as_str());
    let daily_quota_limit = bal_info.as_ref().and_then(|(_, l)| *l);

    let auto_source = if obs_rows
        .iter()
        .any(|o| o.source.as_deref() == Some("official_api"))
        || matches!(
            trimmed,
            provider_ids::KIMI_CODE | provider_ids::MINIMAX_CODE | provider_ids::QCODE
        ) {
        "official_api".to_string()
    } else if obs_rows
        .iter()
        .any(|o| o.source.as_deref() == Some("official_cli"))
    {
        "official_cli".to_string()
    } else if !obs_rows.is_empty()
        && obs_rows
            .iter()
            .any(|o| o.source.as_deref() != Some("local_log_hint"))
    {
        "official_log_snapshot".to_string()
    } else {
        "local_log".to_string()
    };

    let plan_hint = obs_rows
        .iter()
        .filter_map(|o| {
            o.plan_hint
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| (o.observed_at_ms, s.to_string()))
        })
        .max_by_key(|(ts, _)| *ts)
        .map(|(_, s)| s)
        .or_else(|| {
            if trimmed == provider_ids::ANTIGRAVITY {
                let mut stmt = conn
                    .prepare(
                        "SELECT DISTINCT LOWER(model_id) FROM usage_events WHERE provider_id = ?1",
                    )
                    .ok()?;
                let models: Vec<String> = stmt
                    .query_map(params![provider_ids::ANTIGRAVITY], |r| r.get::<_, String>(0))
                    .ok()?
                    .flatten()
                    .collect();
                if models
                    .iter()
                    .any(|m| m.contains("ultra") && (m.contains("5x") || m.contains("99")))
                {
                    Some("ultra_5x".to_string())
                } else if models
                    .iter()
                    .any(|m| m.contains("ultra") || m.contains("carbon") || m.contains("g1-ultra"))
                {
                    Some("ultra".to_string())
                } else if models.iter().any(|m| m.contains("enterprise")) {
                    Some("enterprise".to_string())
                } else {
                    None
                }
            } else {
                None
            }
        });

    let real_obs_rows: Vec<&ObservedRateLimitWindow> = obs_rows
        .iter()
        .filter(|o| o.source.as_deref() != Some("local_log_hint"))
        .collect();
    if trimmed == provider_ids::ANTIGRAVITY && plan_hint.is_none() && real_obs_rows.is_empty() {
        return Ok(());
    }

    let hint_lower = plan_hint
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();

    let templates = load_subscription_catalog_templates(conn)?;
    let preferred_tpl = templates.iter().find(|tpl| {
        tpl.provider_id == trimmed
            && match trimmed {
                provider_ids::CODEX if hint_lower.contains("pro") && hint_lower.contains("100") => {
                    tpl.template_id == "codex_pro_100"
                }
                provider_ids::CODEX if hint_lower.contains("pro") && hint_lower.contains("500") => {
                    tpl.template_id == "codex_pro_500"
                }
                provider_ids::CODEX if hint_lower.contains("pro") => tpl.template_id == "codex_pro",
                provider_ids::CODEX if hint_lower.contains("team") => {
                    tpl.template_id == "codex_team"
                }
                provider_ids::CODEX => tpl.template_id == "codex_plus",
                provider_ids::ANTIGRAVITY
                    if hint_lower.contains("ultra")
                        && (hint_lower.contains("5x") || hint_lower.contains("99")) =>
                {
                    tpl.template_id == "google_ai_ultra_5x"
                }
                provider_ids::ANTIGRAVITY if hint_lower.contains("ultra") => {
                    tpl.template_id == "google_ai_ultra"
                }
                provider_ids::ANTIGRAVITY
                    if hint_lower.contains("enterprise") || hint_lower.contains("business") =>
                {
                    tpl.template_id == "gemini_enterprise"
                }
                provider_ids::ANTIGRAVITY if hint_lower.contains("plus") => {
                    tpl.template_id == "google_ai_plus"
                }
                provider_ids::ANTIGRAVITY if hint_lower.contains("free") => {
                    tpl.template_id == "google_ai_free"
                }
                provider_ids::ANTIGRAVITY if hint_lower.contains("pro") => {
                    tpl.template_id == "google_ai_pro"
                }
                provider_ids::ANTIGRAVITY => false,
                provider_ids::KIMI_CODE => {
                    tpl.template_id
                        == kimi_template_for_hint(&hint_lower).unwrap_or("kimi_code_allegretto")
                }
                _ => true,
            }
    });
    let tpl_windows: Vec<SubscriptionWindowRule> = preferred_tpl
        .map(|t| t.windows.clone())
        .unwrap_or_default();

    let mut rules_by_id: BTreeMap<String, SubscriptionWindowRule> = BTreeMap::new();
    for obs in &real_obs_rows {
        let rule = infer_window_rule_from_observed(
            trimmed,
            obs,
            &tpl_windows,
            daily_quota_limit,
            balance_currency,
        );
        rules_by_id.entry(rule.window_id.clone()).or_insert(rule);
    }

    // Only keep windows backed by actual authoritative observations (never fabricate unverified
    // local_estimation windows for Antigravity or unreturned monthly windows).
    let mut windows: Vec<SubscriptionWindowRule> = rules_by_id.into_values().collect();
    windows.sort_by(|a, b| {
        let a_3p = a.window_id.to_ascii_lowercase().starts_with("3p-");
        let b_3p = b.window_id.to_ascii_lowercase().starts_with("3p-");
        a_3p.cmp(&b_3p)
            .then_with(|| a.duration_hours.cmp(&b.duration_hours))
            .then_with(|| a.window_id.cmp(&b.window_id))
    });
    if windows.is_empty() && trimmed != provider_ids::ANTIGRAVITY {
        return Ok(());
    }

    let (plan_name, monthly_fee_micro_usd) = match trimmed {
        provider_ids::CLAUDE_CODE => {
            if hint_lower.contains("max") && (hint_lower.contains("20") || hint_lower.contains("200")) {
                ("Claude Max ($200)".to_string(), 200_000_000)
            } else if hint_lower.contains("max") {
                ("Claude Max ($100)".to_string(), 100_000_000)
            } else if hint_lower.contains("team") {
                ("Claude Team".to_string(), 30_000_000)
            } else if hint_lower.contains("pro") {
                ("Claude Pro".to_string(), 20_000_000)
            } else {
                ("Claude Code 官方订阅".to_string(), 0)
            }
        }
        provider_ids::CODEX => {
            if hint_lower.contains("pro") && hint_lower.contains("500") {
                (
                    preferred_tpl
                        .map(|t| t.plan_name.clone())
                        .unwrap_or_else(|| "ChatGPT Pro ($500) / Codex".to_string()),
                    preferred_tpl
                        .map(|t| t.monthly_fee_micro_usd)
                        .unwrap_or(500_000_000),
                )
            } else if hint_lower.contains("pro") && hint_lower.contains("100") {
                (
                    preferred_tpl
                        .map(|t| t.plan_name.clone())
                        .unwrap_or_else(|| "ChatGPT Pro ($100) / Codex".to_string()),
                    preferred_tpl
                        .map(|t| t.monthly_fee_micro_usd)
                        .unwrap_or(100_000_000),
                )
            } else if hint_lower.contains("pro") {
                ("ChatGPT Pro / Codex".to_string(), 200_000_000)
            } else if hint_lower.contains("team") {
                ("ChatGPT Team / Codex".to_string(), 25_000_000)
            } else if hint_lower.contains("enterprise") {
                ("ChatGPT Enterprise / Codex".to_string(), 0)
            } else if hint_lower.contains("edu") {
                ("ChatGPT Edu / Codex".to_string(), 0)
            } else if hint_lower.contains("plus") {
                ("ChatGPT Plus / Codex".to_string(), 20_000_000)
            } else if hint_lower.contains("free") {
                ("ChatGPT Free / Codex".to_string(), 0)
            } else {
                ("Codex 官方订阅".to_string(), 0)
            }
        }
        provider_ids::ANTIGRAVITY => {
            if hint_lower.contains("ultra")
                && (hint_lower.contains("5x") || hint_lower.contains("99"))
            {
                (
                    preferred_tpl
                        .map(|t| t.plan_name.clone())
                        .unwrap_or_else(|| "Google AI Ultra 5x".to_string()),
                    preferred_tpl
                        .map(|t| t.monthly_fee_micro_usd)
                        .unwrap_or(99_990_000),
                )
            } else if hint_lower.contains("ultra") && hint_lower.contains("249") {
                ("Google AI Ultra".to_string(), 249_990_000)
            } else if hint_lower.contains("ultra") {
                (
                    preferred_tpl
                        .map(|t| t.plan_name.clone())
                        .unwrap_or_else(|| "Google AI Ultra".to_string()),
                    preferred_tpl
                        .map(|t| t.monthly_fee_micro_usd)
                        .unwrap_or(199_990_000),
                )
            } else if hint_lower.contains("enterprise") || hint_lower.contains("business") {
                ("Gemini Enterprise / Antigravity".to_string(), 30_000_000)
            } else if hint_lower.contains("plus") {
                (
                    preferred_tpl
                        .map(|t| t.plan_name.clone())
                        .unwrap_or_else(|| "Google AI Plus".to_string()),
                    preferred_tpl
                        .map(|t| t.monthly_fee_micro_usd)
                        .unwrap_or(4_990_000),
                )
            } else if hint_lower.contains("free") {
                ("Google AI Free / Antigravity".to_string(), 0)
            } else if hint_lower.contains("pro") {
                (
                    preferred_tpl
                        .map(|t| t.plan_name.clone())
                        .unwrap_or_else(|| "Google AI Pro".to_string()),
                    preferred_tpl
                        .map(|t| t.monthly_fee_micro_usd)
                        .unwrap_or(19_990_000),
                )
            } else {
                ("Antigravity 官方配额".to_string(), 0)
            }
        }
        provider_ids::KIMI_CODE => match (kimi_template_for_hint(&hint_lower), preferred_tpl) {
            // A recognised membership level maps 1:1 onto a catalog tier: take both the tier name
            // and its monthly fee from the catalog so they never drift apart.
            (Some(_), Some(tpl)) => (tpl.plan_name.clone(), tpl.monthly_fee_micro_usd),
            // The usages response carries no level: keep a neutral name and no fee so the user
            // picks the tier explicitly instead of us guessing a price.
            _ => ("Kimi Code 会员订阅".to_string(), 0),
        },
        provider_ids::MINIMAX_CODE => {
            if hint_lower.contains("max") {
                ("MiniMax Coding Max".to_string(), 0)
            } else if hint_lower.contains("pro") || hint_lower.contains("high") {
                ("MiniMax Coding Pro".to_string(), 0)
            } else if hint_lower.contains("plus") {
                ("MiniMax Coding Plus".to_string(), 0)
            } else if !hint_lower.is_empty() {
                let raw_hint = plan_hint.as_deref().unwrap_or("").trim();
                (format!("MiniMax Coding ({raw_hint})"), 0)
            } else {
                ("MiniMax Coding Plan".to_string(), 0)
            }
        }
        provider_ids::QCODE => {
            if hint_lower.contains("pro") || hint_lower.contains("max") {
                ("QCode Pro 订阅".to_string(), 0)
            } else if hint_lower.contains("plus") {
                ("QCode Plus 订阅".to_string(), 0)
            } else {
                ("QCode 日额度套餐".to_string(), 0)
            }
        }
        other => (format!("{other} 官方订阅"), 0),
    };

    let windows_json = serde_json::to_string(&windows)?;
    conn.execute(
        r#"
        INSERT INTO subscription_plans
            (plan_id, provider_id, plan_name, enabled, monthly_fee_micro_usd, windows_json, is_user_configured, auto_source, user_customized, user_dismissed, updated_at_ms)
        VALUES (?1, ?2, ?3, 1, ?4, ?5, 1, ?6, 0, 0, ?7)
        ON CONFLICT(plan_id) DO UPDATE SET
            plan_name = excluded.plan_name,
            monthly_fee_micro_usd = excluded.monthly_fee_micro_usd,
            windows_json = excluded.windows_json,
            is_user_configured = 1,
            auto_source = excluded.auto_source,
            updated_at_ms = excluded.updated_at_ms
        WHERE subscription_plans.user_customized = 0 AND subscription_plans.user_dismissed = 0
        "#,
        params![
            auto_plan_id,
            trimmed,
            plan_name,
            monthly_fee_micro_usd,
            windows_json,
            auto_source,
            now_ms,
        ],
    )?;

    Ok(())
}

pub(crate) fn load_subscription_catalog_templates(
    conn: &Connection,
) -> CoreResult<Vec<SubscriptionPlanTemplate>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT template_id, provider_id, plan_name, monthly_fee_micro_usd, windows_json, description
        FROM subscription_catalog
        ORDER BY sort_order ASC, template_id ASC
        "#,
    )?;
    let rows = stmt.query_map([], |row| {
        let windows_json: String = row.get(4)?;
        let windows =
            serde_json::from_str::<Vec<SubscriptionWindowRule>>(&windows_json).unwrap_or_default();
        Ok(SubscriptionPlanTemplate {
            template_id: row.get(0)?,
            provider_id: row.get(1)?,
            plan_name: row.get(2)?,
            monthly_fee_micro_usd: row.get(3)?,
            windows,
            description: row.get(5)?,
            live_windows: Vec::new(),
        })
    })?;
    let mut synced = Vec::new();
    for row in rows {
        synced.push(row?);
    }

    let built_in = subscription_plan_templates();
    if synced.is_empty() {
        return Ok(built_in);
    }

    let seen_ids: BTreeSet<String> = synced.iter().map(|t| t.template_id.clone()).collect();
    for tpl in built_in {
        if !seen_ids.contains(&tpl.template_id) {
            synced.push(tpl);
        }
    }
    Ok(synced)
}

pub(super) fn default_payg_account_name(provider_id: &str, currency: &str) -> String {
    match (provider_id, currency) {
        (provider_ids::CODEX, c) if c.starts_with("CREDITS") => "Codex Credits".to_string(),
        (provider_ids::CODEX, _) => "Codex 按量余额".to_string(),
        (provider_ids::DEEPSEEK_HARNESS, _) => "DeepSeek 官方账户余额".to_string(),
        (provider_ids::KIMI_CODE, _) => "Kimi Code 账户余额".to_string(),
        (provider_ids::QCODE, _) => "QCode 账户余额".to_string(),
        (provider_ids::MINIMAX_CODE, _) => "MiniMax 账户余额".to_string(),
        (other, _) => format!("{} 账户余额", provider_ids::display_name(other)),
    }
}

pub(crate) fn sync_auto_payg_for_balance_conn(
    conn: &Connection,
    balance: &ProviderBalanceSnapshot,
) -> CoreResult<()> {
    let provider_id = balance.provider_id.trim();
    if provider_id.is_empty() || balance.currency.eq_ignore_ascii_case("QUOTA") {
        return Ok(());
    }
    let has_manual: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM payg_accounts WHERE provider_id = ?1 AND is_auto = 0 AND user_dismissed = 0 LIMIT 1",
            params![provider_id],
            |r| r.get(0),
        )
        .optional()?;
    if has_manual.is_some() {
        return Ok(());
    }

    let auto_id = format!("auto_{provider_id}");
    let dismissed: Option<i64> = conn
        .query_row(
            "SELECT user_dismissed FROM payg_accounts WHERE account_id = ?1",
            params![auto_id],
            |r| r.get(0),
        )
        .optional()?;
    if dismissed == Some(1) {
        return Ok(());
    }

    let account_name = default_payg_account_name(provider_id, &balance.currency);
    conn.execute(
        r#"
        INSERT INTO payg_accounts (
            account_id, provider_id, account_name, enabled,
            manual_balance_micro_usd, balance_anchor_ms, monthly_budget_micro_usd,
            is_auto, user_dismissed, updated_at_ms
        ) VALUES (?1, ?2, ?3, 1, NULL, NULL, NULL, 1, 0, ?4)
        ON CONFLICT(account_id) DO UPDATE SET
            account_name = excluded.account_name,
            updated_at_ms = excluded.updated_at_ms
        WHERE payg_accounts.user_dismissed = 0 AND payg_accounts.is_auto = 1
        "#,
        params![auto_id, provider_id, account_name, balance.fetched_at_ms],
    )?;
    Ok(())
}

pub(crate) fn load_window_samples_map(
    conn: &Connection,
    now_ms: i64,
) -> CoreResult<HashMap<String, Vec<WindowUsageSample>>> {
    // Load up to 31 days of event samples for evaluating 5h / 7d / 30d windows accurately.
    let lookback_start = now_ms.saturating_sub(31 * DAY_MS);
    let mut stmt = conn.prepare(
        r#"
        SELECT provider_id, timestamp_ms, estimated_cost_micro_usd,
               (input_tokens + output_tokens + cache_read_tokens + cache_write_tokens + reasoning_tokens) AS total_tokens
        FROM usage_events
        WHERE timestamp_ms >= ?1 AND timestamp_ms <= ?2
        "#,
    )?;
    let rows = stmt.query_map(params![lookback_start, now_ms], |row| {
        Ok((
            row.get::<_, String>(0)?,
            WindowUsageSample {
                timestamp_ms: row.get(1)?,
                cost_micro_usd: row.get(2)?,
                total_tokens: row.get(3)?,
            },
        ))
    })?;
    let mut map: HashMap<String, Vec<WindowUsageSample>> = HashMap::new();
    for row in rows {
        let (provider_id, sample) = row?;
        map.entry(provider_id).or_default().push(sample);
    }
    Ok(map)
}
