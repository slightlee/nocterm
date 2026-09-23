use std::sync::Arc;

use nocterm_domain::settings::{
    AppTheme, DEFAULT_TERMINAL_FONT_SIZE, HighlightPreset, SettingsRepository, TerminalColorScheme,
    canonicalize_highlight_overrides, parse_highlight_overrides, validate_terminal_font_size,
};

use crate::error::AppError;

/// 设置用例集中处理默认值和输入校验，IPC 与持久化层不各自复制业务规则。
#[derive(Clone)]
pub struct SettingsService {
    repository: Arc<dyn SettingsRepository>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalAppearance {
    pub font_size: u8,
    pub color_scheme: TerminalColorScheme,
    /// 稳定标识串（`theme`/`mobaxterm`/`high_contrast`），序列化细节留在 DTO。
    pub highlight_preset: String,
    /// 归一化覆盖串，空串表示无覆盖。
    pub highlight_overrides: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightPreferences {
    pub preset: HighlightPreset,
    /// 归一化后的覆盖项（`role=slot` 以 `,` 相连），空串表示无覆盖。
    pub overrides: String,
}

impl Default for HighlightPreferences {
    fn default() -> Self {
        Self {
            preset: HighlightPreset::Theme,
            overrides: String::new(),
        }
    }
}

impl SettingsService {
    pub fn new(repository: Arc<dyn SettingsRepository>) -> Self {
        Self { repository }
    }

    /// 老数据库没有保存过主题时使用 `system`，无需写入一条隐式默认记录。
    pub fn app_theme(&self) -> Result<AppTheme, AppError> {
        self.repository
            .app_theme()
            .map(|theme| theme.unwrap_or(AppTheme::System))
            .map_err(|_| AppError::new("SETTINGS_READ_FAILED", "读取应用设置失败", true))
    }

    pub fn set_app_theme(&self, value: &str) -> Result<AppTheme, AppError> {
        let theme = AppTheme::parse(value)
            .map_err(|error| AppError::new(error.code, error.message, false))?;
        self.repository
            .set_app_theme(theme)
            .map_err(|_| AppError::new("SETTINGS_WRITE_FAILED", "保存应用设置失败", true))?;
        Ok(theme)
    }

    pub fn terminal_appearance(&self) -> Result<TerminalAppearance, AppError> {
        let font_size = self
            .repository
            .terminal_font_size()
            .map_err(|_| AppError::new("SETTINGS_READ_FAILED", "读取终端设置失败", true))?
            .unwrap_or(DEFAULT_TERMINAL_FONT_SIZE);
        let color_scheme = self
            .repository
            .terminal_color_scheme()
            .map_err(|_| AppError::new("SETTINGS_READ_FAILED", "读取终端设置失败", true))?
            .unwrap_or(TerminalColorScheme::FollowApp);
        let highlight = self.highlight_preferences()?;
        Ok(TerminalAppearance {
            font_size,
            color_scheme,
            highlight_preset: highlight.preset.as_str().to_string(),
            highlight_overrides: highlight.overrides,
        })
    }

    fn highlight_preferences(&self) -> Result<HighlightPreferences, AppError> {
        let preset = self
            .repository
            .highlight_preset()
            .map_err(|_| AppError::new("SETTINGS_READ_FAILED", "读取终端设置失败", true))?
            .unwrap_or(HighlightPreset::Theme);
        let overrides = self
            .repository
            .highlight_overrides()
            .map_err(|_| AppError::new("SETTINGS_READ_FAILED", "读取终端设置失败", true))?
            .unwrap_or_default();
        Ok(HighlightPreferences { preset, overrides })
    }

    pub fn set_terminal_appearance(
        &self,
        font_size: u8,
        color_scheme: &str,
        highlight_preset: &str,
        highlight_overrides: &str,
    ) -> Result<TerminalAppearance, AppError> {
        let font_size = validate_terminal_font_size(font_size)
            .map_err(|error| AppError::new(error.code, error.message, false))?;
        let color_scheme = TerminalColorScheme::parse(color_scheme)
            .map_err(|error| AppError::new(error.code, error.message, false))?;
        let preset = HighlightPreset::parse(highlight_preset)
            .map_err(|error| AppError::new(error.code, error.message, false))?;
        let entries = parse_highlight_overrides(highlight_overrides)
            .map_err(|error| AppError::new(error.code, error.message, false))?;
        let overrides = canonicalize_highlight_overrides(&entries);
        self.repository
            .set_terminal_appearance(font_size, color_scheme, preset, &overrides)
            .map_err(|_| AppError::new("SETTINGS_WRITE_FAILED", "保存终端设置失败", true))?;
        Ok(TerminalAppearance {
            font_size,
            color_scheme,
            highlight_preset: preset.as_str().to_string(),
            highlight_overrides: overrides,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use nocterm_domain::settings::SettingsRepositoryError;

    use super::*;

    #[derive(Default)]
    struct FakeSettingsRepository {
        theme: Mutex<Option<AppTheme>>,
        font_size: Mutex<Option<u8>>,
        color_scheme: Mutex<Option<TerminalColorScheme>>,
        highlight_preset: Mutex<Option<HighlightPreset>>,
        highlight_overrides: Mutex<Option<String>>,
    }

    impl SettingsRepository for FakeSettingsRepository {
        fn app_theme(&self) -> Result<Option<AppTheme>, SettingsRepositoryError> {
            Ok(*self.theme.lock().expect("theme lock"))
        }

        fn set_app_theme(&self, theme: AppTheme) -> Result<(), SettingsRepositoryError> {
            *self.theme.lock().expect("theme lock") = Some(theme);
            Ok(())
        }

        fn terminal_font_size(&self) -> Result<Option<u8>, SettingsRepositoryError> {
            Ok(*self.font_size.lock().expect("font size lock"))
        }

        fn terminal_color_scheme(
            &self,
        ) -> Result<Option<TerminalColorScheme>, SettingsRepositoryError> {
            Ok(*self.color_scheme.lock().expect("color scheme lock"))
        }

        fn highlight_preset(&self) -> Result<Option<HighlightPreset>, SettingsRepositoryError> {
            Ok(*self.highlight_preset.lock().expect("highlight preset lock"))
        }

        fn highlight_overrides(&self) -> Result<Option<String>, SettingsRepositoryError> {
            Ok(self
                .highlight_overrides
                .lock()
                .expect("overrides lock")
                .clone())
        }

        fn set_terminal_appearance(
            &self,
            font_size: u8,
            color_scheme: TerminalColorScheme,
            highlight_preset: HighlightPreset,
            highlight_overrides: &str,
        ) -> Result<(), SettingsRepositoryError> {
            *self.font_size.lock().expect("font size lock") = Some(font_size);
            *self.color_scheme.lock().expect("color scheme lock") = Some(color_scheme);
            *self.highlight_preset.lock().expect("highlight preset lock") = Some(highlight_preset);
            *self.highlight_overrides.lock().expect("overrides lock") =
                Some(highlight_overrides.to_string());
            Ok(())
        }
    }

    #[test]
    fn defaults_to_system_and_persists_a_valid_theme() {
        let repository = Arc::new(FakeSettingsRepository::default());
        let service = SettingsService::new(repository);

        assert_eq!(
            service.app_theme().expect("default theme"),
            AppTheme::System
        );
        assert_eq!(
            service.set_app_theme("dark").expect("save theme"),
            AppTheme::Dark
        );
        assert_eq!(service.app_theme().expect("saved theme"), AppTheme::Dark);
    }

    #[test]
    fn rejects_an_unknown_theme_without_persisting_it() {
        let repository = Arc::new(FakeSettingsRepository::default());
        let service = SettingsService::new(repository);

        let error = service
            .set_app_theme("contrast")
            .expect_err("invalid theme");

        assert_eq!(error.code, "SETTINGS_APP_THEME_INVALID");
        assert_eq!(
            service.app_theme().expect("unchanged theme"),
            AppTheme::System
        );
    }

    #[test]
    fn defaults_and_persists_terminal_appearance() {
        let repository = Arc::new(FakeSettingsRepository::default());
        let service = SettingsService::new(repository);

        assert_eq!(
            service.terminal_appearance().expect("default appearance"),
            TerminalAppearance {
                font_size: 13,
                color_scheme: TerminalColorScheme::FollowApp,
                highlight_preset: "theme".to_string(),
                highlight_overrides: String::new(),
            }
        );
        assert_eq!(
            service
                .set_terminal_appearance(16, "nocterm_dark", "mobaxterm", "directory=12")
                .expect("save appearance"),
            TerminalAppearance {
                font_size: 16,
                color_scheme: TerminalColorScheme::NoctermDark,
                highlight_preset: "mobaxterm".to_string(),
                highlight_overrides: "directory=12".to_string(),
            }
        );
    }

    #[test]
    fn rejects_invalid_terminal_appearance() {
        let repository = Arc::new(FakeSettingsRepository::default());
        let service = SettingsService::new(repository);

        assert_eq!(
            service
                .set_terminal_appearance(8, "nocterm_dark", "theme", "")
                .expect_err("invalid font size")
                .code,
            "SETTINGS_TERMINAL_FONT_SIZE_INVALID"
        );
        assert_eq!(
            service
                .set_terminal_appearance(13, "nocterm_dark", "custom", "")
                .expect_err("invalid highlight preset")
                .code,
            "SETTINGS_HIGHLIGHT_PRESET_INVALID"
        );
        assert_eq!(
            service
                .set_terminal_appearance(13, "nocterm_dark", "theme", "directory=99")
                .expect_err("invalid override slot")
                .code,
            "SETTINGS_HIGHLIGHT_OVERRIDES_INVALID"
        );
    }
}
