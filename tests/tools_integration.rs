mod common;

use hive::actors::assistant::Assistant;
use hive::actors::tools::complete::Complete;
use hive::actors::tools::edit_file::{EditFile, TOOL_NAME as EDIT_FILE_TOOL};
use hive::actors::tools::file_reader::{FileReader, FileReaderActor, TOOL_NAME as READ_FILE_TOOL};
use hive::actors::tools::plan_approval::PlanApproval;
use hive::actors::tools::send_message::{
    SEND_MESSAGE_TOOL_NAME, SendMessage, format_send_message_success,
};
use hive::actors::tools::send_manager_message::{
    SEND_MANAGER_MESSAGE_TOOL_NAME, SendManagerMessage, format_send_manager_message_success,
};
use hive::actors::tools::spawn_agent::SpawnAgent;
use hive::actors::{
    Actor, ActorMessage, AgentMessageType, AgentStatus, AgentType, InterAgentMessage, Message,
    ToolCallStatus, ToolCallType, WaitReason,
};
use hive::scope::Scope;
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Mutex;
use tokio::sync::broadcast;
use wiremock::MockServer;

#[tokio::test]
async fn test_edit_file_insert_at_start() {
    // Start mock server
    let mock_server = MockServer::start().await;

    // Create temporary test file
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let test_file_path = temp_dir.path().join("test_edit.txt");
    let initial_content = "line 1\nline 2\nline 3";
    fs::write(&test_file_path, initial_content).expect("Failed to write test file");

    // Create shared broadcast channel and scope
    let (tx, mut rx) = broadcast::channel(100);
    let scope = Scope::new();

    // Create config with mock server URL
    let config = common::create_test_config_with_mock_endpoint(mock_server.uri());

    // Set up sequential mock conversation using the fluent API
    let tool_result_message = format!("Read file: {}", test_file_path.to_str().unwrap());
    common::create_mock_sequence(&mock_server, scope, "Please read and edit the test file")
        .responds_with_read_file(
            "chatcmpl-read",
            "read_call",
            test_file_path.to_str().unwrap(),
        )
        .then_expects_tool_result("read_call", &tool_result_message)
        .responds_with_edit_file(
            "chatcmpl-edit",
            "edit_call",
            test_file_path.to_str().unwrap(),
            "insert_at_start",
            "// Header comment\n",
        )
        .build()
        .await;

    // Create assistant with both file_reader and edit_file
    let assistant = Assistant::new(
        config.hive.main_manager_model.clone(),
        tx.clone(),
        scope,
        vec![FileReaderActor::ACTOR_ID, EditFile::ACTOR_ID],
        None,
        vec![],
    );

    // Create tools
    let file_reader = Arc::new(Mutex::new(FileReader::default()));
    let file_reader_actor =
        FileReaderActor::new(config.clone(), tx.clone(), file_reader.clone(), scope);
    let edit_file_actor = EditFile::new(config.clone(), tx.clone(), file_reader.clone(), scope);

    // Start all actors
    assistant.run();
    file_reader_actor.run();
    edit_file_actor.run();

    // Wait for initial setup
    let mut assistant_ready = false;
    let mut file_reader_ready = false;
    let mut edit_file_ready = false;
    let mut tools_available_count = 0;

    while !assistant_ready || !file_reader_ready || !edit_file_ready || tools_available_count < 2 {
        if let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(1), rx.recv()).await
        {
            let msg = msg.unwrap();
            match &msg.message {
                Message::ActorReady { actor_id } => match actor_id.as_str() {
                    "assistant" => assistant_ready = true,
                    "file_reader" => file_reader_ready = true,
                    "edit_file" => edit_file_ready = true,
                    _ => {}
                },
                Message::ToolsAvailable(_) => {
                    tools_available_count += 1;
                }
                _ => {}
            }
        } else {
            panic!("Timeout waiting for initial setup messages");
        }
    }

    // Wait for idle state
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Consume the idle message
    let _idle_msg = tokio::time::timeout(tokio::time::Duration::from_millis(100), rx.recv())
        .await
        .expect("Timeout waiting for idle state")
        .expect("Channel closed");

    // Send user input to trigger the edit workflow
    tx.send(ActorMessage {
        scope,
        message: Message::UserContext(hive::actors::UserContext::UserTUIInput(
            "Please read and edit the test file".to_string(),
        )),
    })
    .unwrap();

    // Track causality for the edit file workflow
    let mut seen_user_input = false;
    let mut seen_processing_1 = false;
    let mut seen_assistant_response_1 = false;
    let mut seen_read_tool_call = false;
    let mut seen_awaiting_tools_1 = false;
    let mut seen_read_received = false;
    let mut seen_file_read = false;
    let mut seen_read_finished = false;
    let mut seen_processing_2 = false;
    let mut seen_assistant_response_2 = false;
    let mut seen_edit_tool_call = false;
    let mut seen_awaiting_tools_2 = false;
    let mut seen_edit_received = false;
    let mut seen_file_edited = false;
    let mut seen_edit_finished = false;
    let mut seen_processing_3 = false;

    while let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await {
        let msg = msg.unwrap();
        println!("Received message: {:?}", msg.message);

        match &msg.message {
            Message::UserContext(hive::actors::UserContext::UserTUIInput(text)) => {
                assert_eq!(text, "Please read and edit the test file");
                seen_user_input = true;
            }
            Message::Agent(agent_msg) if agent_msg.agent_id == scope => {
                if let AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                    status,
                }) = &agent_msg.message
                {
                    match status {
                        AgentStatus::Processing => {
                            if !seen_processing_1 {
                                assert!(
                                    seen_user_input,
                                    "Processing 1 must come after UserContext"
                                );
                                seen_processing_1 = true;
                            } else if !seen_processing_2 {
                                assert!(
                                    seen_read_finished,
                                    "Processing 2 must come after read finished"
                                );
                                seen_processing_2 = true;
                            } else {
                                assert!(
                                    seen_edit_finished,
                                    "Processing 3 must come after edit finished"
                                );
                                seen_processing_3 = true;
                                println!("✅ SUCCESS: Edit file workflow completed successfully!");
                                break;
                            }
                        }
                        AgentStatus::AwaitingTools { pending_tool_calls } => {
                            if !seen_awaiting_tools_1 {
                                assert!(
                                    seen_read_tool_call,
                                    "AwaitingTools 1 must come after read tool call"
                                );
                                assert_eq!(pending_tool_calls.len(), 1);
                                assert_eq!(pending_tool_calls[0], "read_call");
                                seen_awaiting_tools_1 = true;
                            } else {
                                assert!(
                                    seen_edit_tool_call,
                                    "AwaitingTools 2 must come after edit tool call"
                                );
                                assert_eq!(pending_tool_calls.len(), 1);
                                assert_eq!(pending_tool_calls[0], "edit_call");
                                seen_awaiting_tools_2 = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Message::AssistantResponse(genai::chat::MessageContent::ToolCalls(calls)) => {
                assert_eq!(calls.len(), 1);
                if !seen_assistant_response_1 {
                    assert!(
                        seen_processing_1,
                        "AssistantResponse 1 must come after Processing 1"
                    );
                    assert_eq!(calls[0].fn_name, READ_FILE_TOOL);
                    seen_assistant_response_1 = true;
                } else {
                    assert!(
                        seen_processing_2,
                        "AssistantResponse 2 must come after Processing 2"
                    );
                    assert_eq!(calls[0].fn_name, EDIT_FILE_TOOL);
                    seen_assistant_response_2 = true;
                }
            }
            Message::AssistantToolCall(tc) => match tc.fn_name.as_str() {
                READ_FILE_TOOL => {
                    assert!(
                        seen_assistant_response_1,
                        "Read tool call must come after AssistantResponse 1"
                    );
                    assert_eq!(tc.call_id, "read_call");
                    seen_read_tool_call = true;
                }
                EDIT_FILE_TOOL => {
                    assert!(
                        seen_assistant_response_2,
                        "Edit tool call must come after AssistantResponse 2"
                    );
                    assert_eq!(tc.call_id, "edit_call");
                    seen_edit_tool_call = true;
                }
                _ => panic!("Unexpected tool call: {}", tc.fn_name),
            },
            Message::ToolCallUpdate(update) => match (&update.status, update.call_id.as_str()) {
                (
                    ToolCallStatus::Received {
                        r#type: ToolCallType::ReadFile,
                        ..
                    },
                    "read_call",
                ) => {
                    assert!(
                        seen_awaiting_tools_1,
                        "Read received must come after AwaitingTools 1"
                    );
                    seen_read_received = true;
                }
                (ToolCallStatus::Finished(Ok(_)), "read_call") => {
                    assert!(seen_file_read, "Read finished must come after FileRead");
                    seen_read_finished = true;
                }
                (
                    ToolCallStatus::Received {
                        r#type: ToolCallType::EditFile,
                        ..
                    },
                    "edit_call",
                ) => {
                    assert!(
                        seen_awaiting_tools_2,
                        "Edit received must come after AwaitingTools 2"
                    );
                    seen_edit_received = true;
                }
                (ToolCallStatus::Finished(Ok(content)), "edit_call") => {
                    assert!(seen_file_edited, "Edit finished must come after FileEdited");
                    assert!(content.contains("Successfully edited file"));
                    seen_edit_finished = true;
                }
                _ => {}
            },
            Message::FileRead { content, .. } => {
                assert!(seen_read_received, "FileRead must come after read received");
                assert_eq!(content, initial_content);
                seen_file_read = true;
            }
            Message::FileEdited { content, .. } => {
                assert!(
                    seen_edit_received,
                    "FileEdited must come after edit received"
                );
                assert!(content.starts_with("// Header comment\n"));
                assert!(content.contains(initial_content));
                seen_file_edited = true;
            }
            _ => {}
        }
    }

    // Verify we saw all expected messages
    assert!(seen_user_input, "Missing UserContext");
    assert!(seen_processing_1, "Missing first Processing");
    assert!(seen_assistant_response_1, "Missing first AssistantResponse");
    assert!(seen_read_tool_call, "Missing read tool call");
    assert!(seen_awaiting_tools_1, "Missing first AwaitingTools");
    assert!(seen_read_received, "Missing read received");
    assert!(seen_file_read, "Missing FileRead");
    assert!(seen_read_finished, "Missing read finished");
    assert!(seen_processing_2, "Missing second Processing");
    assert!(
        seen_assistant_response_2,
        "Missing second AssistantResponse"
    );
    assert!(seen_edit_tool_call, "Missing edit tool call");
    assert!(seen_awaiting_tools_2, "Missing second AwaitingTools");
    assert!(seen_edit_received, "Missing edit received");
    assert!(seen_file_edited, "Missing FileEdited");
    assert!(seen_edit_finished, "Missing edit finished");
    assert!(seen_processing_3, "Missing final Processing");

    // Verify the file was actually edited correctly
    let final_content = fs::read_to_string(&test_file_path).expect("Failed to read final file");
    assert!(final_content.starts_with("// Header comment\n"));
    assert!(final_content.contains(initial_content));
}

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
        println!("Received message: {:?}", msg.message);

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
                        AgentStatus::Processing => {
                            if !seen_processing {
                                assert!(seen_user_input, "Processing must come after UserContext");
                                seen_processing = true;
                            }
                        }
                        AgentStatus::AwaitingTools { pending_tool_calls } => {
                            assert!(
                                seen_complete_tool_call,
                                "AwaitingTools must come after tool call"
                            );
                            assert_eq!(pending_tool_calls.len(), 1);
                            assert_eq!(pending_tool_calls[0], "complete_call");
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
            Message::AssistantResponse(genai::chat::MessageContent::ToolCalls(calls)) => {
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
                        println!("✅ SUCCESS: Complete tool workflow finished!");
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

#[tokio::test]
async fn test_spawn_agent_basic() {
    // Start mock server
    let mock_server = MockServer::start().await;

    // Create shared broadcast channel and scope
    let (tx, mut rx) = broadcast::channel(100);
    let scope = Scope::new();

    // Create config with mock server URL
    let config = common::create_test_config_with_mock_endpoint(mock_server.uri());

    // Set up mock using create_mock_sequence
    let agents = vec![common::create_agent_spec(
        "Test Worker",
        "Simple test task",
        "Worker",
    )];
    common::create_mock_sequence(&mock_server, scope, "Spawn a test agent")
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
        if let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(1), rx.recv()).await
        {
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
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    let _ = rx.recv().await;

    // Send user input
    tx.send(ActorMessage {
        scope,
        message: Message::UserContext(hive::actors::UserContext::UserTUIInput(
            "Spawn a test agent".to_string(),
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

    while let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await {
        let msg = msg.unwrap();
        println!("Received message: {:?}", msg.message);

        match &msg.message {
            Message::UserContext(hive::actors::UserContext::UserTUIInput(text)) => {
                assert_eq!(text, "Spawn a test agent");
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
                    assert_eq!(role, "Test Worker");
                    assert_eq!(task_description, "Simple test task");
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
                        assert!(content.contains("Test Worker"));
                        seen_spawn_finished = true;
                        println!("✅ SUCCESS: Spawn agent workflow finished!");
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
#[cfg_attr(not(feature = "test-utils"), ignore)]
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
