use crate::actors::actor_manager::hive::actor::messaging;

use super::ActorState;

impl messaging::Host for ActorState {
    async fn broadcast(&mut self, message_type: String, payload: Vec<u8>) {
        let string = String::from_utf8(payload).unwrap();
        println!("GOT PAYLOAD: {}", string);
    }
}
