// Simple test actor for build testing
wit_bindgen::generate!({
    world: "actor-world",
    path: "../../../hive_actor_bindings/wit",
});

use exports::hive::actor::actor::{Guest, GuestActor, MessageEnvelope};

struct TestActor;

impl GuestActor for TestActor {
    fn new(_scope: String, _config: String) -> Self {
        TestActor
    }

    fn handle_message(&self, _message: MessageEnvelope) {
        // Do nothing
    }

    fn destructor(&self) {
        // Do nothing
    }
}

struct Component;

impl Guest for Component {
    type Actor = TestActor;
}

export!(Component with_types_in crate);