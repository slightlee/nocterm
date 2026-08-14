use tauri::{AppHandle, Emitter, State};

use crate::{dto::health::HealthResponse, state::AppState};

const HEALTH_EVENT: &str = "nocterm://health-checked";

#[tauri::command]
pub fn health_check(app: AppHandle, state: State<'_, AppState>) -> HealthResponse {
    let response = HealthResponse::from(state.health_service().check());

    if let Err(error) = app.emit(HEALTH_EVENT, &response) {
        eprintln!("failed to emit health event: {error}");
    }

    response
}
