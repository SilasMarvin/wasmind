use hive::actors::assistant::Assistant;
use hive::actors::tools::edit_file::{EditFile, TOOL_NAME as EDIT_FILE_TOOL};
use hive::actors::tools::file_reader::{
    FileReader, FileReaderActor, TOOL_NAME as READ_FILE_TOOL,
};
use hive::actors::tools::complete::Complete;
use hive::actors::tools::plan_approval::PlanApproval;
use hive::actors::tools::spawn_agent::SpawnAgent;
use hive::actors::{
    Actor, ActorMessage, AgentMessageType, AgentStatus, AgentType, InterAgentMessage,
    Message, ToolCallStatus, ToolCallType,
};
use hive::config::create_test_config_with_mock_endpoint;
use serde_json::json;
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Mutex;
use tokio::sync::broadcast;
use uuid::Uuid;
use wiremock::{MockServer, Mock, ResponseTemplate};
use wiremock::matchers::{method, path};

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
    let scope = Uuid::new_v4();
    
    // Create config with mock server URL
    let config = create_test_config_with_mock_endpoint(mock_server.uri());

    // Set up mock response for LLM call
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-edit",
            "object": "chat.completion",
            "created": 1677652288,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "read_call",
                        "type": "function",
                        "function": {
                            "name": READ_FILE_TOOL,
                            "arguments": json!({
                                "path": test_file_path.to_str().unwrap()
                            }).to_string()
                        }
                    }, {
                        "id": "edit_call",
                        "type": "function",
                        "function": {
                            "name": EDIT_FILE_TOOL,
                            "arguments": json!({
                                "path": test_file_path.to_str().unwrap(),
                                "action": "insert_at_start",
                                "replacement_text": "// Header comment\n"
                            }).to_string()
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "total_tokens": 150
            }
        })))
        .mount(&mock_server)
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
    let file_reader_actor = FileReaderActor::new(
        config.clone(), 
        tx.clone(), 
        file_reader.clone(), 
        scope
    );
    let edit_file_actor = EditFile::new(
        config.clone(),
        tx.clone(),
        file_reader.clone(),
        scope,
    );

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
        if let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(1), rx.recv()).await {
            let msg = msg.unwrap();
            match &msg.message {
                Message::ActorReady { actor_id } => {
                    match actor_id.as_str() {
                        "assistant" => assistant_ready = true,
                        "file_reader" => file_reader_ready = true,
                        "edit_file" => edit_file_ready = true,
                        _ => {}
                    }
                }
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
    let mut seen_assistant_response = false;
    let mut seen_read_tool_call = false;
    let mut seen_edit_tool_call = false;
    let mut seen_awaiting_tools = false;
    let mut seen_read_received = false;
    let mut seen_file_read = false;
    let mut seen_read_finished = false;
    let mut seen_edit_received = false;
    let mut seen_file_edited = false;
    let mut seen_edit_finished = false;
    let mut seen_processing_2 = false;

    while let Ok(msg) =
        tokio::time::timeout(tokio::time::Duration::from_secs(10), rx.recv()).await
    {
        let msg = msg.unwrap();
        println!("Received message: {:?}", msg.message);

        match &msg.message {
            Message::UserContext(hive::actors::UserContext::UserTUIInput(text)) => {
                assert_eq!(text, "Please read and edit the test file");
                seen_user_input = true;
            }
            Message::Agent(agent_msg) if agent_msg.agent_id == scope => {
                if let AgentMessageType::InterAgentMessage(
                    InterAgentMessage::TaskStatusUpdate { status },
                ) = &agent_msg.message
                {
                    match status {
                        AgentStatus::Processing => {
                            if !seen_processing_1 {
                                assert!(seen_user_input, "Processing must come after UserContext");
                                seen_processing_1 = true;
                            } else {
                                assert!(seen_edit_finished, "Final processing must come after edit finished");
                                seen_processing_2 = true;
                                println!("✅ SUCCESS: Edit file workflow completed successfully!");
                                break;
                            }
                        }
                        AgentStatus::AwaitingTools { pending_tool_calls } => {
                            assert!(seen_edit_tool_call, "AwaitingTools must come after tool calls");
                            // Initially 2 tools, then updates as they complete
                            if !seen_awaiting_tools {
                                assert_eq!(pending_tool_calls.len(), 2);
                                seen_awaiting_tools = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Message::AssistantResponse(genai::chat::MessageContent::ToolCalls(calls)) => {
                assert!(seen_processing_1, "AssistantResponse must come after Processing");
                assert_eq!(calls.len(), 2);
                seen_assistant_response = true;
            }
            Message::AssistantToolCall(tc) => {
                assert!(seen_assistant_response, "AssistantToolCall must come after AssistantResponse");
                match tc.fn_name.as_str() {
                    READ_FILE_TOOL => {
                        assert_eq!(tc.call_id, "read_call");
                        seen_read_tool_call = true;
                    }
                    EDIT_FILE_TOOL => {
                        assert_eq!(tc.call_id, "edit_call");
                        seen_edit_tool_call = true;
                    }
                    _ => panic!("Unexpected tool call: {}", tc.fn_name),
                }
            }
            Message::ToolCallUpdate(update) => {
                match (&update.status, update.call_id.as_str()) {
                    (ToolCallStatus::Received { r#type: ToolCallType::ReadFile, .. }, "read_call") => {
                        assert!(seen_read_tool_call, "Read received must come after read tool call");
                        seen_read_received = true;
                    }
                    (ToolCallStatus::Finished(Ok(_)), "read_call") => {
                        assert!(seen_file_read, "Read finished must come after FileRead");
                        seen_read_finished = true;
                    }
                    (ToolCallStatus::Received { r#type: ToolCallType::EditFile, .. }, "edit_call") => {
                        assert!(seen_edit_tool_call, "Edit received must come after edit tool call");
                        seen_edit_received = true;
                    }
                    (ToolCallStatus::Finished(Ok(content)), "edit_call") => {
                        assert!(seen_file_edited, "Edit finished must come after FileEdited");
                        assert!(content.contains("Successfully edited file"));
                        seen_edit_finished = true;
                    }
                    _ => {}
                }
            }
            Message::FileRead { content, .. } => {
                assert!(seen_read_received, "FileRead must come after read received");
                assert_eq!(content, initial_content);
                seen_file_read = true;
            }
            Message::FileEdited { content, .. } => {
                assert!(seen_edit_received, "FileEdited must come after edit received");
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
    assert!(seen_assistant_response, "Missing AssistantResponse");
    assert!(seen_read_tool_call, "Missing read tool call");
    assert!(seen_edit_tool_call, "Missing edit tool call");
    assert!(seen_awaiting_tools, "Missing AwaitingTools");
    assert!(seen_read_received, "Missing read received");
    assert!(seen_file_read, "Missing FileRead");
    assert!(seen_read_finished, "Missing read finished");
    assert!(seen_edit_received, "Missing edit received");
    assert!(seen_file_edited, "Missing FileEdited");
    assert!(seen_edit_finished, "Missing edit finished");
    assert!(seen_processing_2, "Missing final Processing");

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
    let scope = Uuid::new_v4();
    
    // Create config with mock server URL
    let config = create_test_config_with_mock_endpoint(mock_server.uri());

    // Set up mock response for LLM call
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-complete",
            "object": "chat.completion",
            "created": 1677652288,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "complete_call",
                        "type": "function",
                        "function": {
                            "name": "complete",
                            "arguments": json!({
                                "summary": "Task completed successfully",
                                "success": true
                            }).to_string()
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "total_tokens": 150
            }
        })))
        .mount(&mock_server)
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
        if let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(1), rx.recv()).await {
            let msg = msg.unwrap();
            match &msg.message {
                Message::ActorReady { actor_id } => {
                    match actor_id.as_str() {
                        "assistant" => assistant_ready = true,
                        "complete" => complete_ready = true,
                        _ => {}
                    }
                }
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

    while let Ok(msg) =
        tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await
    {
        let msg = msg.unwrap();
        println!("Received message: {:?}", msg.message);

        match &msg.message {
            Message::UserContext(hive::actors::UserContext::UserTUIInput(text)) => {
                assert_eq!(text, "Complete the task");
                seen_user_input = true;
            }
            Message::Agent(agent_msg) if agent_msg.agent_id == scope => {
                if let AgentMessageType::InterAgentMessage(
                    InterAgentMessage::TaskStatusUpdate { status },
                ) = &agent_msg.message
                {
                    match status {
                        AgentStatus::Processing => {
                            if !seen_processing {
                                assert!(seen_user_input, "Processing must come after UserContext");
                                seen_processing = true;
                            }
                        }
                        AgentStatus::AwaitingTools { pending_tool_calls } => {
                            assert!(seen_complete_tool_call, "AwaitingTools must come after tool call");
                            assert_eq!(pending_tool_calls.len(), 1);
                            assert_eq!(pending_tool_calls[0], "complete_call");
                            seen_awaiting_tools = true;
                        }
                        AgentStatus::Done(result) => {
                            assert!(seen_complete_tool_call, "Done must come after complete tool call");
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
                assert!(seen_processing, "AssistantResponse must come after Processing");
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].call_id, "complete_call");
                seen_assistant_response = true;
            }
            Message::AssistantToolCall(tc) => {
                assert!(seen_assistant_response, "AssistantToolCall must come after AssistantResponse");
                assert_eq!(tc.call_id, "complete_call");
                assert_eq!(tc.fn_name, "complete");
                seen_complete_tool_call = true;
            }
            Message::ToolCallUpdate(update) if update.call_id == "complete_call" => {
                match &update.status {
                    ToolCallStatus::Finished(Ok(content)) => {
                        assert!(seen_agent_done, "Complete finished must come after agent Done");
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
    let scope = Uuid::new_v4();
    let agent_to_approve = Uuid::new_v4();
    
    // Create config with mock server URL
    let config = create_test_config_with_mock_endpoint(mock_server.uri());

    // Set up mock response for LLM call
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-approve",
            "object": "chat.completion",
            "created": 1677652288,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "approve_call",
                        "type": "function",
                        "function": {
                            "name": "approve_plan",
                            "arguments": json!({
                                "agent_id": agent_to_approve.to_string()
                            }).to_string()
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "total_tokens": 150
            }
        })))
        .mount(&mock_server)
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
        if let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(1), rx.recv()).await {
            let msg = msg.unwrap();
            match &msg.message {
                Message::ActorReady { actor_id } => {
                    match actor_id.as_str() {
                        "assistant" => assistant_ready = true,
                        "plan_approval" => plan_approval_ready = true,
                        _ => {}
                    }
                }
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

    while let Ok(msg) =
        tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await
    {
        let msg = msg.unwrap();
        println!("Received message: {:?}", msg.message);

        match &msg.message {
            Message::UserContext(hive::actors::UserContext::UserTUIInput(text)) => {
                assert_eq!(text, "Approve the plan");
                seen_user_input = true;
            }
            Message::Agent(agent_msg) if agent_msg.agent_id == scope => {
                if let AgentMessageType::InterAgentMessage(
                    InterAgentMessage::TaskStatusUpdate { status },
                ) = &agent_msg.message
                {
                    match status {
                        AgentStatus::Processing => {
                            assert!(seen_user_input, "Processing must come after UserContext");
                            seen_processing = true;
                        }
                        AgentStatus::AwaitingTools { pending_tool_calls } => {
                            assert!(seen_approve_tool_call, "AwaitingTools must come after tool call");
                            assert_eq!(pending_tool_calls.len(), 1);
                            assert_eq!(pending_tool_calls[0], "approve_call");
                            seen_awaiting_tools = true;
                        }
                        _ => {}
                    }
                }
            }
            Message::Agent(agent_msg) if agent_msg.agent_id == agent_to_approve => {
                if let AgentMessageType::InterAgentMessage(InterAgentMessage::PlanApproved) = &agent_msg.message {
                    assert!(seen_approve_received, "PlanApproved must come after approve received");
                    seen_plan_approved_message = true;
                }
            }
            Message::AssistantResponse(genai::chat::MessageContent::ToolCalls(calls)) => {
                assert!(seen_processing, "AssistantResponse must come after Processing");
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].call_id, "approve_call");
                seen_assistant_response = true;
            }
            Message::AssistantToolCall(tc) => {
                assert!(seen_assistant_response, "AssistantToolCall must come after AssistantResponse");
                assert_eq!(tc.call_id, "approve_call");
                assert_eq!(tc.fn_name, "approve_plan");
                seen_approve_tool_call = true;
            }
            Message::ToolCallUpdate(update) if update.call_id == "approve_call" => {
                match &update.status {
                    ToolCallStatus::Received { r#type: ToolCallType::MCP, .. } => {
                        assert!(seen_approve_tool_call, "Approve received must come after tool call");
                        seen_approve_received = true;
                    }
                    ToolCallStatus::Finished(Ok(content)) => {
                        assert!(seen_plan_approved_message, "Approve finished must come after PlanApproved");
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
    let scope = Uuid::new_v4();
    
    // Create config with mock server URL
    let config = create_test_config_with_mock_endpoint(mock_server.uri());

    // Set up mock response for LLM call
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-spawn",
            "object": "chat.completion",
            "created": 1677652288,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "spawn_call",
                        "type": "function",
                        "function": {
                            "name": "spawn_agents",
                            "arguments": json!({
                                "agents_to_spawn": [{
                                    "agent_role": "Test Worker",
                                    "task_description": "Simple test task",
                                    "agent_type": "Worker"
                                }],
                                "wait": false
                            }).to_string()
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "total_tokens": 150
            }
        })))
        .mount(&mock_server)
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
        if let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(1), rx.recv()).await {
            let msg = msg.unwrap();
            match &msg.message {
                Message::ActorReady { actor_id } => {
                    match actor_id.as_str() {
                        "assistant" => assistant_ready = true,
                        "spawn_agent" => spawn_agent_ready = true,
                        _ => {}
                    }
                }
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

    while let Ok(msg) =
        tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await
    {
        let msg = msg.unwrap();
        println!("Received message: {:?}", msg.message);

        match &msg.message {
            Message::UserContext(hive::actors::UserContext::UserTUIInput(text)) => {
                assert_eq!(text, "Spawn a test agent");
                seen_user_input = true;
            }
            Message::Agent(agent_msg) if agent_msg.agent_id == scope => {
                if let AgentMessageType::InterAgentMessage(
                    InterAgentMessage::TaskStatusUpdate { status },
                ) = &agent_msg.message
                {
                    match status {
                        AgentStatus::Processing => {
                            assert!(seen_user_input, "Processing must come after UserContext");
                            seen_processing = true;
                        }
                        AgentStatus::AwaitingTools { pending_tool_calls } => {
                            assert!(seen_spawn_tool_call, "AwaitingTools must come after tool call");
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
                } = &agent_msg.message {
                    assert!(seen_spawn_received, "AgentSpawned must come after spawn received");
                    assert_eq!(role, "Test Worker");
                    assert_eq!(task_description, "Simple test task");
                    assert_eq!(*agent_type, AgentType::Worker);
                    seen_agent_spawned = true;
                }
            }
            Message::AssistantResponse(genai::chat::MessageContent::ToolCalls(calls)) => {
                assert!(seen_processing, "AssistantResponse must come after Processing");
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].call_id, "spawn_call");
                seen_assistant_response = true;
            }
            Message::AssistantToolCall(tc) => {
                assert!(seen_assistant_response, "AssistantToolCall must come after AssistantResponse");
                assert_eq!(tc.call_id, "spawn_call");
                assert_eq!(tc.fn_name, "spawn_agents");
                seen_spawn_tool_call = true;
            }
            Message::ToolCallUpdate(update) if update.call_id == "spawn_call" => {
                match &update.status {
                    ToolCallStatus::Received { r#type: ToolCallType::SpawnAgent, .. } => {
                        assert!(seen_spawn_tool_call, "Spawn received must come after tool call");
                        seen_spawn_received = true;
                    }
                    ToolCallStatus::Finished(Ok(content)) => {
                        assert!(seen_agent_spawned, "Spawn finished must come after AgentSpawned");
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