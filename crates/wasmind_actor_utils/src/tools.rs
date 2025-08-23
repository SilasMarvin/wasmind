use wasmind_actor_utils_common_messages::tools::ExecuteTool;

pub trait Tool: Sized {
    fn new(scope: String, config: String) -> Self;

    fn handle_call(&mut self, input: ExecuteTool);
}

#[cfg(feature = "macros")]
pub mod macros {
    pub mod __private {
        pub use serde_json;
        pub use wasmind_llm_types;
    }

    pub use wasmind_actor_utils_macros::Tool;
}
