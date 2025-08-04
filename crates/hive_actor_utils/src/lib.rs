pub mod actors;
pub mod messages;
pub mod tools;

pub use hive_actor_utils_common_messages as common_messages;
pub use hive_llm_types::types as llm_client_types;

/// Generate a random alphanumeric ID of the specified length
pub fn generate_id(len: usize) -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::rng();
    (0..len)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}
