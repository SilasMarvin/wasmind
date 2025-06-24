mod common;

use hive::actors::assistant::Assistant;
use hive::actors::tools::complete::Complete;
use hive::actors::{
    Actor, ActorMessage, AgentMessageType, AgentStatus, InterAgentMessage, Message, ToolCallStatus,
    WaitReason,
};
use hive::scope::Scope;
use tokio::sync::broadcast;
use wiremock::MockServer;

#[tokio::test]
async fn test_complete_tool() {
    // Start mock server
    let mock_server = MockServer::start().await;

    // Create shared broadcast channel and scope
    let (tx, mut rx) = broadcast::channel(100);
    let scope = Scope::new();

    // Create config with mock server URL
    let config = common::create_test_config_with_mock_endpoint(mock_server.uri());

    // Set up mock using create_mock_sequence
    common::create_mock_sequence(&mock_server, scope, "Complete the task")
        .responds_with_complete(
            "chatcmpl-complete",
            "complete_call",
            "Task completed successfully",
            true,
        )
        .build()
        .await;

    // Create assistant with complete tool
    let assistant = Assistant::new(
        config.hive.main_manager_model.clone(),
        tx.clone(),
        scope,
        Scope::new(), // Parent scope is irrelevant for this test
        vec![Complete::ACTOR_ID],
        None,
        vec![],
    );

    // Create complete tool
    let complete_tool = Complete::new(config.clone(), tx.clone(), scope);

    // Start actors
    assistant.run();
    complete_tool.run();

    // Wait for setup and idle
    let mut assistant_ready = false;
    let mut complete_ready = false;
    let mut tools_available = false;

    while !assistant_ready || !complete_ready || !tools_available {
        if let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(1), rx.recv()).await
        {
            let msg = msg.unwrap();
            match &msg.message {
                Message::ActorReady { actor_id } => match actor_id.as_str() {
                    "assistant" => assistant_ready = true,
                    "complete" => complete_ready = true,
                    _ => {}
                },
                Message::ToolsAvailable(tools) => {
                    assert_eq!(tools.len(), 1);
                    assert_eq!(tools[0].name, "complete");
                    tools_available = true;
                }
                _ => {}
            }
        }
    }

    // Wait for idle and consume it
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    let _ = rx.recv().await;

    // Send user input
    tx.send(ActorMessage {
        scope,
        message: Message::UserContext(hive::actors::UserContext::UserTUIInput(
            "Complete the task".to_string(),
        )),
    })
    .unwrap();

    // Track complete tool causality
    let mut seen_user_input = false;
    let mut seen_processing = false;
    let mut seen_assistant_response = false;
    let mut seen_complete_tool_call = false;
    let mut seen_awaiting_tools = false;
    let mut seen_complete_finished = false;
    let mut seen_agent_done = false;

    while let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await {
        let msg = msg.unwrap();
        match &msg.message {
            Message::UserContext(hive::actors::UserContext::UserTUIInput(text)) => {
                assert_eq!(text, "Complete the task");
                seen_user_input = true;
            }
            Message::Agent(agent_msg) if agent_msg.agent_id == scope => {
                if let AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                    status,
                }) = &agent_msg.message
                {
                    match status {
                        AgentStatus::Processing { .. } => {
                            if !seen_processing {
                                assert!(seen_user_input, "Processing must come after UserContext");
                                seen_processing = true;
                            }
                        }
                        AgentStatus::Wait {
                            reason: WaitReason::WaitingForTools { tool_calls },
                        } => {
                            assert!(
                                seen_complete_tool_call,
                                "AwaitingTools must come after tool call"
                            );
                            assert_eq!(tool_calls.len(), 1);
                            assert!(tool_calls.get("complete_call").is_some());
                            seen_awaiting_tools = true;
                        }
                        AgentStatus::Done(result) => {
                            assert!(
                                seen_complete_tool_call,
                                "Done must come after complete tool call"
                            );
                            if let Ok(task_result) = result {
                                assert!(task_result.success, "Task should be successful");
                                assert_eq!(task_result.summary, "Task completed successfully");
                            } else {
                                panic!("Task should not fail");
                            }
                            seen_agent_done = true;
                        }
                        _ => {}
                    }
                }
            }
            Message::AssistantResponse {
                content: genai::chat::MessageContent::ToolCalls(calls),
                ..
            } => {
                assert!(
                    seen_processing,
                    "AssistantResponse must come after Processing"
                );
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].call_id, "complete_call");
                seen_assistant_response = true;
            }
            Message::AssistantToolCall(tc) => {
                assert!(
                    seen_assistant_response,
                    "AssistantToolCall must come after AssistantResponse"
                );
                assert_eq!(tc.call_id, "complete_call");
                assert_eq!(tc.fn_name, "complete");
                seen_complete_tool_call = true;
            }
            Message::ToolCallUpdate(update) if update.call_id == "complete_call" => {
                match &update.status {
                    ToolCallStatus::Finished(Ok(content)) => {
                        assert!(
                            seen_agent_done,
                            "Complete finished must come after agent Done"
                        );
                        assert!(content.contains("Task completed successfully"));
                        seen_complete_finished = true;
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
    assert!(seen_complete_tool_call, "Missing complete tool call");
    assert!(seen_awaiting_tools, "Missing AwaitingTools");
    assert!(seen_complete_finished, "Missing complete finished");
    assert!(seen_agent_done, "Missing agent Done state");
}
