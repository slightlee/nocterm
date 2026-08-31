use tauri::State;

use crate::{
    dto::{
        error::ErrorResponse,
        settings::{AppThemeResponse, SetTerminalAppearanceRequest, TerminalAppearanceResponse},
    },
    state::AppState,
};

/// 设置命令保持轻薄：默认值和合法值都由 Application 决定。
#[tauri::command]
pub fn settings_app_theme_get(
    state: State<'_, AppState>,
) -> Result<AppThemeResponse, ErrorResponse> {
    state
        .settings_service()
        .app_theme()
        .map(AppThemeResponse::from)
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub fn settings_app_theme_set(
    state: State<'_, AppState>,
    value: String,
) -> Result<AppThemeResponse, ErrorResponse> {
    state
        .settings_service()
        .set_app_theme(&value)
        .map(AppThemeResponse::from)
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub fn settings_terminal_appearance_get(
    state: State<'_, AppState>,
) -> Result<TerminalAppearanceResponse, ErrorResponse> {
    state
        .settings_service()
        .terminal_appearance()
        .map(TerminalAppearanceResponse::from)
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub fn settings_terminal_appearance_set(
    state: State<'_, AppState>,
    request: SetTerminalAppearanceRequest,
) -> Result<TerminalAppearanceResponse, ErrorResponse> {
    state
        .settings_service()
        .set_terminal_appearance(request.font_size, &request.color_scheme)
        .map(TerminalAppearanceResponse::from)
        .map_err(ErrorResponse::from)
}
