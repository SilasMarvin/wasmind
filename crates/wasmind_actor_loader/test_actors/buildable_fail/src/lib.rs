// Test actor with intentional compile errors
wit_bindgen::generate!({
    world: "actor-world",
    path: "../../../wasmind_actor_bindings/wit",
});

use exports::wasmind::actor::actor::{Guest, GuestActor, MessageEnvelope};

struct TestActor;

impl GuestActor for TestActor {
    fn new(_scope: String, _config: String) -> Self {
        // Intentional compile error: missing semicolon and undefined variable
        let x = undefined_variable
        TestActor
    }

    fn handle_message(&self, _message: MessageEnvelope) {
        // Another compile error: syntax error with missing closing brace
        if true {
            return;
        // Missing closing brace intentionally
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