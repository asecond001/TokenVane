//! App preferences and device registry.

use super::*;

impl Storage {
    pub fn get_preferences(&self) -> CoreResult<AppPreferences> {
        self.with_conn(load_preferences)
    }

    pub fn update_preferences(&self, prefs: &AppPreferences) -> CoreResult<()> {
        let now_ms = Utc::now().timestamp_millis();
        crate::pricing::parse_exchange_rate(&prefs.usd_to_cny_rate)?;
        let bg_mode = prefs.bg_mode.trim();
        if !matches!(bg_mode, "none" | "image" | "gradient") {
            return Err(CoreError::Validation(format!(
                "Unsupported bg_mode '{}': expected none, image, or gradient",
                prefs.bg_mode
            )));
        }
        if !prefs.bg_opacity.is_finite() {
            return Err(CoreError::Validation(
                "bg_opacity must be a finite number".to_string(),
            ));
        }
        let bg_opacity = prefs.bg_opacity.clamp(0.0, 1.0);
        let bg_blur_px = prefs.bg_blur_px.clamp(0, 64);
        let bg_preset_id = if prefs.bg_preset_id.trim().is_empty() {
            "aurora_slate"
        } else {
            prefs.bg_preset_id.trim()
        };
        let bg_image_path = prefs
            .bg_image_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);

        self.with_conn(|conn| {
            conn.execute(
                r#"
                UPDATE app_preferences
                SET currency = ?1,
                    usd_to_cny_rate = ?2,
                    theme = ?3,
                    minimize_to_tray = ?4,
                    launch_at_startup = ?5,
                    otlp_enabled = ?6,
                    otlp_port = ?7,
                    device_display_name = ?8,
                    bg_mode = ?9,
                    bg_image_path = ?10,
                    bg_preset_id = ?11,
                    bg_opacity = ?12,
                    bg_blur_px = ?13,
                    auto_sync_pricing = ?14,
                    exchange_rate_auto = ?15,
                    auto_check_updates = ?16,
                    floating_window_enabled = ?17,
                    background_cli_quota_poll = ?18,
                    updated_at_ms = ?19
                WHERE id = 1
                "#,
                params![
                    prefs.currency.as_str(),
                    prefs.usd_to_cny_rate,
                    prefs.theme,
                    i64::from(prefs.minimize_to_tray),
                    i64::from(prefs.launch_at_startup),
                    i64::from(prefs.otlp_enabled),
                    i64::from(prefs.otlp_port),
                    prefs.device_display_name,
                    bg_mode,
                    bg_image_path,
                    bg_preset_id,
                    bg_opacity,
                    bg_blur_px,
                    i64::from(prefs.auto_sync_pricing),
                    i64::from(prefs.exchange_rate_auto),
                    i64::from(prefs.auto_check_updates),
                    i64::from(prefs.floating_window_enabled),
                    i64::from(prefs.background_cli_quota_poll),
                    now_ms,
                ],
            )?;
            Ok(())
        })
    }

    pub fn list_devices(&self) -> CoreResult<Vec<DeviceRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT device_id, device_name, platform, created_at_ms FROM devices ORDER BY created_at_ms ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(DeviceRecord {
                    device_id: row.get(0)?,
                    device_name: row.get(1)?,
                    platform: row.get(2)?,
                    created_at_ms: row.get(3)?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }
}

pub(crate) fn load_preferences(conn: &Connection) -> CoreResult<AppPreferences> {
    conn.query_row(
        r#"
        SELECT currency, usd_to_cny_rate, theme, minimize_to_tray, launch_at_startup,
               otlp_enabled, otlp_port, device_display_name,
               bg_mode, bg_image_path, bg_preset_id, bg_opacity, bg_blur_px,
               auto_sync_pricing, exchange_rate_auto, last_rate_sync_at_ms, last_pricing_sync_at_ms,
               last_subscription_sync_at_ms,
               auto_check_updates, floating_window_enabled, background_cli_quota_poll, updated_at_ms
        FROM app_preferences
        WHERE id = 1
        "#,
        [],
        |row| {
            let cur_str: String = row.get(0)?;
            Ok(AppPreferences {
                currency: CurrencyCode::parse(&cur_str),
                usd_to_cny_rate: row.get(1)?,
                theme: row.get(2)?,
                minimize_to_tray: row.get::<_, i64>(3)? != 0,
                launch_at_startup: row.get::<_, i64>(4)? != 0,
                otlp_enabled: row.get::<_, i64>(5)? != 0,
                otlp_port: row.get::<_, i64>(6)?.clamp(1, 65535) as u16,
                device_display_name: row.get(7)?,
                bg_mode: row.get(8)?,
                bg_image_path: row.get(9)?,
                bg_preset_id: row.get(10)?,
                bg_opacity: row.get(11)?,
                bg_blur_px: row.get(12)?,
                auto_sync_pricing: row.get::<_, i64>(13)? != 0,
                exchange_rate_auto: row.get::<_, i64>(14)? != 0,
                last_rate_sync_at_ms: row.get(15)?,
                last_pricing_sync_at_ms: row.get(16)?,
                last_subscription_sync_at_ms: row.get(17)?,
                auto_check_updates: row.get::<_, i64>(18)? != 0,
                floating_window_enabled: row.get::<_, i64>(19)? != 0,
                background_cli_quota_poll: row.get::<_, i64>(20)? != 0,
                updated_at_ms: row.get(21)?,
            })
        },
    )
    .map_err(CoreError::from)
}
