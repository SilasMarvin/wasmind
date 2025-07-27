use hive_actor_utils_common_messages::tools::ExecuteTool;

pub trait Tool: Sized {
    fn new() -> Self;

    fn handle_call(&mut self, input: ExecuteTool);
}

#[cfg(feature = "macros")]
pub mod macros {
    pub mod __private {
        pub use hive_llm_client;
        pub use serde_json;
    }

    pub use hive_actor_utils_macros::Tool;
}
