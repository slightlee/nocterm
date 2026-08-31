pub mod session_password;

use nocterm_application::{
    connection::ConnectionService,
    health::HealthService,
    settings::SettingsService,
    terminal::{LocalTerminalService, TerminalService},
};
use nocterm_infrastructure::{
    ssh::{SshTerminalManager, sftp::SftpManager},
    terminal::LocalTerminalManager,
};
use std::sync::Arc;

use self::session_password::SessionPasswords;

pub struct AppState {
    health_service: HealthService,
    connection_service: ConnectionService,
    settings_service: SettingsService,
    terminal_service: TerminalService,
    local_terminal_service: LocalTerminalService,
    /// 进程内 SFTP 会话管理器：与终端共用 russh 后端，承载远程文件浏览与传输。
    sftp_manager: Arc<SftpManager>,
    /// 终端交互输入的登录口令缓存，仅内存驻留，供同连接的其他标签与 SFTP 复用。
    /// 用 `Arc` 是为了让终端的输出线程能持有一份句柄，在会话收尾时归还租约。
    session_passwords: Arc<SessionPasswords>,
}

impl AppState {
    pub fn new(
        health_service: HealthService,
        connection_service: ConnectionService,
        settings_service: SettingsService,
    ) -> Self {
        Self {
            health_service,
            connection_service,
            settings_service,
            terminal_service: TerminalService::new(Arc::new(SshTerminalManager::default())),
            local_terminal_service: LocalTerminalService::new(Arc::new(
                LocalTerminalManager::default(),
            )),
            sftp_manager: Arc::new(SftpManager::default()),
            session_passwords: Arc::new(SessionPasswords::default()),
        }
    }

    pub fn health_service(&self) -> &HealthService {
        &self.health_service
    }

    pub fn connection_service(&self) -> &ConnectionService {
        &self.connection_service
    }

    pub fn settings_service(&self) -> &SettingsService {
        &self.settings_service
    }

    pub fn terminal_service(&self) -> &TerminalService {
        &self.terminal_service
    }

    pub fn local_terminal_service(&self) -> &LocalTerminalService {
        &self.local_terminal_service
    }

    /// 返回共享的 SFTP 会话管理器，供远程文件命令复用同一会话池。
    pub fn sftp_manager(&self) -> &Arc<SftpManager> {
        &self.sftp_manager
    }

    /// 返回会话级口令缓存：终端交互输入的口令由此供 SFTP 与后续终端标签复用。
    /// 返回 `Arc` 引用而非裸引用，调用方可克隆出句柄交给会话线程归还租约。
    pub fn session_passwords(&self) -> &Arc<SessionPasswords> {
        &self.session_passwords
    }
}
