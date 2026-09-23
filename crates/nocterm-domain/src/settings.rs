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
    MobaXtermVivid,
    CatppuccinLatte,
    CatppuccinFrappe,
    CatppuccinMacchiato,
    RosePine,
    RosePineDawn,
    RosePineMoon,
    EverforestDark,
    Kanagawa,
    AyuDark,
    AyuLight,
    OxocarbonDark,
    OneHalfLight,
    GithubLight,
    Synthwave84,
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
            Self::MobaXtermVivid => "mobaxterm_vivid",
            Self::CatppuccinLatte => "catppuccin_latte",
            Self::CatppuccinFrappe => "catppuccin_frappe",
            Self::CatppuccinMacchiato => "catppuccin_macchiato",
            Self::RosePine => "rose_pine",
            Self::RosePineDawn => "rose_pine_dawn",
            Self::RosePineMoon => "rose_pine_moon",
            Self::EverforestDark => "everforest_dark",
            Self::Kanagawa => "kanagawa",
            Self::AyuDark => "ayu_dark",
            Self::AyuLight => "ayu_light",
            Self::OxocarbonDark => "oxocarbon_dark",
            Self::OneHalfLight => "one_half_light",
            Self::GithubLight => "github_light",
            Self::Synthwave84 => "synthwave_84",
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
            "mobaxterm_vivid" => Ok(Self::MobaXtermVivid),
            "catppuccin_latte" => Ok(Self::CatppuccinLatte),
            "catppuccin_frappe" => Ok(Self::CatppuccinFrappe),
            "catppuccin_macchiato" => Ok(Self::CatppuccinMacchiato),
            "rose_pine" => Ok(Self::RosePine),
            "rose_pine_dawn" => Ok(Self::RosePineDawn),
            "rose_pine_moon" => Ok(Self::RosePineMoon),
            "everforest_dark" => Ok(Self::EverforestDark),
            "kanagawa" => Ok(Self::Kanagawa),
            "ayu_dark" => Ok(Self::AyuDark),
            "ayu_light" => Ok(Self::AyuLight),
            "oxocarbon_dark" => Ok(Self::OxocarbonDark),
            "one_half_light" => Ok(Self::OneHalfLight),
            "github_light" => Ok(Self::GithubLight),
            "synthwave_84" => Ok(Self::Synthwave84),
            _ => Err(SettingsValidationError::new(
                "SETTINGS_TERMINAL_COLOR_SCHEME_INVALID",
                "请选择受支持的终端配色",
            )),
        }
    }
}

/// 终端字体高亮的语义配色预设；`Theme` 即当前主题的出厂角色配色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightPreset {
    Theme,
    MobaXterm,
    HighContrast,
}

impl HighlightPreset {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Theme => "theme",
            Self::MobaXterm => "mobaxterm",
            Self::HighContrast => "high_contrast",
        }
    }

    pub fn parse(value: &str) -> Result<Self, SettingsValidationError> {
        match value {
            "theme" => Ok(Self::Theme),
            "mobaxterm" => Ok(Self::MobaXterm),
            "high_contrast" => Ok(Self::HighContrast),
            _ => Err(SettingsValidationError::new(
                "SETTINGS_HIGHLIGHT_PRESET_INVALID",
                "请选择受支持的字体配色预设",
            )),
        }
    }
}

/// 可自定义颜色的语义角色，前后端共用同一份稳定标识。
pub const HIGHLIGHT_ROLE_IDS: [&str; 18] = [
    "prompt_user_host",
    "prompt_path",
    "permissions",
    "date",
    "directory",
    "executable",
    "symlink",
    "device",
    "archive",
    "log",
    "config",
    "script",
    "media",
    "kw_error",
    "kw_warn",
    "kw_success",
    "kw_info",
    "ip",
];

pub fn is_valid_highlight_role(value: &str) -> bool {
    HIGHLIGHT_ROLE_IDS.contains(&value)
}

/// 角色颜色存 ANSI 色槽（0-15），最终 RGB 由主题调色板翻译。
pub const MAX_HIGHLIGHT_SLOT: u8 = 15;

/// 覆盖项的紧凑序列化：`role=slot` 以 `,` 相连。domain 保持零依赖，
/// 不引入 JSON，解析与校验都在这里完成。
pub fn parse_highlight_overrides(
    value: &str,
) -> Result<Vec<(&'static str, u8)>, SettingsValidationError> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for part in value.split(',') {
        let Some((role, slot)) = part.split_once('=') else {
            return Err(invalid_overrides());
        };
        let Some(role_id) = HIGHLIGHT_ROLE_IDS.iter().find(|id| **id == role) else {
            return Err(invalid_overrides());
        };
        let Ok(slot) = slot.parse::<u8>() else {
            return Err(invalid_overrides());
        };
        if slot > MAX_HIGHLIGHT_SLOT {
            return Err(invalid_overrides());
        }
        entries.push((*role_id, slot));
    }
    Ok(entries)
}

fn invalid_overrides() -> SettingsValidationError {
    SettingsValidationError::new(
        "SETTINGS_HIGHLIGHT_OVERRIDES_INVALID",
        "字体颜色覆盖项格式不正确",
    )
}

/// 归一化：同一角色后者覆盖前者，按角色标识排序，保证同输入同存储。
pub fn canonicalize_highlight_overrides(entries: &[(&'static str, u8)]) -> String {
    let mut deduped: Vec<(&'static str, u8)> = Vec::new();
    for (role, slot) in entries {
        if let Some(existing) = deduped.iter_mut().find(|(r, _)| r == role) {
            existing.1 = *slot;
        } else {
            deduped.push((role, *slot));
        }
    }
    deduped.sort_unstable_by_key(|(role, _)| *role);
    deduped
        .iter()
        .map(|(role, slot)| format!("{role}={slot}"))
        .collect::<Vec<_>>()
        .join(",")
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
    fn highlight_preset(&self) -> Result<Option<HighlightPreset>, SettingsRepositoryError>;
    fn highlight_overrides(&self) -> Result<Option<String>, SettingsRepositoryError>;
    fn set_terminal_appearance(
        &self,
        font_size: u8,
        color_scheme: TerminalColorScheme,
        highlight_preset: HighlightPreset,
        highlight_overrides: &str,
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
        assert_eq!(
            TerminalColorScheme::parse("mobaxterm_vivid"),
            Ok(TerminalColorScheme::MobaXtermVivid)
        );
        assert_eq!(
            TerminalColorScheme::parse("catppuccin_latte"),
            Ok(TerminalColorScheme::CatppuccinLatte)
        );
        assert_eq!(
            TerminalColorScheme::parse("catppuccin_frappe"),
            Ok(TerminalColorScheme::CatppuccinFrappe)
        );
        assert_eq!(
            TerminalColorScheme::parse("catppuccin_macchiato"),
            Ok(TerminalColorScheme::CatppuccinMacchiato)
        );
        assert_eq!(
            TerminalColorScheme::parse("rose_pine"),
            Ok(TerminalColorScheme::RosePine)
        );
        assert_eq!(
            TerminalColorScheme::parse("rose_pine_dawn"),
            Ok(TerminalColorScheme::RosePineDawn)
        );
        assert_eq!(
            TerminalColorScheme::parse("rose_pine_moon"),
            Ok(TerminalColorScheme::RosePineMoon)
        );
        assert_eq!(
            TerminalColorScheme::parse("everforest_dark"),
            Ok(TerminalColorScheme::EverforestDark)
        );
        assert_eq!(
            TerminalColorScheme::parse("kanagawa"),
            Ok(TerminalColorScheme::Kanagawa)
        );
        assert_eq!(
            TerminalColorScheme::parse("ayu_dark"),
            Ok(TerminalColorScheme::AyuDark)
        );
        assert_eq!(
            TerminalColorScheme::parse("ayu_light"),
            Ok(TerminalColorScheme::AyuLight)
        );
        assert_eq!(
            TerminalColorScheme::parse("oxocarbon_dark"),
            Ok(TerminalColorScheme::OxocarbonDark)
        );
        assert_eq!(
            TerminalColorScheme::parse("one_half_light"),
            Ok(TerminalColorScheme::OneHalfLight)
        );
        assert_eq!(
            TerminalColorScheme::parse("github_light"),
            Ok(TerminalColorScheme::GithubLight)
        );
        assert_eq!(
            TerminalColorScheme::parse("synthwave_84"),
            Ok(TerminalColorScheme::Synthwave84)
        );
        assert!(TerminalColorScheme::parse("unknown").is_err());
    }

    #[test]
    fn parses_highlight_presets_and_rejects_unknown() {
        assert_eq!(HighlightPreset::parse("theme"), Ok(HighlightPreset::Theme));
        assert_eq!(
            HighlightPreset::parse("mobaxterm"),
            Ok(HighlightPreset::MobaXterm)
        );
        assert_eq!(
            HighlightPreset::parse("high_contrast"),
            Ok(HighlightPreset::HighContrast)
        );
        assert_eq!(HighlightPreset::Theme.as_str(), "theme");
        assert_eq!(
            HighlightPreset::parse("custom")
                .expect_err("unsupported preset")
                .code,
            "SETTINGS_HIGHLIGHT_PRESET_INVALID"
        );
    }

    #[test]
    fn parses_and_canonicalizes_highlight_overrides() {
        assert_eq!(parse_highlight_overrides(""), Ok(Vec::new()));
        assert_eq!(
            parse_highlight_overrides("directory=4,kw_error=9"),
            Ok(vec![("directory", 4), ("kw_error", 9)])
        );
        assert!(parse_highlight_overrides("unknown_role=3").is_err());
        assert!(parse_highlight_overrides("directory=16").is_err());
        assert!(parse_highlight_overrides("directory=17").is_err());
        assert!(parse_highlight_overrides("directory").is_err());
        assert_eq!(
            canonicalize_highlight_overrides(&[
                ("kw_error", 9),
                ("directory", 1),
                ("directory", 4)
            ]),
            "directory=4,kw_error=9"
        );
    }
}
