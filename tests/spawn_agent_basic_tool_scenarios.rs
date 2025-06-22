mod common;

use hive::actors::assistant::Assistant;
use hive::actors::tools::spawn_agent::SpawnAgent;
use hive::actors::{
    Actor, ActorMessage, AgentMessageType, AgentStatus, AgentType, InterAgentMessage, Message,
    ToolCallStatus, ToolCallType,
};
use hive::scope::Scope;
use std::time::Duration;
use tokio::sync::broadcast;
use wiremock::MockServer;

#[tokio::test]
async fn test_no_wait_immediate_complete() {
    // Start mock server
    let mock_server = MockServer::start().await;

    // Create shared broadcast channel and scope
    let (tx, mut rx) = broadcast::channel(100);
    let scope = Scope::new();

    // Create config with mock server URL
    let config = common::create_test_config_with_mock_endpoint(mock_server.uri());

    // Set up mock using create_mock_sequence
    let agents = vec![common::create_agent_spec(
        "Quick Worker",
        "Complete immediately",
        "Worker",
    )];
    common::create_mock_sequence(&mock_server, scope, "Spawn a quick worker (no wait)")
        .responds_with_spawn_agents("chatcmpl-spawn", "spawn_call", agents, false)
        .build()
        .await;

    // Create assistant with spawn_agent tool
    let assistant = Assistant::new(
        config.hive.main_manager_model.clone(),
        tx.clone(),
        scope,
        vec![SpawnAgent::ACTOR_ID],
        None,
        vec![],
    );

    // Create spawn agent tool
    let spawn_agent = SpawnAgent::new(config.clone(), tx.clone(), scope);

    // Start actors
    assistant.run();
    spawn_agent.run();

    // Wait for setup and idle
    let mut assistant_ready = false;
    let mut spawn_agent_ready = false;
    let mut tools_available = false;

    while !assistant_ready || !spawn_agent_ready || !tools_available {
        if let Ok(msg) = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
            let msg = msg.unwrap();
            match &msg.message {
                Message::ActorReady { actor_id } => match actor_id.as_str() {
                    "assistant" => assistant_ready = true,
                    "spawn_agent" => spawn_agent_ready = true,
                    _ => {}
                },
                Message::ToolsAvailable(tools) => {
                    assert_eq!(tools.len(), 1); // spawn_agents
                    assert_eq!(tools[0].name, "spawn_agents");
                    tools_available = true;
                }
                _ => {}
            }
        }
    }

    // Wait for idle and consume it
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = rx.recv().await;

    // Send user input
    tx.send(ActorMessage {
        scope,
        message: Message::UserContext(hive::actors::UserContext::UserTUIInput(
            "Spawn a quick worker (no wait)".to_string(),
        )),
    })
    .unwrap();

    // Track spawn agent causality
    let mut seen_user_input = false;
    let mut seen_processing = false;
    let mut seen_assistant_response = false;
    let mut seen_spawn_tool_call = false;
    let mut seen_awaiting_tools = false;
    let mut seen_spawn_received = false;
    let mut seen_agent_spawned = false;
    let mut seen_spawn_finished = false;

    while let Ok(msg) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
        let msg = msg.unwrap();
        match &msg.message {
            Message::UserContext(hive::actors::UserContext::UserTUIInput(text)) => {
                assert_eq!(text, "Spawn a quick worker (no wait)");
                seen_user_input = true;
            }
            Message::Agent(agent_msg) if agent_msg.agent_id == scope => {
                if let AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                    status,
                }) = &agent_msg.message
                {
                    match status {
                        AgentStatus::Processing => {
                            assert!(seen_user_input, "Processing must come after UserContext");
                            seen_processing = true;
                        }
                        AgentStatus::AwaitingTools { pending_tool_calls } => {
                            assert!(
                                seen_spawn_tool_call,
                                "AwaitingTools must come after tool call"
                            );
                            assert_eq!(pending_tool_calls.len(), 1);
                            assert_eq!(pending_tool_calls[0], "spawn_call");
                            seen_awaiting_tools = true;
                        }
                        _ => {}
                    }
                }
            }
            Message::Agent(agent_msg) if agent_msg.agent_id != scope => {
                if let AgentMessageType::AgentSpawned {
                    role,
                    task_description,
                    agent_type,
                    ..
                } = &agent_msg.message
                {
                    assert!(
                        seen_spawn_received,
                        "AgentSpawned must come after spawn received"
                    );
                    assert_eq!(role, "Quick Worker");
                    assert_eq!(task_description, "Complete immediately");
                    assert_eq!(*agent_type, AgentType::Worker);
                    seen_agent_spawned = true;
                }
            }
            Message::AssistantResponse(genai::chat::MessageContent::ToolCalls(calls)) => {
                assert!(
                    seen_processing,
                    "AssistantResponse must come after Processing"
                );
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].call_id, "spawn_call");
                seen_assistant_response = true;
            }
            Message::AssistantToolCall(tc) => {
                assert!(
                    seen_assistant_response,
                    "AssistantToolCall must come after AssistantResponse"
                );
                assert_eq!(tc.call_id, "spawn_call");
                assert_eq!(tc.fn_name, "spawn_agents");
                seen_spawn_tool_call = true;
            }
            Message::ToolCallUpdate(update) if update.call_id == "spawn_call" => {
                match &update.status {
                    ToolCallStatus::Received {
                        r#type: ToolCallType::SpawnAgent,
                        ..
                    } => {
                        assert!(
                            seen_spawn_tool_call,
                            "Spawn received must come after tool call"
                        );
                        seen_spawn_received = true;
                    }
                    ToolCallStatus::Finished(Ok(content)) => {
                        assert!(
                            seen_agent_spawned,
                            "Spawn finished must come after AgentSpawned"
                        );
                        assert!(content.contains("Spawned 1 agent"));
                        assert!(content.contains("Quick Worker"));
                        seen_spawn_finished = true;
                        break;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    // Verify all expected messages
    assert!(seen_user_input, "Missing UserContext");
    assert!(seen_processing, "Missing Processing");
    assert!(seen_assistant_response, "Missing AssistantResponse");
    assert!(seen_spawn_tool_call, "Missing spawn tool call");
    assert!(seen_awaiting_tools, "Missing AwaitingTools");
    assert!(seen_spawn_received, "Missing spawn received");
    assert!(seen_agent_spawned, "Missing AgentSpawned");
    assert!(seen_spawn_finished, "Missing spawn finished");
}

#[tokio::test]
async fn test_wait_immediate_complete() {
    // Start mock server
    let mock_server = MockServer::start().await;

    // Create shared broadcast channel and scope
    let (tx, mut rx) = broadcast::channel(100);
    let scope = Scope::new();

    // Create config with mock server URL
    let config = common::create_test_config_with_mock_endpoint(mock_server.uri());

    // Set up mock using create_mock_sequence
    let agents = vec![common::create_agent_spec(
        "Quick Worker",
        "Complete immediately",
        "Worker",
    )];
    common::create_mock_sequence(&mock_server, scope, "Spawn a quick worker and wait")
        .responds_with_spawn_agents("chatcmpl-spawn", "spawn_call", agents, true)
        .build()
        .await;

    // Create assistant with spawn_agent tool
    let assistant = Assistant::new(
        config.hive.main_manager_model.clone(),
        tx.clone(),
        scope,
        vec![SpawnAgent::ACTOR_ID],
        None,
        vec![],
    );

    // Create spawn agent tool
    let spawn_agent = SpawnAgent::new(config.clone(), tx.clone(), scope);

    // Start actors
    assistant.run();
    spawn_agent.run();

    // Wait for setup and idle
    let mut assistant_ready = false;
    let mut spawn_agent_ready = false;
    let mut tools_available = false;

    while !assistant_ready || !spawn_agent_ready || !tools_available {
        if let Ok(msg) = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
            let msg = msg.unwrap();
            match &msg.message {
                Message::ActorReady { actor_id } => match actor_id.as_str() {
                    "assistant" => assistant_ready = true,
                    "spawn_agent" => spawn_agent_ready = true,
                    _ => {}
                },
                Message::ToolsAvailable(tools) => {
                    assert_eq!(tools.len(), 1); // spawn_agents
                    assert_eq!(tools[0].name, "spawn_agents");
                    tools_available = true;
                }
                _ => {}
            }
        }
    }

    // Wait for idle and consume it
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = rx.recv().await;

    // Send user input
    tx.send(ActorMessage {
        scope,
        message: Message::UserContext(hive::actors::UserContext::UserTUIInput(
            "Spawn a quick worker and wait".to_string(),
        )),
    })
    .unwrap();

    // Track spawn agent causality (including Wait state)
    let mut seen_user_input = false;
    let mut seen_processing = false;
    let mut seen_assistant_response = false;
    let mut seen_spawn_tool_call = false;
    let mut seen_awaiting_tools = false;
    let mut seen_spawn_received = false;
    let mut seen_agent_spawned = false;
    let mut seen_spawn_finished = false;
    let mut seen_wait_state = false;

    while let Ok(msg) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
        let msg = msg.unwrap();
        match &msg.message {
            Message::UserContext(hive::actors::UserContext::UserTUIInput(text)) => {
                assert_eq!(text, "Spawn a quick worker and wait");
                seen_user_input = true;
            }
            Message::Agent(agent_msg) if agent_msg.agent_id == scope => {
                if let AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                    status,
                }) = &agent_msg.message
                {
                    match status {
                        AgentStatus::Processing => {
                            assert!(seen_user_input, "Processing must come after UserContext");
                            seen_processing = true;
                        }
                        AgentStatus::AwaitingTools { pending_tool_calls } => {
                            assert!(
                                seen_spawn_tool_call,
                                "AwaitingTools must come after tool call"
                            );
                            assert_eq!(pending_tool_calls.len(), 1);
                            assert_eq!(pending_tool_calls[0], "spawn_call");
                            seen_awaiting_tools = true;
                        }
                        AgentStatus::Wait { tool_call_id, .. } => {
                            assert!(seen_spawn_received, "Wait must come after spawn received");
                            assert_eq!(tool_call_id, "spawn_call");
                            seen_wait_state = true;
                            // Continue to see the rest of the workflow
                        }
                        _ => {}
                    }
                }
            }
            Message::Agent(agent_msg) if agent_msg.agent_id != scope => {
                if let AgentMessageType::AgentSpawned {
                    role,
                    task_description,
                    agent_type,
                    ..
                } = &agent_msg.message
                {
                    assert!(
                        seen_spawn_received,
                        "AgentSpawned must come after spawn received"
                    );
                    assert_eq!(role, "Quick Worker");
                    assert_eq!(task_description, "Complete immediately");
                    assert_eq!(*agent_type, AgentType::Worker);
                    seen_agent_spawned = true;
                }
            }
            Message::AssistantResponse(genai::chat::MessageContent::ToolCalls(calls)) => {
                assert!(
                    seen_processing,
                    "AssistantResponse must come after Processing"
                );
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].call_id, "spawn_call");
                seen_assistant_response = true;
            }
            Message::AssistantToolCall(tc) => {
                assert!(
                    seen_assistant_response,
                    "AssistantToolCall must come after AssistantResponse"
                );
                assert_eq!(tc.call_id, "spawn_call");
                assert_eq!(tc.fn_name, "spawn_agents");
                seen_spawn_tool_call = true;
            }
            Message::ToolCallUpdate(update) if update.call_id == "spawn_call" => {
                match &update.status {
                    ToolCallStatus::Received {
                        r#type: ToolCallType::SpawnAgent,
                        ..
                    } => {
                        assert!(
                            seen_spawn_tool_call,
                            "Spawn received must come after tool call"
                        );
                        seen_spawn_received = true;
                    }
                    ToolCallStatus::Finished(Ok(content)) => {
                        assert!(
                            seen_agent_spawned,
                            "Spawn finished must come after AgentSpawned"
                        );
                        assert!(content.contains("Spawned 1 agent"));
                        assert!(content.contains("Quick Worker"));
                        seen_spawn_finished = true;
                        // Break here once we've seen everything including wait state
                        if seen_wait_state {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    // Verify all expected messages including Wait state
    assert!(seen_user_input, "Missing UserContext");
    assert!(seen_processing, "Missing Processing");
    assert!(seen_assistant_response, "Missing AssistantResponse");
    assert!(seen_spawn_tool_call, "Missing spawn tool call");
    assert!(seen_awaiting_tools, "Missing AwaitingTools");
    assert!(seen_spawn_received, "Missing spawn received");
    assert!(seen_agent_spawned, "Missing AgentSpawned");
    assert!(seen_spawn_finished, "Missing spawn finished");
    assert!(
        seen_wait_state,
        "Missing Wait state - parent should wait for child completion"
    );
}

#[tokio::test]
async fn test_no_wait_long_running() {
    // Start mock server
    let mock_server = MockServer::start().await;

    // Create shared broadcast channel and scope
    let (tx, mut rx) = broadcast::channel(100);
    let scope = Scope::new();

    // Create config with mock server URL
    let config = common::create_test_config_with_mock_endpoint(mock_server.uri());

    // Set up mock using create_mock_sequence
    let agents = vec![common::create_agent_spec(
        "Long Worker",
        "Process data for 3 seconds",
        "Worker",
    )];
    common::create_mock_sequence(
        &mock_server,
        scope,
        "Spawn a long-running worker without waiting",
    )
    .responds_with_spawn_agents("chatcmpl-spawn", "spawn_call", agents, false)
    .build()
    .await;

    // Create assistant with spawn_agent tool
    let assistant = Assistant::new(
        config.hive.main_manager_model.clone(),
        tx.clone(),
        scope,
        vec![SpawnAgent::ACTOR_ID],
        None,
        vec![],
    );

    // Create spawn agent tool
    let spawn_agent = SpawnAgent::new(config.clone(), tx.clone(), scope);

    // Start actors
    assistant.run();
    spawn_agent.run();

    // Wait for setup and idle
    let mut assistant_ready = false;
    let mut spawn_agent_ready = false;
    let mut tools_available = false;

    while !assistant_ready || !spawn_agent_ready || !tools_available {
        if let Ok(msg) = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
            let msg = msg.unwrap();
            match &msg.message {
                Message::ActorReady { actor_id } => match actor_id.as_str() {
                    "assistant" => assistant_ready = true,
                    "spawn_agent" => spawn_agent_ready = true,
                    _ => {}
                },
                Message::ToolsAvailable(tools) => {
                    assert_eq!(tools.len(), 1);
                    tools_available = true;
                }
                _ => {}
            }
        }
    }

    // Wait for idle and consume it
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = rx.recv().await;

    // Send user input
    tx.send(ActorMessage {
        scope,
        message: Message::UserContext(hive::actors::UserContext::UserTUIInput(
            "Spawn a long-running worker without waiting".to_string(),
        )),
    })
    .unwrap();

    // Track spawn workflow - parent should continue immediately
    let mut seen_spawn_finished = false;
    let mut seen_parent_continues = false;
    let start_time = std::time::Instant::now();

    while let Ok(msg) = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
        let msg = msg.unwrap();

        match &msg.message {
            Message::ToolCallUpdate(update) if update.call_id == "spawn_call" => {
                match &update.status {
                    ToolCallStatus::Finished(Ok(_)) => {
                        seen_spawn_finished = true;
                        let elapsed = start_time.elapsed();
                        // Parent should finish spawn quickly (under 1 second)
                        assert!(
                            elapsed.as_millis() < 1000,
                            "Parent took too long to finish spawn: {}ms",
                            elapsed.as_millis()
                        );
                    }
                    _ => {}
                }
            }
            Message::Agent(agent_msg) if agent_msg.agent_id == scope => {
                if let AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                    status,
                }) = &agent_msg.message
                {
                    match status {
                        AgentStatus::Processing => {
                            if seen_spawn_finished {
                                // Parent continues processing after spawn
                                seen_parent_continues = true;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    assert!(seen_spawn_finished, "Spawn should finish quickly");
    assert!(
        seen_parent_continues || start_time.elapsed().as_millis() < 2000,
        "Parent should continue immediately without waiting"
    );
}

#[tokio::test]
async fn test_wait_long_running() {
    // Start mock server
    let mock_server = MockServer::start().await;

    // Create shared broadcast channel and scope
    let (tx, mut rx) = broadcast::channel(100);
    let scope = Scope::new();

    // Create config with mock server URL
    let config = common::create_test_config_with_mock_endpoint(mock_server.uri());

    // Set up mock using create_mock_sequence
    let agents = vec![common::create_agent_spec(
        "Long Worker",
        "Process data for several seconds",
        "Worker",
    )];
    common::create_mock_sequence(
        &mock_server,
        scope,
        "Spawn a long-running worker and wait for it",
    )
    .responds_with_spawn_agents("chatcmpl-spawn", "spawn_call", agents, true)
    .build()
    .await;

    // Create assistant with spawn_agent tool
    let assistant = Assistant::new(
        config.hive.main_manager_model.clone(),
        tx.clone(),
        scope,
        vec![SpawnAgent::ACTOR_ID],
        None,
        vec![],
    );

    // Create spawn agent tool
    let spawn_agent = SpawnAgent::new(config.clone(), tx.clone(), scope);

    // Start actors
    assistant.run();
    spawn_agent.run();

    // Wait for setup and idle
    let mut assistant_ready = false;
    let mut spawn_agent_ready = false;
    let mut tools_available = false;

    while !assistant_ready || !spawn_agent_ready || !tools_available {
        if let Ok(msg) = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
            let msg = msg.unwrap();
            match &msg.message {
                Message::ActorReady { actor_id } => match actor_id.as_str() {
                    "assistant" => assistant_ready = true,
                    "spawn_agent" => spawn_agent_ready = true,
                    _ => {}
                },
                Message::ToolsAvailable(tools) => {
                    assert_eq!(tools.len(), 1);
                    tools_available = true;
                }
                _ => {}
            }
        }
    }

    // Wait for idle and consume it
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = rx.recv().await;

    // Send user input
    tx.send(ActorMessage {
        scope,
        message: Message::UserContext(hive::actors::UserContext::UserTUIInput(
            "Spawn a long-running worker and wait for it".to_string(),
        )),
    })
    .unwrap();

    // Track wait state - parent should enter and remain in wait state
    let mut seen_wait_state = false;
    let mut seen_spawn_finished = false;
    let mut seen_processing = false;

    while let Ok(msg) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
        let msg = msg.unwrap();

        match &msg.message {
            Message::Agent(agent_msg) if agent_msg.agent_id == scope => {
                if let AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                    status,
                }) = &agent_msg.message
                {
                    match status {
                        AgentStatus::Wait { tool_call_id, .. } => {
                            assert!(!seen_wait_state);
                            assert!(seen_processing);
                            assert!(!seen_spawn_finished);
                            assert_eq!(tool_call_id, "spawn_call");
                            seen_wait_state = true;
                        }
                        AgentStatus::Processing => {
                            assert!(!seen_processing);
                            assert!(!seen_spawn_finished);
                            assert!(!seen_wait_state);
                            seen_processing = true;
                        }
                        _ => {}
                    }
                }
            }
            Message::ToolCallUpdate(update) if update.call_id == "spawn_call" => {
                match &update.status {
                    ToolCallStatus::Finished(Ok(_)) => {
                        seen_spawn_finished = true;
                        break;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    assert!(seen_processing, "Parent should enter Processing state");
    assert!(seen_wait_state, "Parent should enter Wait state");
    assert!(seen_spawn_finished, "Spawn should finish");
}
