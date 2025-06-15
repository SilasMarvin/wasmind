mod common;

use hive::actors::{ActorMessage, Message, ToolCallStatus, ToolCallType};
use hive::config::Config;
use hive::hive::start_headless_hive;
use std::fs;
use tokio::sync::broadcast;
use tokio::time::{Duration, timeout};

#[test]
fn test_file_read_message_order_for_file_read() {
    // Initialize test logger
    common::init_test_logger();

    let runtime = tokio::runtime::Runtime::new().unwrap();

    // Create a test file
    let test_file_path = "/tmp/test_hive_file.txt";
    let test_content = "Hello from the test file!";
    fs::write(test_file_path, test_content).expect("Failed to write test file");

    // Create broadcast channel for testing
    let (tx, mut rx) = broadcast::channel::<ActorMessage>(1024);

    // Load test config using Config::from_file
    let config = Config::from_file("tests/test_config.toml")
        .expect("Failed to load test config")
        .try_into()
        .expect("Failed to parse test config");

    // Start hive with a file reading prompt
    let prompt = format!(
        "What are the contents of the file {}? Use a sub agent and `wait` on it.",
        test_file_path
    );
    let _handle = start_headless_hive(&runtime, config, prompt.clone(), Some(tx.clone()));

    runtime.block_on(async move {

    // Track our position in expected message sequence
    let mut index = 0;

    // Listen for messages with timeout
    let result = timeout(Duration::from_secs(45), async {
        loop {
            match rx.recv().await {
                Ok(actor_msg) => {
                    let msg = &actor_msg.message;

                    // Check expected messages in order
                    // 1. MainManager calls spawn_agents fn
                    // 2. spawn_agents fn broadcasts Received
                    // 3. spawn_agents fn broadcasts InterAgentMessage task status update to Waiting for MainManager
                    // 4. spawn_agents fn is finished
                    // 5. Worker sub agent calls read_file tool
                    // 6. read_file tool broadcasts finished
                    // 7. Worker sub agent calls complete fn
                    // 8. complete fn broadcasts InterAgentMessage task status update to Done for
                    //    MainManager
                    // 9. MainManager calls complete fn
                    if index == 0 && matches!(msg, Message::AssistantToolCall(tool_call) if tool_call.fn_name == "spawn_agents") {
                        index += 1;
                    } else if index == 1 && matches!(msg, Message::ToolCallUpdate(update) if matches!(update.status, ToolCallStatus::Received {r#type: ToolCallType::SpawnAgent, friendly_command_display: _ })) {
                        index += 1;
                    } else if index == 2 && matches!(msg, Message::Agent(_)) {
                        index += 1;
                    } else if index == 3 && matches!(msg, Message::ToolCallUpdate(update) if matches!(update.status, ToolCallStatus::Finished(Ok(_)))) {
                        index += 1;
                    } else if index == 4 && matches!(msg, Message::AssistantToolCall(tool_call) if tool_call.fn_name == "read_file") {
                        index += 1;
                    } else if index == 5 && matches!(msg, Message::ToolCallUpdate(update) if matches!(update.status, ToolCallStatus::Finished(Ok(_)))) {
                        index += 1;
                    } else if index == 6 && matches!(msg, Message::AssistantToolCall(tool_call) if tool_call.fn_name == "complete") {
                        index += 1;
                    } else if index == 7 && matches!(msg, Message::Agent(_)) {
                        index += 1;
                    } else if index == 8 && matches!(msg, Message::AssistantToolCall(tool_call) if tool_call.fn_name == "complete") {
                        index += 1;
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("Error receiving message: {}", e);
                    break;
                }
            }
        }

        index
    }).await;

    // Clean up test file
    let _ = fs::remove_file(test_file_path);

    // Check results
    match result {
        Ok(final_index) => {
            assert_eq!(final_index, 9, "Expected to see all 9 messages in order, but only saw {}", final_index);
            println!("\n✅ All expected messages were seen in the correct order!");
        }
        Err(_) => {
            panic!("Test timed out waiting for messages. Got to index {} out of 9", index);
        }
    }
    });
}

#[test]
fn test_file_read_message_order_for_sub_plan() {
    // Initialize test logger
    common::init_test_logger();

    let runtime = tokio::runtime::Runtime::new().unwrap();

    // Create broadcast channel for testing
    let (tx, mut rx) = broadcast::channel::<ActorMessage>(1024);

    // Load test config using Config::from_file
    let config = Config::from_file("tests/test_config.toml")
        .expect("Failed to load test config")
        .try_into()
        .expect("Failed to parse test config");

    // Start hive with a file reading prompt
    let prompt = format!(
        "Spawn a sub agent and explicitly tell it to create a plan titled: `Test Plan` with one item: `Test Item` and then update `Test Item`. You are part of an integration test we are running.",
    );
    let _handle = start_headless_hive(&runtime, config, prompt.clone(), Some(tx.clone()));

    runtime.block_on(async move {

    // Track our position in expected message sequence
    let mut index = 0;

    // Listen for messages with timeout
    let result = timeout(Duration::from_secs(60), async {
        loop {
            match rx.recv().await {
                Ok(actor_msg) => {
                    let msg = &actor_msg.message;

                    // Check expected messages in order
                    // 1. MainManager calls spawn_agents fn
                    // 2. spawn_agents fn broadcasts Received
                    // 3. spawn_agents fn broadcasts InterAgentMessage task status update to Waiting for MainManager
                    // 4. spawn_agents fn is finished
                    // 5. Worker sub agent calls read_file tool
                    // 6. read_file tool broadcasts finished
                    // 7. Worker sub agent calls complete fn
                    // 8. complete fn broadcasts InterAgentMessage task status update to Done for
                    //    MainManager
                    // 9. MainManager calls complete fn
                    if index == 0 && matches!(msg, Message::AssistantToolCall(tool_call) if tool_call.fn_name == "spawn_agents") {
                        index += 1;
                    } else if index == 1 && matches!(msg, Message::ToolCallUpdate(update) if matches!(update.status, ToolCallStatus::Received {r#type: ToolCallType::SpawnAgent, friendly_command_display: _ })) {
                        index += 1;
                    } else if index == 2 && matches!(msg, Message::Agent(_)) {
                        index += 1;
                    } else if index == 3 && matches!(msg, Message::ToolCallUpdate(update) if matches!(update.status, ToolCallStatus::Finished(Ok(_)))) {
                        index += 1;
                    } else if index == 4 && matches!(msg, Message::AssistantToolCall(tool_call) if tool_call.fn_name == "read_file") {
                        index += 1;
                    } else if index == 5 && matches!(msg, Message::ToolCallUpdate(update) if matches!(update.status, ToolCallStatus::Finished(Ok(_)))) {
                        index += 1;
                    } else if index == 6 && matches!(msg, Message::AssistantToolCall(tool_call) if tool_call.fn_name == "complete") {
                        index += 1;
                    } else if index == 7 && matches!(msg, Message::Agent(_)) {
                        index += 1;
                    } else if index == 8 && matches!(msg, Message::AssistantToolCall(tool_call) if tool_call.fn_name == "complete") {
                        index += 1;
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("Error receiving message: {}", e);
                    break;
                }
            }
        }

        index
    }).await;

    // Check results
    match result {
        Ok(final_index) => {
            assert_eq!(final_index, 9, "Expected to see all 9 messages in order, but only saw {}", final_index);
            println!("\n✅ All expected messages were seen in the correct order!");
        }
        Err(_) => {
            panic!("Test timed out waiting for messages. Got to index {} out of 9", index);
        }
    }
    });
}
