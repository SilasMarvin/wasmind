pub trait Tool: Sized {
    fn new() -> Self;

    fn handle_call(&self, input: serde_json::Value);
}

#[cfg(feature = "macros")]
pub mod macros {
    pub mod __private {
        pub use serde_json;
    }

    pub use hive_actor_utils_macros::Tool;
}
