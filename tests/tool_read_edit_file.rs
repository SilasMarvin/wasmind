mod common;

use hive::actors::assistant::Assistant;
use hive::actors::tools::edit_file::{EditFile, TOOL_NAME as EDIT_FILE_TOOL};
use hive::actors::tools::file_reader::{FileReader, FileReaderActor, TOOL_NAME as READ_FILE_TOOL};
use hive::actors::{
    Actor, ActorMessage, AgentMessageType, AgentStatus, InterAgentMessage, Message, ToolCallStatus,
    ToolCallType, WaitReason,
};
use hive::scope::Scope;
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Mutex;
use tokio::sync::broadcast;
use wiremock::MockServer;

#[tokio::test]
async fn test_read_edit_file() {
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
        Scope::new(), // Parent scope is not used for this test
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
        match &msg.message {
            Message::UserContext(hive::actors::UserContext::UserTUIInput(text)) => {
                assert_eq!(text, "Please read and edit the test file");
                seen_user_input = true;
            }
            Message::Agent(agent_msg) if agent_msg.agent_id == scope => {
                if let AgentMessageType::InterAgentMessage(
                    InterAgentMessage::StatusUpdateRequest { status },
                ) = &agent_msg.message
                {
                    match status {
                        AgentStatus::Processing { .. } => {
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
                                break;
                            }
                        }
                        AgentStatus::Wait {
                            reason: WaitReason::WaitingForTools { tool_calls },
                        } => {
                            if !seen_awaiting_tools_1 {
                                assert!(
                                    seen_read_tool_call,
                                    "AwaitingTools 1 must come after read tool call"
                                );
                                assert_eq!(tool_calls.len(), 1);
                                assert!(tool_calls.get("read_call").is_some());
                                seen_awaiting_tools_1 = true;
                            } else {
                                assert!(
                                    seen_edit_tool_call,
                                    "AwaitingTools 2 must come after edit tool call"
                                );
                                assert_eq!(tool_calls.len(), 1);
                                assert!(tool_calls.get("edit_call").is_some());
                                seen_awaiting_tools_2 = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Message::AssistantResponse {
                content: genai::chat::MessageContent::ToolCalls(calls),
                ..
            } => {
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
