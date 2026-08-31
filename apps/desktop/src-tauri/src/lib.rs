mod commands;
mod dto;
mod state;

use std::sync::Arc;

use commands::sftp::{SftpTransferState, shutdown_sftp};
use nocterm_application::{
    connection::ConnectionService, health::HealthService, settings::SettingsService,
};
use nocterm_infrastructure::{
    credential::SystemCredentialStore, persistence::SqliteConnectionRepository,
    platform::SystemPlatformProbe,
};
use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let database_path = app.path().app_data_dir()?.join("nocterm.db");
            let repository = Arc::new(
                SqliteConnectionRepository::open(&database_path)
                    .map_err(|error| std::io::Error::other(error.to_string()))?,
            );
            let credential_store = Arc::new(SystemCredentialStore::default());
            let connection_service =
                ConnectionService::with_credential_store(repository.clone(), credential_store);
            let settings_service = SettingsService::new(repository.clone());
            // 产品版本由 Tauri 配置解析根 package.json；Cargo crate 版本仅描述内部包。
            let health_service = HealthService::new(
                Arc::new(SystemPlatformProbe),
                app.package_info().version.to_string(),
            );

            app.manage(AppState::new(
                health_service,
                connection_service,
                settings_service,
            ));
            app.manage(SftpTransferState::default());
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
            commands::credential::connection_bind_private_key,
            commands::settings::settings_app_theme_get,
            commands::settings::settings_app_theme_set,
            commands::settings::settings_terminal_appearance_get,
            commands::settings::settings_terminal_appearance_set,
            commands::local_terminal::local_terminal_open,
            commands::local_terminal::local_terminal_write,
            commands::local_terminal::local_terminal_resize,
            commands::local_terminal::local_terminal_close,
            commands::ssh_terminal::ssh_terminal_open,
            commands::ssh_terminal::ssh_terminal_write,
            commands::ssh_terminal::ssh_terminal_resize,
            commands::ssh_terminal::ssh_terminal_close,
            commands::sftp::list_local_dir,
            commands::sftp::list_remote_dir,
            commands::sftp::local_path_exists,
            commands::sftp::remote_path_exists,
            commands::sftp::create_local_dir,
            commands::sftp::create_remote_dir,
            commands::sftp::rename_local_path,
            commands::sftp::rename_remote_path,
            commands::sftp::delete_local_path,
            commands::sftp::delete_remote_path,
            commands::sftp::upload_local_to_remote,
            commands::sftp::download_remote_to_local,
            commands::sftp::cancel_file_transfer,
            commands::sftp::close_sftp_session,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Nocterm desktop application");
    app.run(|app_handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            shutdown_sftp(app_handle);
        }
    });
}
