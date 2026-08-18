mod commands;
mod dto;
mod state;

use std::sync::Arc;

use nocterm_application::{connection::ConnectionService, health::HealthService};
use nocterm_infrastructure::{
    credential::SystemCredentialStore, persistence::SqliteConnectionRepository,
    platform::SystemPlatformProbe,
};
use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let health_service =
        HealthService::new(Arc::new(SystemPlatformProbe), env!("CARGO_PKG_VERSION"));

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let database_path = app.path().app_data_dir()?.join("nocterm.db");
            let repository = Arc::new(
                SqliteConnectionRepository::open(&database_path)
                    .map_err(|error| std::io::Error::other(error.to_string()))?,
            );
            let credential_store = Arc::new(SystemCredentialStore);
            let connection_service =
                ConnectionService::with_credential_store(repository.clone(), credential_store);

            app.manage(AppState::new(health_service, connection_service));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::health::health_check,
            commands::connection::connection_list,
            commands::connection::connection_create,
            commands::connection::connection_update,
            commands::connection::connection_delete,
            commands::connection::connection_group_list,
            commands::connection::connection_group_upsert,
            commands::connection::connection_group_delete,
            commands::connection::connection_reorder,
            commands::connection::connection_backup_import,
            commands::connection::connection_backup_pick_and_read,
            commands::connection::connection_ssh_config_pick_and_read,
            commands::connection::connection_backup_save,
            commands::credential::credential_store,
            commands::credential::credential_delete,
            commands::credential::credential_store_file,
            commands::local_terminal::local_terminal_open,
            commands::local_terminal::local_terminal_write,
            commands::local_terminal::local_terminal_resize,
            commands::local_terminal::local_terminal_close,
            commands::ssh_terminal::ssh_terminal_open,
            commands::ssh_terminal::ssh_terminal_write,
            commands::ssh_terminal::ssh_terminal_resize,
            commands::ssh_terminal::ssh_terminal_close,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Nocterm desktop application");
}
