use nocterm_application::health::HealthService;

pub struct AppState {
    health_service: HealthService,
}

impl AppState {
    pub fn new(health_service: HealthService) -> Self {
        Self { health_service }
    }

    pub fn health_service(&self) -> &HealthService {
        &self.health_service
    }
}
