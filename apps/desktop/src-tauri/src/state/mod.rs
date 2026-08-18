use nocterm_application::{
    connection::ConnectionService,
    health::HealthService,
    terminal::{LocalTerminalService, TerminalService},
};
use nocterm_infrastructure::terminal::{LocalTerminalManager, SshTerminalManager};
use std::sync::Arc;

pub struct AppState {
    health_service: HealthService,
    connection_service: ConnectionService,
    terminal_service: TerminalService,
    local_terminal_service: LocalTerminalService,
}

impl AppState {
    pub fn new(health_service: HealthService, connection_service: ConnectionService) -> Self {
        Self {
            health_service,
            connection_service,
            terminal_service: TerminalService::new(Arc::new(SshTerminalManager::default())),
            local_terminal_service: LocalTerminalService::new(Arc::new(
                LocalTerminalManager::default(),
            )),
        }
    }

    pub fn health_service(&self) -> &HealthService {
        &self.health_service
    }

    pub fn connection_service(&self) -> &ConnectionService {
        &self.connection_service
    }

    pub fn terminal_service(&self) -> &TerminalService {
        &self.terminal_service
    }

    pub fn local_terminal_service(&self) -> &LocalTerminalService {
        &self.local_terminal_service
    }
}
