use bindings::{
    exports::hive::actor::actor_interface::{Guest, GuestActor},
    hive::actor::runtime_interface::broadcast,
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

    fn handle_message(
        &self,
        message: bindings::exports::hive::actor::actor_interface::MessageEnvelope,
    ) -> () {
        broadcast("TEST2".as_bytes());
    }

    fn destructor(&self) -> () {}
}

bindings::export!(Component with_types_in bindings);
