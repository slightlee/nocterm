use std::sync::Arc;

use nocterm_domain::settings::{
    AppTheme, DEFAULT_TERMINAL_FONT_SIZE, SettingsRepository, TerminalColorScheme,
    validate_terminal_font_size,
};

use crate::error::AppError;

/// 设置用例集中处理默认值和输入校验，IPC 与持久化层不各自复制业务规则。
#[derive(Clone)]
pub struct SettingsService {
    repository: Arc<dyn SettingsRepository>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalAppearance {
    pub font_size: u8,
    pub color_scheme: TerminalColorScheme,
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
        Ok(TerminalAppearance {
            font_size,
            color_scheme,
        })
    }

    pub fn set_terminal_appearance(
        &self,
        font_size: u8,
        color_scheme: &str,
    ) -> Result<TerminalAppearance, AppError> {
        let font_size = validate_terminal_font_size(font_size)
            .map_err(|error| AppError::new(error.code, error.message, false))?;
        let color_scheme = TerminalColorScheme::parse(color_scheme)
            .map_err(|error| AppError::new(error.code, error.message, false))?;
        self.repository
            .set_terminal_appearance(font_size, color_scheme)
            .map_err(|_| AppError::new("SETTINGS_WRITE_FAILED", "保存终端设置失败", true))?;
        Ok(TerminalAppearance {
            font_size,
            color_scheme,
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

        fn set_terminal_appearance(
            &self,
            font_size: u8,
            color_scheme: TerminalColorScheme,
        ) -> Result<(), SettingsRepositoryError> {
            *self.font_size.lock().expect("font size lock") = Some(font_size);
            *self.color_scheme.lock().expect("color scheme lock") = Some(color_scheme);
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
            }
        );
        assert_eq!(
            service
                .set_terminal_appearance(16, "nocterm_dark")
                .expect("save appearance"),
            TerminalAppearance {
                font_size: 16,
                color_scheme: TerminalColorScheme::NoctermDark,
            }
        );
    }

    #[test]
    fn rejects_invalid_terminal_appearance() {
        let repository = Arc::new(FakeSettingsRepository::default());
        let service = SettingsService::new(repository);

        assert_eq!(
            service
                .set_terminal_appearance(8, "nocterm_dark")
                .expect_err("invalid font size")
                .code,
            "SETTINGS_TERMINAL_FONT_SIZE_INVALID"
        );
    }
}
