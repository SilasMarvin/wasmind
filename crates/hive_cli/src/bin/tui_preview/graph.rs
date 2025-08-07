use std::{sync::Arc, time::Duration};

use hive::coordinator::HiveCoordinator;
use hive_actor_loader::LoadedActor;
use hive_actor_utils::STARTING_SCOPE;
use hive_actor_utils_common_messages::assistant::{ChatState, ChatStateUpdated};
use hive_cli::{TuiResult, tui};
use hive_llm_types::types::{ChatMessage, SystemChatMessage};

use crate::utils::create_spawn_agent_message;

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
        .start_hive(&vec![], "Root Agent".to_string())
        .await?;

    tui.run();

    // Spawn some agents
    for i in 0..2000 {
        let (spawn_agent_message, _agent1_scope) = create_spawn_agent_message(
            &format!("Sub Manager {i}"),
            Some(&STARTING_SCOPE.to_string()),
        );
        coordinator.broadcast_common_message(spawn_agent_message, false)?;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Create VERY large sample chat state for testing scrolling performance
    let large_system_content = format!(
        "{}{}{}{}{}{}{}",
        "You are an advanced AI assistant with extensive capabilities in software development, data analysis, creative writing, and problem-solving. ".repeat(5),
        "\n\n[MARKER-1] Your responses should be thorough, well-structured, and demonstrate deep understanding of the topics discussed. ".repeat(3),
        "[MARKER-2] When providing code examples, include detailed explanations and consider edge cases. ".repeat(4),
        "\n\n[MARKER-3] For complex problems, break them down into manageable steps and provide clear reasoning for your approach. ".repeat(3),
        "[MARKER-4] This is a very long system message designed to test the scrolling performance of the new efficient chat history implementation. ".repeat(1),
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
        "\n\nI hope this helps with your distributed systems architecture! Let me know if you need more specific examples or have questions about any of these patterns. ".repeat(1)
    );
    let chat_state = ChatState {
        system: SystemChatMessage {
            content: large_system_content,
        },
        tools: vec![],
        messages: vec![
            ChatMessage::user(user_content),
            ChatMessage::assistant(assistant_content),
        ],
    };

    let chat_update = ChatStateUpdated { chat_state };
    coordinator.broadcast_common_message(chat_update, false)?;

    tokio::time::sleep(Duration::from_secs(150)).await;

    Ok(())
}
