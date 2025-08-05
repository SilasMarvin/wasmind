pub mod actors;
pub mod messages;
pub mod tools;
pub mod utils;

pub use hive_actor_utils_common_messages as common_messages;
pub use hive_llm_types::types as llm_client_types;

/// The starting scope for the root agent
/// This is used by the hive library and is always the first scope
pub const STARTING_SCOPE: &str = "000000";
