mod common;

use hive::actors::assistant::Assistant;
use hive::actors::tools::plan_approval::PlanApproval;
use hive::actors::{
    Actor, ActorMessage, AgentMessageType, AgentStatus, InterAgentMessage, Message, ToolCallStatus,
    ToolCallType,
};
use hive::scope::Scope;
use tokio::sync::broadcast;
use wiremock::MockServer;

#[tokio::test]
async fn test_plan_approval_approve() {
    // Start mock server
    let mock_server = MockServer::start().await;

    // Create shared broadcast channel and scope
    let (tx, mut rx) = broadcast::channel(100);
    let scope = Scope::new();
    let agent_to_approve = Scope::new();

    // Create config with mock server URL
    let config = common::create_test_config_with_mock_endpoint(mock_server.uri());

    // Set up mock using create_mock_sequence
    common::create_mock_sequence(&mock_server, scope, "Approve the plan")
        .responds_with_approve_plan(
            "chatcmpl-approve",
            "approve_call",
            &agent_to_approve.to_string(),
        )
        .build()
        .await;

    // Create assistant with plan_approval tool
    let assistant = Assistant::new(
        config.hive.main_manager_model.clone(),
        tx.clone(),
        scope,
        vec![PlanApproval::ACTOR_ID],
        None,
        vec![],
    );

    // Create plan approval tool
    let plan_approval = PlanApproval::new(config.clone(), tx.clone(), scope);

    // Start actors
    assistant.run();
    plan_approval.run();

    // Wait for setup and idle
    let mut assistant_ready = false;
    let mut plan_approval_ready = false;
    let mut tools_available = false;

    while !assistant_ready || !plan_approval_ready || !tools_available {
        if let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(1), rx.recv()).await
        {
            let msg = msg.unwrap();
            match &msg.message {
                Message::ActorReady { actor_id } => match actor_id.as_str() {
                    "assistant" => assistant_ready = true,
                    "plan_approval" => plan_approval_ready = true,
                    _ => {}
                },
                Message::ToolsAvailable(tools) => {
                    assert_eq!(tools.len(), 2); // approve_plan and reject_plan
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
            "Approve the plan".to_string(),
        )),
    })
    .unwrap();

    // Track plan approval causality
    let mut seen_user_input = false;
    let mut seen_processing = false;
    let mut seen_assistant_response = false;
    let mut seen_approve_tool_call = false;
    let mut seen_awaiting_tools = false;
    let mut seen_approve_received = false;
    let mut seen_plan_approved_message = false;
    let mut seen_approve_finished = false;

    while let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await {
        let msg = msg.unwrap();
        println!("Received message: {:?}", msg.message);

        match &msg.message {
            Message::UserContext(hive::actors::UserContext::UserTUIInput(text)) => {
                assert_eq!(text, "Approve the plan");
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
                                seen_approve_tool_call,
                                "AwaitingTools must come after tool call"
                            );
                            assert_eq!(pending_tool_calls.len(), 1);
                            assert_eq!(pending_tool_calls[0], "approve_call");
                            seen_awaiting_tools = true;
                        }
                        _ => {}
                    }
                }
            }
            Message::Agent(agent_msg) if agent_msg.agent_id == agent_to_approve => {
                if let AgentMessageType::InterAgentMessage(InterAgentMessage::PlanApproved) =
                    &agent_msg.message
                {
                    assert!(
                        seen_approve_received,
                        "PlanApproved must come after approve received"
                    );
                    seen_plan_approved_message = true;
                }
            }
            Message::AssistantResponse(genai::chat::MessageContent::ToolCalls(calls)) => {
                assert!(
                    seen_processing,
                    "AssistantResponse must come after Processing"
                );
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].call_id, "approve_call");
                seen_assistant_response = true;
            }
            Message::AssistantToolCall(tc) => {
                assert!(
                    seen_assistant_response,
                    "AssistantToolCall must come after AssistantResponse"
                );
                assert_eq!(tc.call_id, "approve_call");
                assert_eq!(tc.fn_name, "approve_plan");
                seen_approve_tool_call = true;
            }
            Message::ToolCallUpdate(update) if update.call_id == "approve_call" => {
                match &update.status {
                    ToolCallStatus::Received {
                        r#type: ToolCallType::MCP,
                        ..
                    } => {
                        assert!(
                            seen_approve_tool_call,
                            "Approve received must come after tool call"
                        );
                        seen_approve_received = true;
                    }
                    ToolCallStatus::Finished(Ok(content)) => {
                        assert!(
                            seen_plan_approved_message,
                            "Approve finished must come after PlanApproved"
                        );
                        assert!(content.contains("approved"));
                        seen_approve_finished = true;
                        println!("✅ SUCCESS: Plan approval workflow finished!");
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
    assert!(seen_approve_tool_call, "Missing approve tool call");
    assert!(seen_awaiting_tools, "Missing AwaitingTools");
    assert!(seen_approve_received, "Missing approve received");
    assert!(seen_plan_approved_message, "Missing PlanApproved message");
    assert!(seen_approve_finished, "Missing approve finished");
}
