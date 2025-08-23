pub mod actors;
pub mod messages;
pub mod tools;
pub mod utils;

pub use wasmind_actor_utils_common_messages as common_messages;
pub use wasmind_llm_types as llm_client_types;

/// The starting scope for the root agent
/// This is used by the wasmind library and is always the first scope
pub const STARTING_SCOPE: &str = "000000";
