use crate::actors::{MessageEnvelope, manager::hive::actor::messaging};

use super::ActorState;

impl messaging::Host for ActorState {
    async fn broadcast(&mut self, message_type: String, payload: Vec<u8>) {
        // Use current message ID as parent, or generate root if no current message
        let id = self.current_message_id
            .as_ref()
            .map(|parent_id| crate::utils::generate_child_correlation_id(parent_id))
            .unwrap_or_else(crate::utils::generate_root_correlation_id);
            
        let _ = self.tx.send(MessageEnvelope {
            id,
            message_type,
            from_actor_id: self.actor_id.to_string(),
            from_scope: self.scope.to_string(),
            payload,
        });
    }
}
