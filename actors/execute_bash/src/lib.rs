use hive_actor_bindings::{Guest, host::send_message};

struct Component;

impl Guest for Component {
    fn add(x: u32, y: u32) -> u32 {
        send_message(x + y);
        x + y
    }
}

hive_actor_bindings::export!(Component with_types_in hive_actor_bindings);
