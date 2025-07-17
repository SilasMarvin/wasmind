use bindings::{
    exports::hive::actor::actor::{Guest, GuestActor, MessageEnvelope},
    hive::actor::messaging::broadcast,
};

#[allow(warnings)]
mod bindings;

struct Component;

struct Actor {
    scope: String,
}

impl Guest for Component {
    type Actor = Actor;
}

impl GuestActor for Actor {
    fn new(scope: String) -> Self {
        Self { scope }
    }

    fn handle_message(&self, message: MessageEnvelope) -> () {
        broadcast("TEST2".as_bytes());
    }

    fn destructor(&self) -> () {}
}

bindings::export!(Component with_types_in bindings);
