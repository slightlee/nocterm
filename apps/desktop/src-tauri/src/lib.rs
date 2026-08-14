mod commands;
mod dto;
mod state;

use std::sync::Arc;

use nocterm_application::health::HealthService;
use nocterm_infrastructure::platform::SystemPlatformProbe;
use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let health_service =
        HealthService::new(Arc::new(SystemPlatformProbe), env!("CARGO_PKG_VERSION"));

    tauri::Builder::default()
        .manage(AppState::new(health_service))
        .invoke_handler(tauri::generate_handler![commands::health::health_check])
        .run(tauri::generate_context!())
        .expect("failed to run Nocterm desktop application");
}
