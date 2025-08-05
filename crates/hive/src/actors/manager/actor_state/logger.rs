use crate::actors::manager::hive::actor::logger;

use super::ActorState;

impl logger::Host for ActorState {
    async fn log(&mut self, level: logger::LogLevel, message: String) {
        let actor_context = format!("[{}:{}]", self.actor_id, self.scope);
        let correlation_id = self.current_message_id.as_deref().unwrap_or("no_msg");
        
        match level {
            logger::LogLevel::Debug => tracing::debug!(
                correlation_id = correlation_id,
                "{} {}", 
                actor_context, 
                message
            ),
            logger::LogLevel::Info => tracing::info!(
                correlation_id = correlation_id,
                "{} {}", 
                actor_context, 
                message
            ),
            logger::LogLevel::Warn => tracing::warn!(
                correlation_id = correlation_id,
                "{} {}", 
                actor_context, 
                message
            ),
            logger::LogLevel::Error => tracing::error!(
                correlation_id = correlation_id,
                "{} {}", 
                actor_context, 
                message
            ),
        }
    }
}
