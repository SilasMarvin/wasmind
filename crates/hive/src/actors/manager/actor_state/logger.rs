use crate::actors::manager::hive::actor::logger;

use super::ActorState;

impl logger::Host for ActorState {
    async fn log(&mut self, level: logger::LogLevel, message: String) {
        let actor_context = format!("[{}:{}]", self.actor_id, self.scope);

        match level {
            logger::LogLevel::Debug => tracing::debug!("{} {}", actor_context, message),
            logger::LogLevel::Info => tracing::info!("{} {}", actor_context, message),
            logger::LogLevel::Warn => tracing::warn!("{} {}", actor_context, message),
            logger::LogLevel::Error => tracing::error!("{} {}", actor_context, message),
        }
    }
}
