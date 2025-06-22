mod common;

use hive::actors::assistant::Assistant;
use hive::actors::tools::send_message::{
    SEND_MESSAGE_TOOL_NAME, SendMessage, format_send_message_success,
};
use hive::actors::{
    Actor, ActorMessage, AgentMessageType, AgentStatus, InterAgentMessage, Message, ToolCallStatus,
    WaitReason,
};
use hive::scope::Scope;
use tokio::sync::broadcast;
use wiremock::MockServer;

#[tokio::test]
async fn test_send_message_tool() {
    // Start mock server
    let mock_server = MockServer::start().await;

    // Create shared broadcast channel and scopes
    let (tx, mut rx) = broadcast::channel(1000);
    let manager_scope = Scope::new();
    let child_scope = Scope::new();

    // Create config with mock server URL
    let config = common::create_test_config_with_mock_endpoint(mock_server.uri());

    // Set up mock LLM response for send_information tool call
    common::create_mock_sequence(
        &mock_server,
        manager_scope.clone(),
        "Send information to the child agent",
    )
    .responds_with_tool_call(
        "chatcmpl-send-info",
        "send_message_call",
        SEND_MESSAGE_TOOL_NAME,
        serde_json::json!({
            "agent_id": child_scope.to_string(),
            "message": "Focus on performance optimization and error handling",
            "wait": false
        }),
    )
    .build()
    .await;

    // Create manager assistant with send_information tool
    let assistant = Assistant::new(
        config.hive.main_manager_model.clone(),
        tx.clone(),
        manager_scope.clone(),
        vec![SendMessage::ACTOR_ID],
        None,
        vec![],
    );

    // Create send_information tool
    let send_info = SendMessage::new(config.clone(), tx.clone(), manager_scope.clone());

    // Start actors
    assistant.run();
    send_info.run();

    // Wait for setup
    let mut ready_count = 0;
    while ready_count < 3 {
        // 2 actors + 1 tools available
        if let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await
        {
            let msg = msg.unwrap();
            match &msg.message {
                Message::ActorReady { .. } | Message::ToolsAvailable(_) => ready_count += 1,
                _ => {}
            }
        }
    }

    // Send user input
    tx.send(ActorMessage {
        scope: manager_scope.clone(),
        message: Message::UserContext(hive::actors::UserContext::UserTUIInput(
            "Send information to the child agent".to_string(),
        )),
    })
    .unwrap();

    // Track expected messages
    let mut seen_user_input = false;
    let mut seen_processing = false;
    let mut seen_tool_call = false;
    let mut seen_awaiting_tools = false;
    let mut seen_manager_message = false;
    let mut seen_tool_finished = false;

    while let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await {
        let msg = msg.unwrap();

        println!("{msg:?}");

        match &msg.message {
            Message::UserContext(hive::actors::UserContext::UserTUIInput(text)) => {
                assert_eq!(text, "Send information to the child agent");
                seen_user_input = true;
            }
            Message::Agent(agent_msg) => match &agent_msg.message {
                AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                    status,
                }) if agent_msg.agent_id == manager_scope => match status {
                    AgentStatus::Processing => {
                        assert!(seen_user_input, "Processing must come after UserContext");
                        seen_processing = true;
                    }
                    AgentStatus::AwaitingTools { pending_tool_calls } => {
                        assert!(seen_processing, "Processing must come before AwaitingTools");
                        assert!(seen_tool_call, "AwaitingTools must come after tool call");
                        assert_eq!(pending_tool_calls.len(), 1);
                        assert_eq!(pending_tool_calls[0], "send_message_call");
                        seen_awaiting_tools = true;
                    }
                    _ => {}
                },
                AgentMessageType::InterAgentMessage(InterAgentMessage::ManagerMessage {
                    message,
                }) if agent_msg.agent_id == child_scope => {
                    assert!(seen_tool_call, "ManagerMessage must come after tool call");
                    assert_eq!(
                        message,
                        "Focus on performance optimization and error handling"
                    );
                    seen_manager_message = true;
                }
                _ => {}
            },
            Message::AssistantToolCall(tool_call) => {
                assert!(seen_processing, "Tool call must come after Processing");
                assert_eq!(tool_call.fn_name, SEND_MESSAGE_TOOL_NAME);
                assert_eq!(tool_call.call_id, "send_message_call");
                seen_tool_call = true;
            }
            Message::ToolCallUpdate(update) => {
                if let ToolCallStatus::Finished(Ok(result)) = &update.status {
                    assert!(
                        seen_awaiting_tools,
                        "Tool finish must come after AwaitingTools"
                    );
                    assert_eq!(update.call_id, "send_message_call");
                    assert_eq!(
                        result,
                        &format_send_message_success(&child_scope.to_string(), false)
                    );
                    seen_tool_finished = true;
                    break; // Test complete
                }
            }
            _ => {}
        }
    }

    // Verify all steps occurred
    assert!(seen_user_input, "Should receive user input");
    assert!(seen_processing, "Should see manager processing");
    assert!(seen_tool_call, "Should see send_information tool call");
    assert!(seen_awaiting_tools, "Should see manager awaiting tools");
    assert!(
        seen_manager_message,
        "Should see ManagerMessage sent to child"
    );
    assert!(seen_tool_finished, "Should see tool call finished");
}

#[tokio::test]
#[cfg_attr(not(feature = "test-utils"), ignore)]
async fn test_send_message_tool_wait() {
    // Start mock server
    let mock_server = MockServer::start().await;

    // Create shared broadcast channel and scopes
    let (tx, mut rx) = broadcast::channel(1000);
    let manager_scope = Scope::new();
    let child_scope = Scope::new();

    // Create config with mock server URL
    let config = common::create_test_config_with_mock_endpoint(mock_server.uri());

    // Set up mock LLM response for send_message tool call with wait=true
    common::create_mock_sequence(
        &mock_server,
        manager_scope.clone(),
        "Send information to the child agent and wait",
    )
    .responds_with_tool_call(
        "chatcmpl-send-info",
        "send_message_call",
        SEND_MESSAGE_TOOL_NAME,
        serde_json::json!({
            "agent_id": child_scope.to_string(),
            "message": "Please analyze the requirements and respond",
            "wait": true
        }),
    )
    .build()
    .await;

    // Create manager assistant with send_message tool
    let assistant = Assistant::new(
        config.hive.main_manager_model.clone(),
        tx.clone(),
        manager_scope.clone(),
        vec![SendMessage::ACTOR_ID],
        None,
        vec![],
    );

    // Create send_message tool
    let send_message = SendMessage::new(config.clone(), tx.clone(), manager_scope.clone());

    // Start actors
    assistant.run();
    send_message.run();

    // Wait for setup
    let mut ready_count = 0;
    while ready_count < 3 {
        // 2 actors + 1 tools available
        if let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await
        {
            let msg = msg.unwrap();
            match &msg.message {
                Message::ActorReady { .. } | Message::ToolsAvailable(_) => ready_count += 1,
                _ => {}
            }
        }
    }

    // Send user input
    tx.send(ActorMessage {
        scope: manager_scope.clone(),
        message: Message::UserContext(hive::actors::UserContext::UserTUIInput(
            "Send information to the child agent and wait".to_string(),
        )),
    })
    .unwrap();

    // Track expected messages
    let mut seen_user_input = false;
    let mut seen_processing = false;
    let mut seen_tool_call = false;
    let mut seen_awaiting_tools = false;
    let mut seen_manager_message = false;
    let mut seen_manager_wait = false;
    let mut seen_tool_finished = false;

    while let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await {
        let msg = msg.unwrap();

        println!("{msg:?}");

        match &msg.message {
            Message::UserContext(hive::actors::UserContext::UserTUIInput(text)) => {
                assert_eq!(text, "Send information to the child agent and wait");
                seen_user_input = true;
            }
            Message::Agent(agent_msg) => match &agent_msg.message {
                AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                    status,
                }) if agent_msg.agent_id == manager_scope => match status {
                    AgentStatus::Processing => {
                        assert!(seen_user_input, "Processing must come after UserContext");
                        seen_processing = true;
                    }
                    AgentStatus::AwaitingTools { pending_tool_calls } => {
                        assert!(seen_processing, "Processing must come before AwaitingTools");
                        assert!(seen_tool_call, "AwaitingTools must come after tool call");
                        assert_eq!(pending_tool_calls.len(), 1);
                        assert_eq!(pending_tool_calls[0], "send_message_call");
                        seen_awaiting_tools = true;
                    }
                    AgentStatus::Wait {
                        tool_call_id,
                        reason,
                    } => {
                        assert!(seen_awaiting_tools, "Wait must come after AwaitingTools");
                        assert_eq!(tool_call_id, "send_message_call");
                        assert!(
                            matches!(reason, WaitReason::WaitingForAgentResponse { agent_id } if *agent_id == child_scope)
                        );
                        seen_manager_wait = true;
                    }
                    _ => {}
                },
                AgentMessageType::InterAgentMessage(InterAgentMessage::ManagerMessage {
                    message,
                }) if agent_msg.agent_id == child_scope => {
                    assert!(seen_tool_call, "ManagerMessage must come after tool call");
                    assert_eq!(message, "Please analyze the requirements and respond");
                    seen_manager_message = true;
                }
                _ => {}
            },
            Message::AssistantToolCall(tool_call) => {
                assert!(seen_processing, "Tool call must come after Processing");
                assert_eq!(tool_call.fn_name, SEND_MESSAGE_TOOL_NAME);
                assert_eq!(tool_call.call_id, "send_message_call");
                seen_tool_call = true;
            }
            Message::ToolCallUpdate(update) => {
                if let ToolCallStatus::Finished(Ok(result)) = &update.status {
                    assert!(
                        seen_awaiting_tools,
                        "Tool finish must come after AwaitingTools"
                    );
                    assert_eq!(update.call_id, "send_message_call");
                    assert_eq!(
                        result,
                        &format_send_message_success(&child_scope.to_string(), true)
                    );
                    seen_tool_finished = true;
                    break;
                }
            }
            _ => {}
        }
    }

    // Verify all steps occurred
    assert!(seen_user_input, "Should receive user input");
    assert!(seen_processing, "Should see manager processing");
    assert!(seen_tool_call, "Should see send_message tool call");
    assert!(seen_awaiting_tools, "Should see manager awaiting tools");
    assert!(
        seen_manager_message,
        "Should see ManagerMessage sent to child"
    );
    assert!(seen_manager_wait, "Should see manager in Wait state");
    assert!(seen_tool_finished, "Should see tool finished");
}
