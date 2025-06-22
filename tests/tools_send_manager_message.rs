// mod common;
//
// use hive::actors::assistant::Assistant;
// use hive::actors::tools::send_manager_message::{
//     SEND_MANAGER_MESSAGE_TOOL_NAME, SendManagerMessage, format_send_manager_message_success,
// };
// use hive::actors::{
//     Actor, ActorMessage, AgentMessageType, AgentStatus, InterAgentMessage, Message, ToolCallStatus,
//     WaitReason,
// };
// use hive::scope::Scope;
// use tokio::sync::broadcast;
// use wiremock::MockServer;
//
// #[tokio::test]
// async fn test_send_manager_message_tool() {
//     // Start mock server
//     let mock_server = MockServer::start().await;
//
//     // Create shared broadcast channel and scopes
//     let (tx, mut rx) = broadcast::channel(1000);
//     let child_scope = Scope::new();
//     let manager_scope = Scope::new();
//
//     // Create config with mock server URL
//     let config = common::create_test_config_with_mock_endpoint(mock_server.uri());
//
//     // Set up mock LLM response for send_manager_message tool call
//     common::create_mock_sequence(
//         &mock_server,
//         child_scope.clone(),
//         "Send a message to my manager",
//     )
//     .responds_with_tool_call(
//         "chatcmpl-send-manager-msg",
//         "send_manager_message_call",
//         SEND_MANAGER_MESSAGE_TOOL_NAME,
//         serde_json::json!({
//             "message": "I need clarification on the requirements",
//             "wait": false
//         }),
//     )
//     .build()
//     .await;
//
//     // Create child assistant with send_manager_message tool
//     let assistant = Assistant::new(
//         config.hive.worker_model.clone(),
//         tx.clone(),
//         child_scope.clone(),
//         vec![SendManagerMessage::ACTOR_ID],
//         None,
//         vec![],
//     );
//
//     // Create send_manager_message tool
//     let send_manager_message = SendManagerMessage::new(
//         config.clone(),
//         tx.clone(),
//         child_scope.clone(),
//         manager_scope.clone(),
//     );
//
//     // Start actors
//     assistant.run();
//     send_manager_message.run();
//
//     // Wait for setup
//     let mut ready_count = 0;
//     while ready_count < 3 {
//         // 2 actors + 1 tools available
//         if let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await
//         {
//             let msg = msg.unwrap();
//             match &msg.message {
//                 Message::ActorReady { .. } | Message::ToolsAvailable(_) => ready_count += 1,
//                 _ => {}
//             }
//         }
//     }
//
//     // Send user input
//     tx.send(ActorMessage {
//         scope: child_scope.clone(),
//         message: Message::UserContext(hive::actors::UserContext::UserTUIInput(
//             "Send a message to my manager".to_string(),
//         )),
//     })
//     .unwrap();
//
//     // Track expected messages
//     let mut seen_user_input = false;
//     let mut seen_processing = false;
//     let mut seen_tool_call = false;
//     let mut seen_awaiting_tools = false;
//     let mut seen_sub_agent_message = false;
//     let mut seen_tool_finished = false;
//
//     while let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await {
//         let msg = msg.unwrap();
//
//         println!("{msg:?}");
//
//         match &msg.message {
//             Message::UserContext(hive::actors::UserContext::UserTUIInput(text)) => {
//                 assert_eq!(text, "Send a message to my manager");
//                 seen_user_input = true;
//             }
//             Message::Agent(agent_msg) => match &agent_msg.message {
//                 AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
//                     status,
//                 }) if agent_msg.agent_id == child_scope => match status {
//                     AgentStatus::Processing => {
//                         assert!(seen_user_input, "Processing must come after UserContext");
//                         seen_processing = true;
//                     }
//                     AgentStatus::AwaitingTools { pending_tool_calls } => {
//                         assert!(seen_processing, "Processing must come before AwaitingTools");
//                         assert!(seen_tool_call, "AwaitingTools must come after tool call");
//                         assert_eq!(pending_tool_calls.len(), 1);
//                         assert_eq!(pending_tool_calls[0], "send_manager_message_call");
//                         seen_awaiting_tools = true;
//                     }
//                     _ => {}
//                 },
//                 AgentMessageType::InterAgentMessage(InterAgentMessage::SubAgentMessage {
//                     message,
//                 }) if agent_msg.agent_id == manager_scope => {
//                     assert!(seen_tool_call, "SubAgentMessage must come after tool call");
//                     assert_eq!(message, "I need clarification on the requirements");
//                     seen_sub_agent_message = true;
//                 }
//                 _ => {}
//             },
//             Message::AssistantToolCall(tool_call) => {
//                 assert!(seen_processing, "Tool call must come after Processing");
//                 assert_eq!(tool_call.fn_name, SEND_MANAGER_MESSAGE_TOOL_NAME);
//                 assert_eq!(tool_call.call_id, "send_manager_message_call");
//                 seen_tool_call = true;
//             }
//             Message::ToolCallUpdate(update) => {
//                 if let ToolCallStatus::Finished(Ok(result)) = &update.status {
//                     assert!(
//                         seen_awaiting_tools,
//                         "Tool finish must come after AwaitingTools"
//                     );
//                     assert_eq!(update.call_id, "send_manager_message_call");
//                     assert_eq!(result, &format_send_manager_message_success(false));
//                     seen_tool_finished = true;
//                     break; // Test complete
//                 }
//             }
//             _ => {}
//         }
//     }
//
//     // Verify all steps occurred
//     assert!(seen_user_input, "Should receive user input");
//     assert!(seen_processing, "Should see child processing");
//     assert!(seen_tool_call, "Should see send_manager_message tool call");
//     assert!(seen_awaiting_tools, "Should see child awaiting tools");
//     assert!(
//         seen_sub_agent_message,
//         "Should see SubAgentMessage sent to manager"
//     );
//     assert!(seen_tool_finished, "Should see tool call finished");
// }
//
// #[tokio::test]
// #[cfg_attr(not(feature = "test-utils"), ignore)]
// async fn test_send_manager_message_tool_wait() {
//     // Start mock server
//     let mock_server = MockServer::start().await;
//
//     // Create shared broadcast channel and scopes
//     let (tx, mut rx) = broadcast::channel(1000);
//     let child_scope = Scope::new();
//     let manager_scope = Scope::new();
//
//     // Create config with mock server URL
//     let config = common::create_test_config_with_mock_endpoint(mock_server.uri());
//
//     // Set up mock LLM response for send_manager_message tool call with wait=true
//     common::create_mock_sequence(
//         &mock_server,
//         child_scope.clone(),
//         "Send a message to my manager and wait for response",
//     )
//     .responds_with_tool_call(
//         "chatcmpl-send-manager-msg",
//         "send_manager_message_call",
//         SEND_MANAGER_MESSAGE_TOOL_NAME,
//         serde_json::json!({
//             "message": "I need guidance on how to proceed with this task",
//             "wait": true
//         }),
//     )
//     .build()
//     .await;
//
//     // Create child assistant with send_manager_message tool
//     let assistant = Assistant::new(
//         config.hive.worker_model.clone(),
//         tx.clone(),
//         child_scope.clone(),
//         vec![SendManagerMessage::ACTOR_ID],
//         None,
//         vec![],
//     );
//
//     // Create send_manager_message tool
//     let send_manager_message = SendManagerMessage::new(
//         config.clone(),
//         tx.clone(),
//         child_scope.clone(),
//         manager_scope.clone(),
//     );
//
//     // Start actors
//     assistant.run();
//     send_manager_message.run();
//
//     // Wait for setup
//     let mut ready_count = 0;
//     while ready_count < 3 {
//         // 2 actors + 1 tools available
//         if let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await
//         {
//             let msg = msg.unwrap();
//             match &msg.message {
//                 Message::ActorReady { .. } | Message::ToolsAvailable(_) => ready_count += 1,
//                 _ => {}
//             }
//         }
//     }
//
//     // Send user input
//     tx.send(ActorMessage {
//         scope: child_scope.clone(),
//         message: Message::UserContext(hive::actors::UserContext::UserTUIInput(
//             "Send a message to my manager and wait for response".to_string(),
//         )),
//     })
//     .unwrap();
//
//     // Track expected messages
//     let mut seen_user_input = false;
//     let mut seen_processing = false;
//     let mut seen_tool_call = false;
//     let mut seen_awaiting_tools = false;
//     let mut seen_sub_agent_message = false;
//     let mut seen_child_wait = false;
//     let mut seen_tool_finished = false;
//
//     while let Ok(msg) = tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await {
//         let msg = msg.unwrap();
//
//         println!("{msg:?}");
//
//         match &msg.message {
//             Message::UserContext(hive::actors::UserContext::UserTUIInput(text)) => {
//                 assert_eq!(text, "Send a message to my manager and wait for response");
//                 seen_user_input = true;
//             }
//             Message::Agent(agent_msg) => match &agent_msg.message {
//                 AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
//                     status,
//                 }) if agent_msg.agent_id == child_scope => match status {
//                     AgentStatus::Processing => {
//                         assert!(seen_user_input, "Processing must come after UserContext");
//                         seen_processing = true;
//                     }
//                     AgentStatus::AwaitingTools { pending_tool_calls } => {
//                         assert!(seen_processing, "Processing must come before AwaitingTools");
//                         assert!(seen_tool_call, "AwaitingTools must come after tool call");
//                         assert_eq!(pending_tool_calls.len(), 1);
//                         assert_eq!(pending_tool_calls[0], "send_manager_message_call");
//                         seen_awaiting_tools = true;
//                     }
//                     AgentStatus::Wait {
//                         tool_call_id,
//                         reason,
//                     } => {
//                         assert!(seen_awaiting_tools, "Wait must come after AwaitingTools");
//                         assert_eq!(tool_call_id, "send_manager_message_call");
//                         assert!(matches!(reason, WaitReason::WaitingForManagerResponse));
//                         seen_child_wait = true;
//                     }
//                     _ => {}
//                 },
//                 AgentMessageType::InterAgentMessage(InterAgentMessage::SubAgentMessage {
//                     message,
//                 }) if agent_msg.agent_id == manager_scope => {
//                     assert!(seen_tool_call, "SubAgentMessage must come after tool call");
//                     assert_eq!(message, "I need guidance on how to proceed with this task");
//                     seen_sub_agent_message = true;
//                 }
//                 _ => {}
//             },
//             Message::AssistantToolCall(tool_call) => {
//                 assert!(seen_processing, "Tool call must come after Processing");
//                 assert_eq!(tool_call.fn_name, SEND_MANAGER_MESSAGE_TOOL_NAME);
//                 assert_eq!(tool_call.call_id, "send_manager_message_call");
//                 seen_tool_call = true;
//             }
//             Message::ToolCallUpdate(update) => {
//                 if let ToolCallStatus::Finished(Ok(result)) = &update.status {
//                     assert!(
//                         seen_awaiting_tools,
//                         "Tool finish must come after AwaitingTools"
//                     );
//                     assert_eq!(update.call_id, "send_manager_message_call");
//                     assert_eq!(result, &format_send_manager_message_success(true));
//                     seen_tool_finished = true;
//                     break; // Test complete
//                 }
//             }
//             _ => {}
//         }
//     }
//
//     // Verify all steps occurred
//     assert!(seen_user_input, "Should receive user input");
//     assert!(seen_processing, "Should see child processing");
//     assert!(seen_tool_call, "Should see send_manager_message tool call");
//     assert!(seen_awaiting_tools, "Should see child awaiting tools");
//     assert!(
//         seen_sub_agent_message,
//         "Should see SubAgentMessage sent to manager"
//     );
//     assert!(seen_child_wait, "Should see child in Wait state");
//     assert!(seen_tool_finished, "Should see tool finish");
// }
