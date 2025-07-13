use tuirealm::event::{Key, KeyEvent, KeyModifiers};

use crate::{
    actors::tools::{Tool, wait::WaitTool},
    llm_client::{AssistantChatMessage, ChatMessage},
};
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
        if let ChatMessage::Assistant(AssistantChatMessage { tool_calls, .. }) = message {
            if let Some(calls) = tool_calls {
                if calls.len() == 1 && calls[0].function.name == WaitTool::TOOL_NAME {
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
            if name == WaitTool::TOOL_NAME
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
            ChatMessage::Assistant(AssistantChatMessage { tool_calls, .. }) => {
                if let Some(calls) = tool_calls {
                    // Only filter if it's a single wait call that has a confirmed pair
                    !(calls.len() == 1
                        && calls[0].function.name == WaitTool::TOOL_NAME
                        && confirmed_wait_pairs.contains(&calls[0].id))
                } else {
                    true
                }
            }
            ChatMessage::Tool {
                tool_call_id, name, ..
            } => {
                // Filter out tool responses for confirmed wait calls
                !(name == WaitTool::TOOL_NAME && confirmed_wait_pairs.contains(tool_call_id))
            }
            _ => true,
        })
        .cloned()
        .collect()
}

pub fn parse_key_combination(input: &str) -> Option<KeyEvent> {
    let parts: Vec<&str> = input.split('-').collect();
    let mut modifiers = KeyModifiers::empty();

    // Handle the last part as the key, everything before it as modifiers
    let (modifier_parts, key_part) = parts.split_at(parts.len() - 1);

    // Parse modifiers
    for modifier in modifier_parts {
        match modifier.to_lowercase().as_str() {
            "ctrl" => modifiers.insert(KeyModifiers::CONTROL),
            "alt" | "cmd" => modifiers.insert(KeyModifiers::ALT),
            "shift" => modifiers.insert(KeyModifiers::SHIFT),
            _ => return None,
        }
    }

    // Parse the actual key (case insensitive)
    let key_str = key_part[0].to_lowercase();
    let code = match key_str.as_str() {
        // Single character
        s if s.len() == 1 => Key::Char(s.chars().next().unwrap()),
        // Special keys
        "esc" | "escape" => Key::Esc,
        "enter" | "return" => Key::Enter,
        "tab" => Key::Tab,
        "backtab" => Key::BackTab,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "insert" | "ins" => Key::Insert,
        "down" => Key::Down,
        "up" => Key::Up,
        "left" => Key::Left,
        "right" => Key::Right,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "pgup" => Key::PageUp,
        "pagedown" | "pgdn" => Key::PageDown,
        "space" => Key::Char(' '),
        _ => return None,
    };

    Some(KeyEvent::new(code, modifiers))
}

pub fn key_event_to_string(event: &KeyEvent) -> String {
    let mut parts = Vec::new();

    // Add modifiers in consistent order: ctrl-shift-alt
    if event.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl");
    }
    if event.modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("shift");
    }
    if event.modifiers.contains(KeyModifiers::ALT) {
        parts.push("alt");
    }

    // Add the key itself
    let key_str = match event.code {
        Key::Char(' ') => "space".to_string(),
        Key::Char(c) => c.to_lowercase().to_string(),
        Key::Esc => "esc".to_string(),
        Key::Enter => "enter".to_string(),
        Key::Tab => "tab".to_string(),
        Key::BackTab => "backtab".to_string(),
        Key::Backspace => "backspace".to_string(),
        Key::Delete => "delete".to_string(),
        Key::Insert => "insert".to_string(),
        Key::Down => "down".to_string(),
        Key::Up => "up".to_string(),
        Key::Left => "left".to_string(),
        Key::Right => "right".to_string(),
        Key::Home => "home".to_string(),
        Key::End => "end".to_string(),
        Key::PageUp => "pageup".to_string(),
        Key::PageDown => "pagedown".to_string(),
        _ => return String::new(), // Unsupported key
    };

    parts.push(&key_str);
    parts.join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_client::{Function, ToolCall};

    #[test]
    fn test_parse_simple_keys() {
        // Single key
        let event = parse_key_combination("a").unwrap();
        assert_eq!(event.code, Key::Char('a'));
        assert_eq!(event.modifiers, KeyModifiers::NONE);

        // Special key
        let event = parse_key_combination("esc").unwrap();
        assert_eq!(event.code, Key::Esc);
        assert_eq!(event.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn test_parse_with_modifiers() {
        // Ctrl+A
        let event = parse_key_combination("ctrl-a").unwrap();
        assert_eq!(event.code, Key::Char('a'));
        assert_eq!(event.modifiers, KeyModifiers::CONTROL);

        // Ctrl+Shift+Tab
        let event = parse_key_combination("ctrl-shift-tab").unwrap();
        assert_eq!(event.code, Key::Tab);
        assert_eq!(event.modifiers, KeyModifiers::CONTROL | KeyModifiers::SHIFT);

        // Cmd+Enter (cmd maps to alt)
        let event = parse_key_combination("cmd-enter").unwrap();
        assert_eq!(event.code, Key::Enter);
        assert_eq!(event.modifiers, KeyModifiers::ALT);
    }

    #[test]
    fn test_case_insensitive() {
        let event1 = parse_key_combination("CTRL-A").unwrap();
        let event2 = parse_key_combination("ctrl-a").unwrap();
        assert_eq!(event1.code, event2.code);
        assert_eq!(event1.modifiers, event2.modifiers);
    }

    #[test]
    fn test_key_event_to_string() {
        // Simple key
        let event = KeyEvent::new(Key::Char('a'), KeyModifiers::NONE);
        assert_eq!(key_event_to_string(&event), "a");

        // With modifiers
        let event = KeyEvent::new(Key::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(key_event_to_string(&event), "ctrl-a");

        // Multiple modifiers (consistent order)
        let event = KeyEvent::new(Key::Tab, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        assert_eq!(key_event_to_string(&event), "ctrl-shift-tab");

        // Special keys
        let event = KeyEvent::new(Key::Char(' '), KeyModifiers::ALT);
        assert_eq!(key_event_to_string(&event), "alt-space");
    }

    #[test]
    fn test_roundtrip() {
        let test_cases = vec![
            "a",
            "ctrl-a",
            "ctrl-shift-tab",
            "alt-enter",
            "ctrl-shift-alt-space",
            "esc",
            "delete",
        ];

        for input in test_cases {
            let event = parse_key_combination(input).unwrap();
            let output = key_event_to_string(&event);
            let parsed_again = parse_key_combination(&output).unwrap();
            assert_eq!(event.code, parsed_again.code);
            assert_eq!(event.modifiers, parsed_again.modifiers);
        }
    }

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
        ChatMessage::Assistant(AssistantChatMessage {
            content: None,
            tool_calls,
            reasoning_content: None,
            thinking_blocks: None,
            provider_specific_fields: None,
        })
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
            create_assistant_message(Some(vec![create_tool_call("wait-1", WaitTool::TOOL_NAME)])),
            create_tool_message("wait-1", WaitTool::TOOL_NAME),
            create_assistant_message(Some(vec![create_tool_call("other-1", "other_tool")])),
            create_tool_message("other-1", "other_tool"),
        ];

        let filtered = filter_wait_tool_calls(&messages);

        assert_eq!(filtered.len(), 3); // System + other tool pair
        assert!(matches!(&filtered[0], ChatMessage::System { .. }));
        if let ChatMessage::Assistant(AssistantChatMessage { tool_calls, .. }) = &filtered[1] {
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
            create_assistant_message(Some(vec![create_tool_call("wait-1", WaitTool::TOOL_NAME)])),
            ChatMessage::System {
                content: "Interleaved".to_string(),
            },
            create_tool_message("wait-1", WaitTool::TOOL_NAME),
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
                create_tool_call("wait-1", WaitTool::TOOL_NAME),
                create_tool_call("tool-1", "other_tool"),
            ])),
            create_tool_message("wait-1", WaitTool::TOOL_NAME),
            create_tool_message("tool-1", "other_tool"),
        ];

        let filtered = filter_wait_tool_calls(&messages);

        // Should keep all messages because assistant has multiple tool calls
        assert_eq!(filtered.len(), 3);
    }

    #[test]
    fn test_filter_wait_tool_calls_handles_orphaned_wait_call() {
        let messages = vec![
            create_assistant_message(Some(vec![create_tool_call("wait-1", WaitTool::TOOL_NAME)])),
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
            create_tool_message("wait-1", WaitTool::TOOL_NAME),
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
            create_assistant_message(Some(vec![create_tool_call("wait-1", WaitTool::TOOL_NAME)])),
            create_assistant_message(Some(vec![create_tool_call("wait-2", WaitTool::TOOL_NAME)])),
            create_tool_message("wait-1", WaitTool::TOOL_NAME),
            create_tool_message("wait-2", WaitTool::TOOL_NAME),
            create_assistant_message(Some(vec![create_tool_call("tool-1", "other_tool")])),
        ];

        let filtered = filter_wait_tool_calls(&messages);

        // Should only keep the non-wait assistant message
        assert_eq!(filtered.len(), 1);
        if let ChatMessage::Assistant(AssistantChatMessage { tool_calls, .. }) = &filtered[0] {
            assert_eq!(tool_calls.as_ref().unwrap()[0].function.name, "other_tool");
        } else {
            panic!("Expected assistant message");
        }
    }
}
