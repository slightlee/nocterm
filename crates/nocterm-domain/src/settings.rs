use std::{error::Error, fmt};

/// 应用主题保存的是用户偏好，而不是当前解析后的明暗结果。
/// `System` 会继续响应操作系统主题变化，避免把一次系统状态固化为用户选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTheme {
    System,
    Light,
    Dark,
}

pub const DEFAULT_TERMINAL_FONT_SIZE: u8 = 13;
pub const MIN_TERMINAL_FONT_SIZE: u8 = 10;
pub const MAX_TERMINAL_FONT_SIZE: u8 = 24;

/// 终端配色保存稳定标识；`FollowApp` 在展示时才解析成当前应用明暗主题。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalColorScheme {
    FollowApp,
    NoctermLight,
    NoctermDark,
    Midnight,
    Graphite,
    Forest,
    Amber,
    SolarizedDark,
    Dracula,
    Monokai,
    Nord,
    GruvboxDark,
    TokyoNight,
    OneDark,
    CatppuccinMocha,
    MaterialOcean,
}

impl TerminalColorScheme {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FollowApp => "follow_app",
            Self::NoctermLight => "nocterm_light",
            Self::NoctermDark => "nocterm_dark",
            Self::Midnight => "midnight",
            Self::Graphite => "graphite",
            Self::Forest => "forest",
            Self::Amber => "amber",
            Self::SolarizedDark => "solarized_dark",
            Self::Dracula => "dracula",
            Self::Monokai => "monokai",
            Self::Nord => "nord",
            Self::GruvboxDark => "gruvbox_dark",
            Self::TokyoNight => "tokyo_night",
            Self::OneDark => "one_dark",
            Self::CatppuccinMocha => "catppuccin_mocha",
            Self::MaterialOcean => "material_ocean",
        }
    }

    pub fn parse(value: &str) -> Result<Self, SettingsValidationError> {
        match value {
            "follow_app" => Ok(Self::FollowApp),
            "nocterm_light" => Ok(Self::NoctermLight),
            "nocterm_dark" => Ok(Self::NoctermDark),
            "midnight" => Ok(Self::Midnight),
            "graphite" => Ok(Self::Graphite),
            "forest" => Ok(Self::Forest),
            "amber" => Ok(Self::Amber),
            "solarized_dark" => Ok(Self::SolarizedDark),
            "dracula" => Ok(Self::Dracula),
            "monokai" => Ok(Self::Monokai),
            "nord" => Ok(Self::Nord),
            "gruvbox_dark" => Ok(Self::GruvboxDark),
            "tokyo_night" => Ok(Self::TokyoNight),
            "one_dark" => Ok(Self::OneDark),
            "catppuccin_mocha" => Ok(Self::CatppuccinMocha),
            "material_ocean" => Ok(Self::MaterialOcean),
            _ => Err(SettingsValidationError::new(
                "SETTINGS_TERMINAL_COLOR_SCHEME_INVALID",
                "请选择受支持的终端配色",
            )),
        }
    }
}

pub fn validate_terminal_font_size(value: u8) -> Result<u8, SettingsValidationError> {
    if (MIN_TERMINAL_FONT_SIZE..=MAX_TERMINAL_FONT_SIZE).contains(&value) {
        return Ok(value);
    }
    Err(SettingsValidationError::new(
        "SETTINGS_TERMINAL_FONT_SIZE_INVALID",
        "终端字号必须在 10 到 24 之间",
    ))
}

impl AppTheme {
    /// 返回跨 SQLite 与 IPC 使用的稳定代码，展示文案留在 UI 层。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn parse(value: &str) -> Result<Self, SettingsValidationError> {
        match value {
            "system" => Ok(Self::System),
            "light" => Ok(Self::Light),
            "dark" => Ok(Self::Dark),
            _ => Err(SettingsValidationError::new(
                "SETTINGS_APP_THEME_INVALID",
                "请选择受支持的应用主题",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsValidationError {
    pub code: &'static str,
    pub message: &'static str,
}

impl SettingsValidationError {
    const fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }
}

impl fmt::Display for SettingsValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl Error for SettingsValidationError {}

/// 设置仓储只暴露类型安全的偏好；键名和值的存储方式属于 Infrastructure。
pub trait SettingsRepository: Send + Sync {
    fn app_theme(&self) -> Result<Option<AppTheme>, SettingsRepositoryError>;
    fn set_app_theme(&self, theme: AppTheme) -> Result<(), SettingsRepositoryError>;
    fn terminal_font_size(&self) -> Result<Option<u8>, SettingsRepositoryError>;
    fn terminal_color_scheme(&self)
    -> Result<Option<TerminalColorScheme>, SettingsRepositoryError>;
    fn set_terminal_appearance(
        &self,
        font_size: u8,
        color_scheme: TerminalColorScheme,
    ) -> Result<(), SettingsRepositoryError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsRepositoryError {
    message: String,
}

impl SettingsRepositoryError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SettingsRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for SettingsRepositoryError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_supported_app_themes() {
        assert_eq!(AppTheme::parse("system"), Ok(AppTheme::System));
        assert_eq!(AppTheme::parse("light"), Ok(AppTheme::Light));
        assert_eq!(AppTheme::parse("dark"), Ok(AppTheme::Dark));
        assert_eq!(
            AppTheme::parse("contrast")
                .expect_err("unsupported theme")
                .code,
            "SETTINGS_APP_THEME_INVALID"
        );
    }

    #[test]
    fn validates_terminal_preferences() {
        assert_eq!(validate_terminal_font_size(13), Ok(13));
        assert!(validate_terminal_font_size(9).is_err());
        assert_eq!(
            TerminalColorScheme::parse("nocterm_dark"),
            Ok(TerminalColorScheme::NoctermDark)
        );
        assert_eq!(
            TerminalColorScheme::parse("midnight"),
            Ok(TerminalColorScheme::Midnight)
        );
        assert_eq!(
            TerminalColorScheme::parse("graphite"),
            Ok(TerminalColorScheme::Graphite)
        );
        assert_eq!(
            TerminalColorScheme::parse("forest"),
            Ok(TerminalColorScheme::Forest)
        );
        assert_eq!(
            TerminalColorScheme::parse("amber"),
            Ok(TerminalColorScheme::Amber)
        );
        assert_eq!(
            TerminalColorScheme::parse("solarized_dark"),
            Ok(TerminalColorScheme::SolarizedDark)
        );
        assert_eq!(
            TerminalColorScheme::parse("dracula"),
            Ok(TerminalColorScheme::Dracula)
        );
        assert_eq!(
            TerminalColorScheme::parse("monokai"),
            Ok(TerminalColorScheme::Monokai)
        );
        assert_eq!(
            TerminalColorScheme::parse("nord"),
            Ok(TerminalColorScheme::Nord)
        );
        assert_eq!(
            TerminalColorScheme::parse("gruvbox_dark"),
            Ok(TerminalColorScheme::GruvboxDark)
        );
        assert_eq!(
            TerminalColorScheme::parse("tokyo_night"),
            Ok(TerminalColorScheme::TokyoNight)
        );
        assert_eq!(
            TerminalColorScheme::parse("one_dark"),
            Ok(TerminalColorScheme::OneDark)
        );
        assert_eq!(
            TerminalColorScheme::parse("catppuccin_mocha"),
            Ok(TerminalColorScheme::CatppuccinMocha)
        );
        assert_eq!(
            TerminalColorScheme::parse("material_ocean"),
            Ok(TerminalColorScheme::MaterialOcean)
        );
        assert!(TerminalColorScheme::parse("unknown").is_err());
    }
}
