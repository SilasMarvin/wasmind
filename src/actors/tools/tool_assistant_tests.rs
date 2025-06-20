#[cfg(test)]
mod tests {
    use crate::actors::assistant::Assistant;
    use crate::actors::tools::command::{Command, TOOL_NAME as COMMAND_TOOL};
    use crate::actors::tools::file_reader::{
        FileReader, FileReaderActor, TOOL_NAME as READ_FILE_TOOL,
    };
    use crate::actors::{
        Actor, ActorMessage, AgentMessageType, AgentStatus, InterAgentMessage,
        Message, ToolCallStatus, ToolCallType,
    };
    use crate::config::create_test_config_with_mock_endpoint;
    use serde_json::json;
    use std::fs;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tokio::sync::broadcast;
    use uuid::Uuid;
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path};


    #[tokio::test]
    async fn test_read_file_tool_call_lifecycle() {
        // Start mock server
        let mock_server = MockServer::start().await;
        
        // Create test file
        let test_file_path = "/tmp/test_tool_lifecycle.txt";
        let test_content = "Test content for tool lifecycle";
        fs::write(test_file_path, test_content).expect("Failed to write test file");

        // Create shared broadcast channel and scope
        let (tx, mut rx) = broadcast::channel(100);
        let scope = Uuid::new_v4();
        
        // Create config with mock server URL
        let config = create_test_config_with_mock_endpoint(mock_server.uri());

        // Set up mock response for LLM call
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "chatcmpl-123",
                "object": "chat.completion",
                "created": 1677652288,
                "model": "test-model",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "test_call_123",
                            "type": "function",
                            "function": {
                                "name": READ_FILE_TOOL,
                                "arguments": json!({
                                    "path": test_file_path
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

        // Create assistant
        let assistant = Assistant::new(
            config.hive.main_manager_model.clone(),
            tx.clone(),
            scope,
            vec![FileReaderActor::ACTOR_ID],
            None,
            vec![],
        );

        // Create file reader tool
        let file_reader = Arc::new(Mutex::new(FileReader::default()));
        let file_reader_actor =
            FileReaderActor::new(config.clone(), tx.clone(), file_reader, scope);

        // Start both actors using run() method
        assistant.run();
        file_reader_actor.run();

        // Wait for initial ActorReady messages (can come in any order)
        let mut assistant_ready = false;
        let mut file_reader_ready = false;
        let mut tools_available = false;
        
        while !assistant_ready || !file_reader_ready || !tools_available {
            if let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(1), rx.recv()).await {
                let msg = msg.unwrap();
                match &msg.message {
                    Message::ActorReady { actor_id } => {
                        if actor_id == "assistant" {
                            assistant_ready = true;
                        } else if actor_id == "file_reader" {
                            file_reader_ready = true;
                        }
                    }
                    Message::ToolsAvailable(tools) => {
                        assert_eq!(tools.len(), 1);
                        assert_eq!(tools[0].name, READ_FILE_TOOL);
                        tools_available = true;
                    }
                    _ => panic!("Unexpected message during setup: {:?}", msg.message),
                }
            } else {
                panic!("Timeout waiting for initial setup messages");
            }
        }

        // Wait a bit after file_reader is ready
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Verify assistant is in Idle state
        let idle_msg = tokio::time::timeout(tokio::time::Duration::from_millis(100), rx.recv())
            .await
            .expect("Timeout waiting for idle state")
            .expect("Channel closed");
        
        if let Message::Agent(agent_msg) = &idle_msg.message {
            assert_eq!(agent_msg.agent_id, scope);
            if let AgentMessageType::InterAgentMessage(
                InterAgentMessage::TaskStatusUpdate { status },
            ) = &agent_msg.message
            {
                assert!(matches!(status, AgentStatus::Idle), 
                       "Assistant should be Idle after setup, got: {:?}", status);
            } else {
                panic!("Expected TaskStatusUpdate, got: {:?}", agent_msg.message);
            }
        } else {
            panic!("Expected Agent message for idle state, got: {:?}", idle_msg.message);
        }

        // Now send user input to trigger LLM call
        tx.send(ActorMessage {
            scope,
            message: Message::UserContext(crate::actors::UserContext::UserTUIInput(
                "Please read the test file".to_string(),
            )),
        })
        .unwrap();

        // Track which messages we've seen to verify causality
        let mut seen_user_input = false;
        let mut seen_processing_1 = false;
        let mut seen_assistant_response = false;
        let mut seen_assistant_tool_call = false;
        let mut seen_awaiting_tools = false;
        let mut seen_tool_received = false;
        let mut seen_file_read = false;
        let mut seen_tool_finished = false;
        let mut seen_processing_2 = false;

        // Causality constraints to verify:
        // 1. UserContext must come before first Processing state
        // 2. Processing state must come before AssistantResponse
        // 3. AssistantResponse must come before AssistantToolCall
        // 4. AssistantToolCall must come before any ToolCallUpdate for that call_id
        // 5. ToolCallUpdate(Received) must come before FileRead
        // 6. FileRead must come before ToolCallUpdate(Finished)
        // 7. ToolCallUpdate(Finished) must come before final Processing state

        while let Ok(msg) =
            tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await
        {
            let msg = msg.unwrap();
            println!("Received message: {:?}", msg.message);

            match &msg.message {
                Message::UserContext(crate::actors::UserContext::UserTUIInput(text)) => {
                    assert_eq!(text, "Please read the test file");
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
                                    // First Processing state - must come after UserContext
                                    assert!(seen_user_input, "Processing state must come after UserContext");
                                    seen_processing_1 = true;
                                } else {
                                    // Second Processing state - must come after tool finished
                                    assert!(seen_tool_finished, "Final Processing state must come after ToolCallUpdate(Finished)");
                                    seen_processing_2 = true;
                                    println!("✅ SUCCESS: All causal constraints verified!");
                                    break;
                                }
                            }
                            AgentStatus::AwaitingTools { pending_tool_calls } => {
                                // AwaitingTools must come after AssistantToolCall
                                assert!(seen_assistant_tool_call, "AwaitingTools must come after AssistantToolCall");
                                assert_eq!(pending_tool_calls.len(), 1);
                                assert_eq!(pending_tool_calls[0], "test_call_123");
                                seen_awaiting_tools = true;
                            }
                            _ => {} // Ignore other states
                        }
                    }
                }
                Message::AssistantResponse(genai::chat::MessageContent::ToolCalls(calls)) => {
                    // AssistantResponse must come after first Processing state
                    assert!(seen_processing_1, "AssistantResponse must come after Processing state");
                    assert_eq!(calls.len(), 1);
                    assert_eq!(calls[0].call_id, "test_call_123");
                    seen_assistant_response = true;
                }
                Message::AssistantToolCall(tc) => {
                    // AssistantToolCall must come after AssistantResponse
                    assert!(seen_assistant_response, "AssistantToolCall must come after AssistantResponse");
                    assert_eq!(tc.call_id, "test_call_123");
                    seen_assistant_tool_call = true;
                }
                Message::ToolCallUpdate(update) if update.call_id == "test_call_123" => {
                    match &update.status {
                        ToolCallStatus::Received { r#type: ToolCallType::ReadFile, .. } => {
                            // ToolCallUpdate(Received) must come after AssistantToolCall
                            assert!(seen_assistant_tool_call, "ToolCallUpdate(Received) must come after AssistantToolCall");
                            seen_tool_received = true;
                        }
                        ToolCallStatus::Finished(Ok(content)) => {
                            // ToolCallUpdate(Finished) must come after FileRead
                            assert!(seen_file_read, "ToolCallUpdate(Finished) must come after FileRead");
                            assert!(content.contains(test_file_path));
                            seen_tool_finished = true;
                        }
                        _ => panic!("Unexpected tool call status: {:?}", update.status),
                    }
                }
                Message::FileRead { path, content, .. } => {
                    // FileRead must come after ToolCallUpdate(Received)
                    assert!(seen_tool_received, "FileRead must come after ToolCallUpdate(Received)");
                    assert!(path.to_str().unwrap().ends_with("test_tool_lifecycle.txt"));
                    assert_eq!(content, test_content);
                    seen_file_read = true;
                }
                _ => {} // Ignore other messages
            }
        }

        // Verify we saw all expected messages
        assert!(seen_user_input, "Missing UserContext message");
        assert!(seen_processing_1, "Missing first Processing state");
        assert!(seen_assistant_response, "Missing AssistantResponse");
        assert!(seen_assistant_tool_call, "Missing AssistantToolCall");
        assert!(seen_awaiting_tools, "Missing AwaitingTools state");
        assert!(seen_tool_received, "Missing ToolCallUpdate(Received)");
        assert!(seen_file_read, "Missing FileRead");
        assert!(seen_tool_finished, "Missing ToolCallUpdate(Finished)");
        assert!(seen_processing_2, "Missing final Processing state");

        // Clean up
        fs::remove_file(test_file_path).ok();
    }

    #[tokio::test]
    async fn test_command_tool_with_approval() {
        // Start mock server
        let mock_server = MockServer::start().await;
        
        // Create shared broadcast channel and scope
        let (tx, mut rx) = broadcast::channel(100);
        let scope = Uuid::new_v4();
        
        // Create config with mock server URL and command approval required
        let mut config = create_test_config_with_mock_endpoint(mock_server.uri());
        config.auto_approve_commands = false; // Require approval

        // Set up mock response for LLM call
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "chatcmpl-456",
                "object": "chat.completion",
                "created": 1677652288,
                "model": "gpt-4o",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "cmd_call_456",
                            "type": "function",
                            "function": {
                                "name": COMMAND_TOOL,
                                "arguments": json!({
                                    "command": "/bin/sh",
                                    "args": ["-c", "echo test command"]
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

        // Create assistant
        let assistant = Assistant::new(
            config.hive.main_manager_model.clone(),
            tx.clone(),
            scope,
            vec![Command::ACTOR_ID],
            None,
            vec![],
        );

        // Create command tool
        let command_tool = Command::new(config.clone(), tx.clone(), scope);

        // Start both actors using run() method
        assistant.run();
        command_tool.run();

        // Wait for initial ActorReady messages (can come in any order)
        let mut assistant_ready = false;
        let mut command_ready = false;
        let mut tools_available = false;
        
        while !assistant_ready || !command_ready || !tools_available {
            if let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(1), rx.recv()).await {
                let msg = msg.unwrap();
                match &msg.message {
                    Message::ActorReady { actor_id } => {
                        if actor_id == "assistant" {
                            assistant_ready = true;
                        } else if actor_id == "command" {
                            command_ready = true;
                        }
                    }
                    Message::ToolsAvailable(tools) => {
                        assert_eq!(tools.len(), 1);
                        assert_eq!(tools[0].name, COMMAND_TOOL);
                        tools_available = true;
                    }
                    _ => panic!("Unexpected message during setup: {:?}", msg.message),
                }
            } else {
                panic!("Timeout waiting for initial setup messages");
            }
        }

        // Wait a bit after command tool is ready
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Verify assistant is in Idle state
        let idle_msg = tokio::time::timeout(tokio::time::Duration::from_millis(100), rx.recv())
            .await
            .expect("Timeout waiting for idle state")
            .expect("Channel closed");
        
        if let Message::Agent(agent_msg) = &idle_msg.message {
            assert_eq!(agent_msg.agent_id, scope);
            if let AgentMessageType::InterAgentMessage(
                InterAgentMessage::TaskStatusUpdate { status },
            ) = &agent_msg.message
            {
                assert!(matches!(status, AgentStatus::Idle), 
                       "Assistant should be Idle after setup, got: {:?}", status);
            } else {
                panic!("Expected TaskStatusUpdate, got: {:?}", agent_msg.message);
            }
        } else {
            panic!("Expected Agent message for idle state, got: {:?}", idle_msg.message);
        }

        // Now send user input to trigger LLM call
        tx.send(ActorMessage {
            scope,
            message: Message::UserContext(crate::actors::UserContext::UserTUIInput(
                "Please run a test command".to_string(),
            )),
        })
        .unwrap();

        // Track which messages we've seen to verify causality for command approval flow
        let mut seen_user_input = false;
        let mut seen_processing_1 = false;
        let mut seen_assistant_response = false;
        let mut seen_assistant_tool_call = false;
        let mut seen_awaiting_tools = false;
        let mut seen_tool_received = false;
        let mut seen_tool_awaiting_approval = false;
        let mut seen_tool_finished = false;
        let mut seen_processing_2 = false;

        // Causality constraints for command approval flow:
        // 1. UserContext must come before first Processing state
        // 2. Processing state must come before AssistantResponse
        // 3. AssistantResponse must come before AssistantToolCall
        // 4. AssistantToolCall must come before any ToolCallUpdate for that call_id
        // 5. ToolCallUpdate(Received) must come before ToolCallUpdate(AwaitingUserYNConfirmation)
        // 6. We'll send approval after seeing AwaitingUserYNConfirmation
        // 7. ToolCallUpdate(Finished) must come after approval
        // 8. Final Processing state must come after ToolCallUpdate(Finished)

        while let Ok(msg) =
            tokio::time::timeout(tokio::time::Duration::from_secs(10), rx.recv()).await
        {
            let msg = msg.unwrap();
            println!("Received message: {:?}", msg.message);

            match &msg.message {
                Message::UserContext(crate::actors::UserContext::UserTUIInput(text)) => {
                    assert_eq!(text, "Please run a test command");
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
                                    // First Processing state - must come after UserContext
                                    assert!(seen_user_input, "Processing state must come after UserContext");
                                    seen_processing_1 = true;
                                } else {
                                    // Second Processing state - must come after tool finished
                                    assert!(seen_tool_finished, "Final Processing state must come after ToolCallUpdate(Finished)");
                                    seen_processing_2 = true;
                                    println!("✅ SUCCESS: All causal constraints verified for command approval flow!");
                                    break;
                                }
                            }
                            AgentStatus::AwaitingTools { pending_tool_calls } => {
                                // AwaitingTools must come after AssistantToolCall
                                assert!(seen_assistant_tool_call, "AwaitingTools must come after AssistantToolCall");
                                assert_eq!(pending_tool_calls.len(), 1);
                                assert_eq!(pending_tool_calls[0], "cmd_call_456");
                                seen_awaiting_tools = true;
                            }
                            _ => {} // Ignore other states
                        }
                    }
                }
                Message::AssistantResponse(genai::chat::MessageContent::ToolCalls(calls)) => {
                    // AssistantResponse must come after first Processing state
                    assert!(seen_processing_1, "AssistantResponse must come after Processing state");
                    assert_eq!(calls.len(), 1);
                    assert_eq!(calls[0].call_id, "cmd_call_456");
                    seen_assistant_response = true;
                }
                Message::AssistantToolCall(tc) => {
                    // AssistantToolCall must come after AssistantResponse
                    assert!(seen_assistant_response, "AssistantToolCall must come after AssistantResponse");
                    assert_eq!(tc.call_id, "cmd_call_456");
                    seen_assistant_tool_call = true;
                }
                Message::ToolCallUpdate(update) if update.call_id == "cmd_call_456" => {
                    match &update.status {
                        ToolCallStatus::Received { r#type: ToolCallType::Command, .. } => {
                            // ToolCallUpdate(Received) must come after AssistantToolCall
                            assert!(seen_assistant_tool_call, "ToolCallUpdate(Received) must come after AssistantToolCall");
                            seen_tool_received = true;
                        }
                        ToolCallStatus::AwaitingUserYNConfirmation => {
                            // AwaitingUserYNConfirmation must come after Received
                            assert!(seen_tool_received, "ToolCallUpdate(AwaitingUserYNConfirmation) must come after ToolCallUpdate(Received)");
                            seen_tool_awaiting_approval = true;
                            
                            // Send approval
                            tx.send(ActorMessage {
                                scope,
                                message: Message::ToolCallUpdate(
                                    crate::actors::ToolCallUpdate {
                                        call_id: "cmd_call_456".to_string(),
                                        status: ToolCallStatus::ReceivedUserYNConfirmation(true),
                                    },
                                ),
                            })
                            .unwrap();
                        }
                        ToolCallStatus::Finished(Ok(output)) => {
                            // ToolCallUpdate(Finished) must come after we sent approval
                            assert!(seen_tool_awaiting_approval, "ToolCallUpdate(Finished) must come after AwaitingUserYNConfirmation");
                            assert!(output.contains("test command"));
                            seen_tool_finished = true;
                        }
                        ToolCallStatus::ReceivedUserYNConfirmation(_) => {
                            // This is the approval message we sent - ignore it
                        }
                        _ => panic!("Unexpected tool call status: {:?}", update.status),
                    }
                }
                _ => {} // Ignore other messages
            }
        }

        // Verify we saw all expected messages
        assert!(seen_user_input, "Missing UserContext message");
        assert!(seen_processing_1, "Missing first Processing state");
        assert!(seen_assistant_response, "Missing AssistantResponse");
        assert!(seen_assistant_tool_call, "Missing AssistantToolCall");
        assert!(seen_awaiting_tools, "Missing AwaitingTools state");
        assert!(seen_tool_received, "Missing ToolCallUpdate(Received)");
        assert!(seen_tool_awaiting_approval, "Missing ToolCallUpdate(AwaitingUserYNConfirmation)");
        assert!(seen_tool_finished, "Missing ToolCallUpdate(Finished)");
        assert!(seen_processing_2, "Missing final Processing state");
    }

    #[tokio::test]
    async fn test_tool_error_handling() {
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
                "id": "chatcmpl-789",
                "object": "chat.completion",
                "created": 1677652288,
                "model": "gpt-4o",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "error_call",
                            "type": "function",
                            "function": {
                                "name": READ_FILE_TOOL,
                                "arguments": json!({
                                    "path": "/nonexistent/file/path.txt"
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

        // Create assistant
        let assistant = Assistant::new(
            config.hive.main_manager_model.clone(),
            tx.clone(),
            scope,
            vec![FileReaderActor::ACTOR_ID],
            None,
            vec![],
        );

        // Create file reader tool
        let file_reader = Arc::new(Mutex::new(FileReader::default()));
        let file_reader_actor =
            FileReaderActor::new(config.clone(), tx.clone(), file_reader, scope);

        // Start both actors using run() method
        assistant.run();
        file_reader_actor.run();

        // Wait for initial ActorReady messages (can come in any order)
        let mut assistant_ready = false;
        let mut file_reader_ready = false;
        let mut tools_available = false;
        
        while !assistant_ready || !file_reader_ready || !tools_available {
            if let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(1), rx.recv()).await {
                let msg = msg.unwrap();
                match &msg.message {
                    Message::ActorReady { actor_id } => {
                        if actor_id == "assistant" {
                            assistant_ready = true;
                        } else if actor_id == "file_reader" {
                            file_reader_ready = true;
                        }
                    }
                    Message::ToolsAvailable(tools) => {
                        assert_eq!(tools.len(), 1);
                        assert_eq!(tools[0].name, READ_FILE_TOOL);
                        tools_available = true;
                    }
                    _ => panic!("Unexpected message during setup: {:?}", msg.message),
                }
            } else {
                panic!("Timeout waiting for initial setup messages");
            }
        }

        // Wait a bit after file_reader is ready
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Verify assistant is in Idle state
        let idle_msg = tokio::time::timeout(tokio::time::Duration::from_millis(100), rx.recv())
            .await
            .expect("Timeout waiting for idle state")
            .expect("Channel closed");
        
        if let Message::Agent(agent_msg) = &idle_msg.message {
            assert_eq!(agent_msg.agent_id, scope);
            if let AgentMessageType::InterAgentMessage(
                InterAgentMessage::TaskStatusUpdate { status },
            ) = &agent_msg.message
            {
                assert!(matches!(status, AgentStatus::Idle), 
                       "Assistant should be Idle after setup, got: {:?}", status);
            } else {
                panic!("Expected TaskStatusUpdate, got: {:?}", agent_msg.message);
            }
        } else {
            panic!("Expected Agent message for idle state, got: {:?}", idle_msg.message);
        }

        // Now send user input to trigger LLM call
        tx.send(ActorMessage {
            scope,
            message: Message::UserContext(crate::actors::UserContext::UserTUIInput(
                "Please read a nonexistent file".to_string(),
            )),
        })
        .unwrap();

        // Track which messages we've seen to verify causality for error handling flow
        let mut seen_user_input = false;
        let mut seen_processing_1 = false;
        let mut seen_assistant_response = false;
        let mut seen_assistant_tool_call = false;
        let mut seen_awaiting_tools = false;
        let mut seen_tool_received = false;
        let mut seen_tool_error = false;
        let mut seen_processing_2 = false;

        // Causality constraints for error handling flow:
        // 1. UserContext must come before first Processing state
        // 2. Processing state must come before AssistantResponse
        // 3. AssistantResponse must come before AssistantToolCall
        // 4. AssistantToolCall must come before any ToolCallUpdate for that call_id
        // 5. ToolCallUpdate(Received) must come before ToolCallUpdate(Finished(Err))
        // 6. ToolCallUpdate(Finished(Err)) must come before final Processing state

        while let Ok(msg) =
            tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await
        {
            let msg = msg.unwrap();
            println!("Received message: {:?}", msg.message);

            match &msg.message {
                Message::UserContext(crate::actors::UserContext::UserTUIInput(text)) => {
                    assert_eq!(text, "Please read a nonexistent file");
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
                                    // First Processing state - must come after UserContext
                                    assert!(seen_user_input, "Processing state must come after UserContext");
                                    seen_processing_1 = true;
                                } else {
                                    // Second Processing state - must come after tool error
                                    assert!(seen_tool_error, "Final Processing state must come after ToolCallUpdate(Finished(Err))");
                                    seen_processing_2 = true;
                                    println!("✅ SUCCESS: All causal constraints verified for error handling flow!");
                                    break;
                                }
                            }
                            AgentStatus::AwaitingTools { pending_tool_calls } => {
                                // AwaitingTools must come after AssistantToolCall
                                assert!(seen_assistant_tool_call, "AwaitingTools must come after AssistantToolCall");
                                assert_eq!(pending_tool_calls.len(), 1);
                                assert_eq!(pending_tool_calls[0], "error_call");
                                seen_awaiting_tools = true;
                            }
                            _ => {} // Ignore other states
                        }
                    }
                }
                Message::AssistantResponse(genai::chat::MessageContent::ToolCalls(calls)) => {
                    // AssistantResponse must come after first Processing state
                    assert!(seen_processing_1, "AssistantResponse must come after Processing state");
                    assert_eq!(calls.len(), 1);
                    assert_eq!(calls[0].call_id, "error_call");
                    seen_assistant_response = true;
                }
                Message::AssistantToolCall(tc) => {
                    // AssistantToolCall must come after AssistantResponse
                    assert!(seen_assistant_response, "AssistantToolCall must come after AssistantResponse");
                    assert_eq!(tc.call_id, "error_call");
                    seen_assistant_tool_call = true;
                }
                Message::ToolCallUpdate(update) if update.call_id == "error_call" => {
                    match &update.status {
                        ToolCallStatus::Received { r#type: ToolCallType::ReadFile, .. } => {
                            // ToolCallUpdate(Received) must come after AssistantToolCall
                            assert!(seen_assistant_tool_call, "ToolCallUpdate(Received) must come after AssistantToolCall");
                            seen_tool_received = true;
                        }
                        ToolCallStatus::Finished(Err(err)) => {
                            // ToolCallUpdate(Finished(Err)) must come after Received
                            assert!(seen_tool_received, "ToolCallUpdate(Finished(Err)) must come after ToolCallUpdate(Received)");
                            assert!(
                                err.contains("No such file")
                                    || err.contains("not found")
                                    || err.contains("does not exist"),
                                "Expected file not found error, got: {}", err
                            );
                            seen_tool_error = true;
                        }
                        _ => panic!("Unexpected tool call status: {:?}", update.status),
                    }
                }
                _ => {} // Ignore other messages
            }
        }

        // Verify we saw all expected messages
        assert!(seen_user_input, "Missing UserContext message");
        assert!(seen_processing_1, "Missing first Processing state");
        assert!(seen_assistant_response, "Missing AssistantResponse");
        assert!(seen_assistant_tool_call, "Missing AssistantToolCall");
        assert!(seen_awaiting_tools, "Missing AwaitingTools state");
        assert!(seen_tool_received, "Missing ToolCallUpdate(Received)");
        assert!(seen_tool_error, "Missing ToolCallUpdate(Finished(Err))");
        assert!(seen_processing_2, "Missing final Processing state");
    }
}
