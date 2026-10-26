//! 设置仓储实现：应用主题、终端字号、配色方案与高亮配置的键值持久化。
//! 值仍由 Domain/Application 解析和校验，SQLite 只承载稳定键值表。

use nocterm_domain::settings::{
    AppTheme, HighlightPreset, SettingsRepository, SettingsRepositoryError, TerminalColorScheme,
};
use rusqlite::{OptionalExtension, params};

use super::SqliteConnectionRepository;

/// 设置仓储错误统一包装：与连接仓储错误同等只保留诊断文本。
fn settings_error(error: impl std::fmt::Display) -> SettingsRepositoryError {
    SettingsRepositoryError::new(error.to_string())
}

impl SettingsRepository for SqliteConnectionRepository {
    fn app_theme(&self) -> Result<Option<AppTheme>, SettingsRepositoryError> {
        self.setting_value("app_theme")?
            .map(|value| AppTheme::parse(&value).map_err(settings_error))
            .transpose()
    }

    fn set_app_theme(&self, theme: AppTheme) -> Result<(), SettingsRepositoryError> {
        self.set_setting_value("app_theme", theme.as_str())
    }

    fn terminal_font_size(&self) -> Result<Option<u8>, SettingsRepositoryError> {
        self.setting_value("terminal_font_size")?
            .map(|value| value.parse::<u8>().map_err(settings_error))
            .transpose()
    }

    fn terminal_color_scheme(
        &self,
    ) -> Result<Option<TerminalColorScheme>, SettingsRepositoryError> {
        self.setting_value("terminal_color_scheme")?
            .map(|value| TerminalColorScheme::parse(&value).map_err(settings_error))
            .transpose()
    }

    fn highlight_preset(&self) -> Result<Option<HighlightPreset>, SettingsRepositoryError> {
        self.setting_value("terminal_highlight_preset")?
            .map(|value| HighlightPreset::parse(&value).map_err(settings_error))
            .transpose()
    }

    fn highlight_overrides(&self) -> Result<Option<String>, SettingsRepositoryError> {
        self.setting_value("terminal_highlight_overrides")
    }

    fn set_terminal_appearance(
        &self,
        font_size: u8,
        color_scheme: TerminalColorScheme,
        highlight_preset: HighlightPreset,
        highlight_overrides: &str,
    ) -> Result<(), SettingsRepositoryError> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| settings_error("database lock poisoned"))?;
        let transaction = connection.transaction().map_err(settings_error)?;
        for (key, value) in [
            ("terminal_font_size", font_size.to_string()),
            ("terminal_color_scheme", color_scheme.as_str().to_string()),
            (
                "terminal_highlight_preset",
                highlight_preset.as_str().to_string(),
            ),
            (
                "terminal_highlight_overrides",
                highlight_overrides.to_string(),
            ),
        ] {
            transaction
                .execute(
                    "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = unixepoch()",
                    params![key, value],
                )
                .map_err(settings_error)?;
        }
        transaction.commit().map_err(settings_error)
    }
}

impl SqliteConnectionRepository {
    /// 通用键值表的 SQL 只保留一份，领域仓储仍以类型安全方法对外暴露。
    fn setting_value(&self, key: &str) -> Result<Option<String>, SettingsRepositoryError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| settings_error("database lock poisoned"))?;
        connection
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                [key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(settings_error)
    }

    fn set_setting_value(&self, key: &str, value: &str) -> Result<(), SettingsRepositoryError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| settings_error("database lock poisoned"))?;
        connection
            .execute(
                "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = unixepoch()",
                params![key, value],
            )
            .map_err(settings_error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::SqliteConnectionRepository;
    use nocterm_domain::settings::{
        AppTheme, HighlightPreset, SettingsRepository, TerminalColorScheme,
    };

    #[test]
    fn persists_application_theme_without_materializing_the_default() {
        let repository = SqliteConnectionRepository::open_in_memory().expect("open database");

        assert_eq!(repository.app_theme().expect("read default"), None);
        repository
            .set_app_theme(AppTheme::Dark)
            .expect("save application theme");
        assert_eq!(
            repository.app_theme().expect("read saved theme"),
            Some(AppTheme::Dark)
        );

        assert_eq!(repository.terminal_font_size().expect("read font"), None);
        repository
            .set_terminal_appearance(
                16,
                TerminalColorScheme::NoctermDark,
                HighlightPreset::MobaXterm,
                "directory=12",
            )
            .expect("save terminal appearance");
        assert_eq!(
            repository.terminal_font_size().expect("read saved font"),
            Some(16)
        );
        assert_eq!(
            repository
                .terminal_color_scheme()
                .expect("read saved color scheme"),
            Some(TerminalColorScheme::NoctermDark)
        );
        assert_eq!(
            repository.highlight_preset().expect("read saved preset"),
            Some(HighlightPreset::MobaXterm)
        );
        assert_eq!(
            repository
                .highlight_overrides()
                .expect("read saved overrides"),
            Some("directory=12".to_string())
        );
    }
}
