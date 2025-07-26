use crate::actors::{MessageEnvelope, manager::hive::actor::messaging};

use super::ActorState;

impl messaging::Host for ActorState {
    async fn broadcast(&mut self, message_type: String, payload: Vec<u8>) {
        let _ = self.tx.send(MessageEnvelope {
            message_type,
            from_actor_id: self.actor_id.to_string(),
            from_scope: self.scope.to_string(),
            payload,
        });
    }
}
