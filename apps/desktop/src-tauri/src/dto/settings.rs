use nocterm_application::settings::TerminalAppearance;
use nocterm_domain::settings::AppTheme;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppThemeResponse {
    value: &'static str,
}

impl From<AppTheme> for AppThemeResponse {
    fn from(value: AppTheme) -> Self {
        Self {
            value: value.as_str(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalAppearanceResponse {
    font_size: u8,
    color_scheme: &'static str,
}

impl From<TerminalAppearance> for TerminalAppearanceResponse {
    fn from(value: TerminalAppearance) -> Self {
        Self {
            font_size: value.font_size,
            color_scheme: value.color_scheme.as_str(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTerminalAppearanceRequest {
    pub font_size: u8,
    pub color_scheme: String,
}
