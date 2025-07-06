use crate::{actors::tools::wait::WAIT_TOOL_NAME, llm_client::ChatMessage};
use std::collections::HashSet;

/// Filters out wait tool calls from chat messages.
///
/// This function removes pairs of messages where:
/// 1. An assistant message contains only one tool call invoking `wait`
/// 2. A corresponding tool response message for that wait call exists
///
/// The function properly matches tool calls by ID regardless of message ordering.
///
/// # Arguments
/// * `messages` - A slice of ChatMessage to filter
///
/// # Returns
/// A Vec<ChatMessage> with wait tool calls filtered out
pub fn filter_wait_tool_calls(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    // First pass: identify wait tool call IDs that should be filtered
    let mut wait_call_ids_to_filter = HashSet::new();

    // Find assistant messages with single wait tool calls
    for message in messages {
        if let ChatMessage::Assistant { tool_calls, .. } = message {
            if let Some(calls) = tool_calls {
                if calls.len() == 1 && calls[0].function.name == WAIT_TOOL_NAME {
                    wait_call_ids_to_filter.insert(calls[0].id.clone());
                }
            }
        }
    }

    // Second pass: verify these wait calls have corresponding tool responses
    let mut confirmed_wait_pairs = HashSet::new();
    for message in messages {
        if let ChatMessage::Tool {
            tool_call_id,
            name,
            content,
        } = message
        {
            if name == WAIT_TOOL_NAME
                && wait_call_ids_to_filter.contains(tool_call_id)
                && !content.to_lowercase().contains("error")
            {
                confirmed_wait_pairs.insert(tool_call_id.clone());
            }
        }
    }

    // Third pass: filter out confirmed wait tool call pairs
    messages
        .iter()
        .filter(|message| match message {
            ChatMessage::Assistant { tool_calls, .. } => {
                if let Some(calls) = tool_calls {
                    // Only filter if it's a single wait call that has a confirmed pair
                    !(calls.len() == 1
                        && calls[0].function.name == WAIT_TOOL_NAME
                        && confirmed_wait_pairs.contains(&calls[0].id))
                } else {
                    true
                }
            }
            ChatMessage::Tool {
                tool_call_id, name, ..
            } => {
                // Filter out tool responses for confirmed wait calls
                !(name == WAIT_TOOL_NAME && confirmed_wait_pairs.contains(tool_call_id))
            }
            _ => true,
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_client::{Function, ToolCall};

    fn create_tool_call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            tool_type: "function".to_string(),
            function: Function {
                name: name.to_string(),
                arguments: "{}".to_string(),
            },
            index: None,
        }
    }

    fn create_assistant_message(tool_calls: Option<Vec<ToolCall>>) -> ChatMessage {
        ChatMessage::Assistant {
            content: None,
            tool_calls,
            reasoning_content: None,
            thinking_blocks: None,
            provider_specific_fields: None,
        }
    }

    fn create_tool_message(id: &str, name: &str) -> ChatMessage {
        ChatMessage::Tool {
            tool_call_id: id.to_string(),
            name: name.to_string(),
            content: "wait".to_string(),
        }
    }

    #[test]
    fn test_filter_wait_tool_calls_removes_wait_pairs() {
        let messages = vec![
            ChatMessage::System {
                content: "System prompt".to_string(),
            },
            create_assistant_message(Some(vec![create_tool_call("wait-1", WAIT_TOOL_NAME)])),
            create_tool_message("wait-1", WAIT_TOOL_NAME),
            create_assistant_message(Some(vec![create_tool_call("other-1", "other_tool")])),
            create_tool_message("other-1", "other_tool"),
        ];

        let filtered = filter_wait_tool_calls(&messages);

        assert_eq!(filtered.len(), 3); // System + other tool pair
        assert!(matches!(&filtered[0], ChatMessage::System { .. }));
        if let ChatMessage::Assistant { tool_calls, .. } = &filtered[1] {
            assert_eq!(tool_calls.as_ref().unwrap()[0].function.name, "other_tool");
        } else {
            panic!("Expected assistant message");
        }
    }

    #[test]
    fn test_filter_wait_tool_calls_handles_interleaved_messages() {
        let messages = vec![
            ChatMessage::User {
                content: "Start".to_string(),
            },
            create_assistant_message(Some(vec![create_tool_call("wait-1", WAIT_TOOL_NAME)])),
            ChatMessage::System {
                content: "Interleaved".to_string(),
            },
            create_tool_message("wait-1", WAIT_TOOL_NAME),
            ChatMessage::User {
                content: "End".to_string(),
            },
        ];

        let filtered = filter_wait_tool_calls(&messages);

        assert_eq!(filtered.len(), 3); // User, System, User
        assert!(matches!(&filtered[0], ChatMessage::User { .. }));
        assert!(matches!(&filtered[1], ChatMessage::System { .. }));
        assert!(matches!(&filtered[2], ChatMessage::User { .. }));
    }

    #[test]
    fn test_filter_wait_tool_calls_keeps_non_wait_messages() {
        let messages = vec![
            create_assistant_message(Some(vec![create_tool_call("tool-1", "some_tool")])),
            create_tool_message("tool-1", "some_tool"),
            ChatMessage::User {
                content: "Hello".to_string(),
            },
            create_assistant_message(Some(vec![create_tool_call("tool-2", "another_tool")])),
        ];

        let filtered = filter_wait_tool_calls(&messages);

        assert_eq!(filtered.len(), 4);
    }

    #[test]
    fn test_filter_wait_tool_calls_keeps_multiple_tool_calls() {
        let messages = vec![
            create_assistant_message(Some(vec![
                create_tool_call("wait-1", WAIT_TOOL_NAME),
                create_tool_call("tool-1", "other_tool"),
            ])),
            create_tool_message("wait-1", WAIT_TOOL_NAME),
            create_tool_message("tool-1", "other_tool"),
        ];

        let filtered = filter_wait_tool_calls(&messages);

        // Should keep all messages because assistant has multiple tool calls
        assert_eq!(filtered.len(), 3);
    }

    #[test]
    fn test_filter_wait_tool_calls_handles_orphaned_wait_call() {
        let messages = vec![
            create_assistant_message(Some(vec![create_tool_call("wait-1", WAIT_TOOL_NAME)])),
            // No corresponding tool response
            ChatMessage::User {
                content: "Next message".to_string(),
            },
        ];

        let filtered = filter_wait_tool_calls(&messages);

        // Should keep the wait call since there's no response
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_wait_tool_calls_handles_orphaned_wait_response() {
        let messages = vec![
            // No corresponding assistant call
            create_tool_message("wait-1", WAIT_TOOL_NAME),
            ChatMessage::User {
                content: "Next message".to_string(),
            },
        ];

        let filtered = filter_wait_tool_calls(&messages);

        // Should keep the orphaned response
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_wait_tool_calls_empty_input() {
        let messages: Vec<ChatMessage> = vec![];
        let filtered = filter_wait_tool_calls(&messages);

        assert_eq!(filtered.len(), 0);
    }

    #[test]
    fn test_filter_wait_tool_calls_multiple_wait_pairs() {
        let messages = vec![
            create_assistant_message(Some(vec![create_tool_call("wait-1", WAIT_TOOL_NAME)])),
            create_assistant_message(Some(vec![create_tool_call("wait-2", WAIT_TOOL_NAME)])),
            create_tool_message("wait-1", WAIT_TOOL_NAME),
            create_tool_message("wait-2", WAIT_TOOL_NAME),
            create_assistant_message(Some(vec![create_tool_call("tool-1", "other_tool")])),
        ];

        let filtered = filter_wait_tool_calls(&messages);

        // Should only keep the non-wait assistant message
        assert_eq!(filtered.len(), 1);
        if let ChatMessage::Assistant { tool_calls, .. } = &filtered[0] {
            assert_eq!(tool_calls.as_ref().unwrap()[0].function.name, "other_tool");
        } else {
            panic!("Expected assistant message");
        }
    }
}

