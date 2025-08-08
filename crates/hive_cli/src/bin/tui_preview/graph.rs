use hive::coordinator::HiveCoordinator;
use hive::hive_actor_loader::LoadedActor;
use hive_actor_utils::STARTING_SCOPE;
use hive_actor_utils::common_messages::{
    Scope,
    assistant::{ChatState, ChatStateUpdated, Status, StatusUpdate, WaitReason},
    tools::{
        AwaitingSystemDetails, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, UIDisplayInfo,
    },
};
use hive_actor_utils::llm_client_types::{ChatMessage, Function, SystemChatMessage, ToolCall};
use hive_cli::{TuiResult, tui};
use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::utils::create_spawn_agent_message;

pub fn spawn_agent(scope: &Scope, coordinator: &mut HiveCoordinator) -> TuiResult<Scope> {
    let (spawn_agent_message, agent_scope) = create_spawn_agent_message(
        &format!("Sub Manager {}", rand::random::<u64>()),
        Some(scope),
    );
    coordinator.broadcast_common_message(spawn_agent_message, false)?;
    let status = match rand::random_range(0..8) {
        0 => Status::Processing {
            request_id: "Filler".to_string(),
        },
        1 => Status::Done {
            result: Err("Filler".to_string()),
        },
        2 => Status::Wait {
            reason: WaitReason::WaitingForAllActorsReady,
        },
        3 => Status::Wait {
            reason: WaitReason::WaitingForUserInput,
        },
        4 => Status::Wait {
            reason: WaitReason::WaitingForSystemInput {
                required_scope: None,
                interruptible_by_user: true,
            },
        },
        5 => Status::Wait {
            reason: WaitReason::WaitingForAgentCoordination {
                coordinating_tool_call_id: "Filler".to_string(),
                coordinating_tool_name: "Filler".to_string(),
                target_agent_scope: None,
                user_can_interrupt: true,
            },
        },
        6 => Status::Wait {
            reason: WaitReason::WaitingForTools {
                tool_calls: HashMap::new(),
            },
        },
        7 => Status::Wait {
            reason: WaitReason::WaitingForLiteLLM,
        },
        _ => unreachable!(),
    };
    coordinator.broadcast_common_message_in_scope(StatusUpdate { status }, &agent_scope, false)?;

    if rand::random_bool(0.25) {
        spawn_agent(&agent_scope, coordinator)
    } else {
        Ok(agent_scope)
    }
}

fn create_sample_tool_calls() -> Vec<ToolCall> {
    vec![
        ToolCall {
            id: "tool_call_1_received".to_string(),
            tool_type: "function".to_string(),
            function: Function {
                name: "read_file".to_string(),
                arguments: r#"{"path": "/tmp/test.txt"}"#.to_string(),
            },
            index: Some(0),
        },
        ToolCall {
            id: "tool_call_2_awaiting".to_string(),
            tool_type: "function".to_string(),
            function: Function {
                name: "execute_bash".to_string(),
                arguments: r#"{"command": "ls -la"}"#.to_string(),
            },
            index: Some(1),
        },
        ToolCall {
            id: "tool_call_3_done".to_string(),
            tool_type: "function".to_string(),
            function: Function {
                name: "search_files".to_string(),
                arguments: r#"{"pattern": "TODO", "path": "./src"}"#.to_string(),
            },
            index: Some(2),
        },
    ]
}

fn create_tool_status_updates() -> Vec<ToolCallStatusUpdate> {
    vec![
        // First tool call - Received state
        ToolCallStatusUpdate {
            id: "tool_call_1_received".to_string(),
            status: ToolCallStatus::Received {
                display_info: UIDisplayInfo {
                    collapsed: "Reading file /tmp/test.txt...".to_string(),
                    expanded: Some("Attempting to read the contents of /tmp/test.txt to analyze the data.".to_string()),
                },
            },
        },
        // Second tool call - AwaitingSystem state
        ToolCallStatusUpdate {
            id: "tool_call_2_awaiting".to_string(),
            status: ToolCallStatus::AwaitingSystem {
                details: AwaitingSystemDetails {
                    required_scope: Some("bash_executor".to_string()),
                    ui_display_info: UIDisplayInfo {
                        collapsed: "Awaiting system approval for bash command...".to_string(),
                        expanded: Some("Waiting for system approval to execute: ls -la\nThis command will list all files in the current directory.".to_string()),
                    },
                },
            },
        },
        // Third tool call - Done state (success)
        ToolCallStatusUpdate {
            id: "tool_call_3_done".to_string(),
            status: ToolCallStatus::Done {
                result: Ok(ToolCallResult {
                    content: "Found 42 TODO items in 8 files:\n- src/main.rs: 5 items\n- src/utils.rs: 3 items\n- src/handlers.rs: 12 items\n...".to_string(),
                    ui_display_info: UIDisplayInfo {
                        collapsed: "Search completed: 42 TODOs found".to_string(),
                        expanded: Some("Search Results:\n\nFound 42 TODO items across 8 files in ./src:\n\n1. src/main.rs (5 items):\n   - Line 23: TODO: Implement error handling\n   - Line 45: TODO: Add logging\n   - Line 67: TODO: Optimize performance\n   - Line 89: TODO: Write tests\n   - Line 101: TODO: Add documentation\n\n2. src/utils.rs (3 items):\n   - Line 12: TODO: Refactor this function\n   - Line 34: TODO: Add input validation\n   - Line 56: TODO: Handle edge cases\n\n[... more results ...]".to_string()),
                    },
                }),
            },
        },
    ]
}

pub async fn run() -> TuiResult<()> {
    let tui_config = hive_cli::config::TuiConfig::default().parse()?;

    let context = Arc::new(hive::context::HiveContext::new::<LoadedActor>(vec![]));
    let mut coordinator: HiveCoordinator = HiveCoordinator::new(context.clone());

    let tui = tui::Tui::new(
        tui_config,
        coordinator.get_sender(),
        Some("Filler user prompt...".to_string()),
        context.clone(),
    );

    coordinator
        .start_hive(&[], "Root Agent".to_string())
        .await?;

    tui.run();

    // Spawn some agents
    for _ in 0..100 {
        spawn_agent(&STARTING_SCOPE.to_string(), &mut coordinator)?;
    }

    // Create VERY large sample chat state for testing scrolling performance
    let large_system_content = format!(
        "{}{}{}{}{}{}{}",
        "You are an advanced AI assistant with extensive capabilities in software development, data analysis, creative writing, and problem-solving. ".repeat(50),
        "\n\n[MARKER-1] Your responses should be thorough, well-structured, and demonstrate deep understanding of the topics discussed. ".repeat(3),
        "[MARKER-2] When providing code examples, include detailed explanations and consider edge cases. ".repeat(4),
        "\n\n[MARKER-3] For complex problems, break them down into manageable steps and provide clear reasoning for your approach. ".repeat(3),
        "[MARKER-4] This is a very long system message designed to test the scrolling performance of the new efficient chat history implementation. ",
        "\n\n[MARKER-5] Additional context about your capabilities and how you should respond to queries goes here. ".repeat(4),
        "[MARKER-6] Remember to always be helpful, harmless, and honest in your responses. [END-MARKER]".repeat(6)
    );
    let user_content = format!(
        "{}{}{}{}{}{}",
        "I'm working on a complex software architecture project that involves microservices, event-driven design, and distributed systems. ".repeat(4),
        "\n\nCould you help me understand the best practices for implementing resilient communication patterns between services? ".repeat(5),
        "I'm particularly interested in how to handle failures, implement circuit breakers, and ensure data consistency across service boundaries. ".repeat(8),
        "\n\nThis message is intentionally long to test scrolling behavior with realistic user input. ".repeat(5),
        "I also need help with understanding how to implement proper logging, monitoring, and observability in a distributed system. ".repeat(7),
        "\n\nCan you provide detailed examples with code snippets in multiple languages? ".repeat(9)
    );
    let assistant_content = format!(
        "{}{}{}{}{}{}{}{}{}",
        "I'd be happy to help you with implementing resilient communication patterns in your microservices architecture! Let me break this down into several key areas:\n\n".repeat(2),
        "## 1. Circuit Breaker Pattern\n\nThe circuit breaker pattern helps prevent cascading failures by monitoring service calls and temporarily stopping requests to failing services. ".repeat(8),
        "\n\n## 2. Retry Strategies\n\nImplement exponential backoff with jitter to avoid thundering herd problems. Consider different retry policies for different types of failures. ".repeat(6),
        "\n\n## 3. Data Consistency\n\nFor distributed systems, consider using the Saga pattern for managing transactions across multiple services. Event sourcing can also help maintain consistency. ".repeat(7),
        "\n\n## 4. Example Implementation\n\n```rust\npub struct CircuitBreaker {\n    state: Arc<Mutex<State>>,\n    failure_threshold: u32,\n    success_threshold: u32,\n    timeout: Duration,\n}\n\nimpl CircuitBreaker {\n    // Implementation details here...\n}\n```\n\n".repeat(5),
        "\n\n## 5. Monitoring and Observability\n\nUse distributed tracing with tools like Jaeger or Zipkin. Implement structured logging with correlation IDs. ".repeat(8),
        "\n\nThis is a comprehensive response that demonstrates how assistant messages with technical content, code examples, and detailed explanations would appear in the chat interface during scrolling. ".repeat(2),
        "\n\n## 6. Best Practices Summary\n\n* Always implement timeouts\n* Use circuit breakers for external calls\n* Implement proper retry logic with backoff\n* Use distributed tracing\n* Log all important events\n".repeat(7),
        "\n\nI hope this helps with your distributed systems architecture! Let me know if you need more specific examples or have questions about any of these patterns. "
    );
    // Create sample tool calls for the second assistant message
    let tool_calls = create_sample_tool_calls();

    // Create an assistant message with tool calls
    let assistant_with_tools_message = ChatMessage::assistant_with_tools(tool_calls);

    let chat_state = ChatState {
        system: SystemChatMessage {
            content: large_system_content,
        },
        tools: vec![],
        messages: vec![
            ChatMessage::user(user_content),
            ChatMessage::assistant(assistant_content),
            assistant_with_tools_message,
        ],
    };

    let chat_update = ChatStateUpdated { chat_state };
    coordinator.broadcast_common_message(chat_update, false)?;

    // Broadcast tool call status updates to show different states
    let tool_status_updates = create_tool_status_updates();
    for status_update in tool_status_updates {
        coordinator.broadcast_common_message(status_update, false)?;
    }

    tokio::time::sleep(Duration::from_secs(150)).await;

    Ok(())
}
