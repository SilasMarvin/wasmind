use crate::actors::MessageEnvelope;
use hive_actor_utils_common_messages::Message;
use serde::de::DeserializeOwned;

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
    // Check if message type matches
    if envelope.message_type != T::MESSAGE_TYPE {
        return None;
    }

    // Try to deserialize the payload
    serde_json::from_slice(&envelope.payload).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hive_actor_utils_common_messages::actors::AgentSpawned;

    #[test]
    fn test_parse_common_message_as_success() {
        let agent_spawned = AgentSpawned {
            agent_id: "test-agent".to_string(),
            name: "Test Agent".to_string(),
            parent_agent: None,
            actors: vec!["actor1".to_string(), "actor2".to_string()],
        };

        let envelope = MessageEnvelope {
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
            from_actor_id: "test".to_string(),
            from_scope: "test-scope".to_string(),
            message_type: AgentSpawned::MESSAGE_TYPE.to_string(),
            payload: b"invalid json".to_vec(),
        };

        let parsed: Option<AgentSpawned> = parse_common_message_as(&envelope);
        assert!(parsed.is_none());
    }
}
