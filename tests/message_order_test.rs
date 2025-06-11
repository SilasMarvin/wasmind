mod common;

use hive::actors::{ActorMessage, Message, Action, UserContext};
use hive::config::Config;
use hive::hive::start_headless_hive;
use tokio::sync::broadcast;
use tokio::time::{timeout, Duration};
use std::fs;

#[tokio::test]
async fn test_file_read_message_order() {
    // Initialize test logger
    common::init_test_logger();
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
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let prompt = format!("What are the contents of the file {}?", test_file_path);
    let _handle = start_headless_hive(
        &runtime,
        config,
        prompt.clone(),
        Some(tx.clone()),
    );
    
    // Track our position in expected message sequence
    let mut index = 0;
    
    // Listen for messages with timeout
    let result = timeout(Duration::from_secs(30), async {
        loop {
            match rx.recv().await {
                Ok(actor_msg) => {
                    let msg = &actor_msg.message;
                    println!("Received message at index {}: {:?}", index, msg);
                    
                    // Check expected messages in order
                    if index == 0 && matches!(msg, Message::Action(Action::Assist)) {
                        println!("✓ [{}] Saw Assist action", index);
                        index += 1;
                    } else if index == 1 && matches!(msg, Message::UserContext(UserContext::UserTUIInput(input)) if input == &prompt) {
                        println!("✓ [{}] Saw correct user input", index);
                        index += 1;
                    } else if index == 2 && matches!(msg, Message::Agent(_)) {
                        println!("✓ [{}] Saw agent message", index);
                        index += 1;
                    } else if index == 3 && matches!(msg, Message::AssistantToolCall(tool_call) if tool_call.fn_name == "spawn_agent_and_assign_task") {
                        println!("✓ [{}] Saw spawn_agent_and_assign_task tool call", index);
                        index += 1;
                    } else if index == 4 && matches!(msg, Message::AssistantToolCall(tool_call) if tool_call.fn_name == "file_reader") {
                        println!("✓ [{}] Saw file_reader tool call", index);
                        index += 1;
                    } else if index == 5 && matches!(msg, Message::AssistantToolCall(tool_call) if tool_call.fn_name == "complete") {
                        println!("✓ [{}] Saw complete tool call from worker", index);
                        index += 1;
                    } 
                    // If message doesn't match expected at current index, we just continue
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
            assert_eq!(final_index, 7, "Expected to see all 7 messages in order, but only saw {}", final_index);
            println!("\n✅ All expected messages were seen in the correct order!");
        }
        Err(_) => {
            panic!("Test timed out waiting for messages. Got to index {} out of 7", index);
        }
    }
}
