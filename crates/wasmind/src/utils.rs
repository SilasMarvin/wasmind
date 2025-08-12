use crate::actors::MessageEnvelope;
use rand::Rng;
use serde::de::DeserializeOwned;
use wasmind_actor_utils::messages::Message;

/// Parse a MessageEnvelope as a specific common message type
///
/// This function checks that the message_type matches the expected common message type
/// and attempts to deserialize the payload as the specified type.
///
/// # Arguments
/// * `envelope` - The MessageEnvelope to parse
///
/// # Returns
/// * `Some(T)` if the message type matches and deserialization succeeds
/// * `None` if the message type doesn't match or deserialization fails
pub fn parse_common_message_as<T>(envelope: &MessageEnvelope) -> Option<T>
where
    T: DeserializeOwned + Message,
{
    if envelope.message_type != T::MESSAGE_TYPE {
        return None;
    }

    serde_json::from_slice(&envelope.payload).ok()
}

/// Generate a short random ID for correlation (6 alphanumeric characters)
fn generate_short_id() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::rng();
    (0..6)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

/// Generate a root correlation ID (format: "root:abc123")
pub fn generate_root_correlation_id() -> String {
    format!("root:{}", generate_short_id())
}

/// Generate a child correlation ID from a parent ID (format: "parent:child")
pub fn generate_child_correlation_id(parent_id: &str) -> String {
    let parent_child_id = parent_id.split(':').next_back().unwrap_or(parent_id);
    format!("{}:{}", parent_child_id, generate_short_id())
}

#[cfg(test)]
mod tests {
    use super::*;

    use wasmind_actor_utils::common_messages::actors::AgentSpawned;

    #[test]
    fn test_parse_common_message_as_success() {
        let agent_spawned = AgentSpawned {
            agent_id: "test-agent".to_string(),
            name: "Test Agent".to_string(),
            parent_agent: None,
            actors: vec!["actor1".to_string(), "actor2".to_string()],
        };

        let envelope = MessageEnvelope {
            id: "root:test123".to_string(),
            from_actor_id: "test".to_string(),
            from_scope: "test-scope".to_string(),
            message_type: AgentSpawned::MESSAGE_TYPE.to_string(),
            payload: serde_json::to_vec(&agent_spawned).unwrap(),
        };

        let parsed: Option<AgentSpawned> = parse_common_message_as(&envelope);
        assert!(parsed.is_some());

        let parsed = parsed.unwrap();
        assert_eq!(parsed.agent_id, "test-agent");
        assert_eq!(parsed.name, "Test Agent");
        assert_eq!(parsed.actors, vec!["actor1", "actor2"]);
    }

    #[test]
    fn test_parse_common_message_as_wrong_message_type() {
        let envelope = MessageEnvelope {
            id: "root:test456".to_string(),
            from_actor_id: "test".to_string(),
            from_scope: "test-scope".to_string(),
            message_type: "wrong.message.type".to_string(),
            payload: vec![],
        };

        let parsed: Option<AgentSpawned> = parse_common_message_as(&envelope);
        assert!(parsed.is_none());
    }

    #[test]
    fn test_parse_common_message_as_invalid_payload() {
        let envelope = MessageEnvelope {
            id: "root:test789".to_string(),
            from_actor_id: "test".to_string(),
            from_scope: "test-scope".to_string(),
            message_type: AgentSpawned::MESSAGE_TYPE.to_string(),
            payload: b"invalid json".to_vec(),
        };

        let parsed: Option<AgentSpawned> = parse_common_message_as(&envelope);
        assert!(parsed.is_none());
    }

    #[test]
    fn test_generate_root_correlation_id() {
        let root_id = generate_root_correlation_id();
        assert!(root_id.starts_with("root:"));
        let parts: Vec<&str> = root_id.split(':').collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], "root");
        assert_eq!(parts[1].len(), 6);
    }

    #[test]
    fn test_generate_child_correlation_id() {
        // Test with root parent
        let child_id = generate_child_correlation_id("root:abc123");
        let parts: Vec<&str> = child_id.split(':').collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], "abc123");
        assert_eq!(parts[1].len(), 6);

        // Test with nested parent
        let grandchild_id = generate_child_correlation_id(&child_id);
        let parts2: Vec<&str> = grandchild_id.split(':').collect();
        assert_eq!(parts2.len(), 2);
        assert_eq!(parts2[0], parts[1]);
        assert_eq!(parts2[1].len(), 6);
    }
}
