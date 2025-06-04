/// Comprehensive Log Analysis Tests
/// 
/// Tests the structured log parsing and verification capabilities
/// for HIVE integration testing.

mod log_parser;
mod docker_sandbox;

use log_parser::{LogParser, LogLevel, LogEntry};
use docker_sandbox::{DockerSandbox, LogVerificationResult};
use std::collections::HashMap;
use chrono::Utc;

#[test]
fn test_log_parser_standard_format() {
    let log_content = r#"
2024-01-01T12:00:00.000000Z DEBUG hive::agent: Agent started successfully
2024-01-01T12:00:01.000000Z INFO hive::system: HIVE system initialized
2024-01-01T12:00:02.000000Z DEBUG hive::tools::complete: complete_tool_call called
2024-01-01T12:00:03.000000Z INFO hive::agent: TaskCompleted received
"#;

    let parser = LogParser::parse_log_content(log_content).unwrap();
    let entries = parser.entries();
    
    assert_eq!(entries.len(), 4);
    
    // Test specific log level filtering
    let debug_entries = parser.entries_by_level(LogLevel::Debug);
    assert_eq!(debug_entries.len(), 2);
    
    // Test target filtering
    let agent_entries = parser.entries_by_target("agent");
    assert_eq!(agent_entries.len(), 2);
    
    // Test message pattern matching
    let complete_entries = parser.entries_with_message("complete_tool_call");
    assert_eq!(complete_entries.len(), 1);
    
    // Test sequence detection
    assert!(parser.contains_sequence(&["Agent started", "HIVE system", "complete_tool_call"]));
    assert!(!parser.contains_sequence(&["complete_tool_call", "Agent started"]));
}

#[test]
fn test_log_parser_json_format() {
    let log_content = r#"
{"timestamp":"2024-01-01T12:00:00.000000Z","level":"DEBUG","target":"hive::agent","message":"Agent started","fields":{"agent_id":"123","role":"Worker"}}
{"timestamp":"2024-01-01T12:00:01.000000Z","level":"INFO","target":"hive::complete","message":"task_completion_signal","fields":{"success":true,"summary":"Task completed"}}
"#;

    let parser = LogParser::parse_log_content(log_content).unwrap();
    let entries = parser.entries();
    
    assert_eq!(entries.len(), 2);
    
    // Test field filtering
    let agent_field_entries = parser.entries_with_field("agent_id");
    assert_eq!(agent_field_entries.len(), 1);
    
    let success_entries = parser.entries_with_field_value("success", &serde_json::Value::Bool(true));
    assert_eq!(success_entries.len(), 1);
    
    // Test field content
    let first_entry = &entries[0];
    assert_eq!(first_entry.fields.get("agent_id").unwrap().as_str().unwrap(), "123");
    assert_eq!(first_entry.fields.get("role").unwrap().as_str().unwrap(), "Worker");
}

#[test]
fn test_log_verification_result_success_criteria() {
    let mut result = LogVerificationResult::new();
    
    // Test minimal success criteria
    assert!(!result.is_successful()); // Should fail initially
    
    result.hive_startup = true;
    result.agent_started = true;
    result.actors_ready_count = 5;
    result.error_count = 0;
    
    assert!(result.is_successful()); // Should pass basic checks
    assert!(!result.is_successful_with_completion()); // Should fail completion check
    
    result.complete_tool_called = true;
    assert!(result.is_successful_with_completion()); // Should pass with completion
}

#[test]
fn test_task_completion_detection() {
    let log_content = r#"
2024-01-01T12:00:00.000000Z DEBUG hive::agent: Agent started
2024-01-01T12:00:01.000000Z DEBUG hive::tools::complete: complete_tool_call called with summary="File read successfully"
2024-01-01T12:00:02.000000Z DEBUG hive::tools::complete: task_completion_signal sent
2024-01-01T12:00:03.000000Z INFO hive::agent: TaskCompleted message received
"#;

    let parser = LogParser::parse_log_content(log_content).unwrap();
    
    // Test all completion patterns
    assert!(!parser.entries_with_message("complete_tool_call").is_empty());
    assert!(!parser.entries_with_message("task_completion_signal").is_empty());
    assert!(!parser.entries_with_message("TaskCompleted").is_empty());
    
    // Test completion sequence
    assert!(parser.contains_sequence(&["complete_tool_call", "task_completion_signal", "TaskCompleted"]));
}

#[test]
fn test_tool_execution_patterns() {
    let log_content = r#"
2024-01-01T12:00:00.000000Z DEBUG hive::agent: spawn_agent_and_assign_task called
2024-01-01T12:00:01.000000Z DEBUG hive::tools::file_reader: file_reader_tool_call executing
2024-01-01T12:00:02.000000Z DEBUG hive::tools::command: command_tool_call executing
2024-01-01T12:00:03.000000Z DEBUG hive::tools::edit_file: edit_file_tool_call executing
"#;

    let parser = LogParser::parse_log_content(log_content).unwrap();
    
    // Test tool delegation
    assert!(!parser.entries_with_message("spawn_agent_and_assign_task").is_empty());
    
    // Test specific tool calls
    assert!(!parser.entries_with_message("file_reader_tool_call").is_empty());
    assert!(!parser.entries_with_message("command_tool_call").is_empty());
    assert!(!parser.entries_with_message("edit_file_tool_call").is_empty());
    
    // Test tool call pattern
    assert!(!parser.entries_with_message("_tool_call").is_empty());
    
    // Test execution sequence
    assert!(parser.contains_sequence(&[
        "spawn_agent_and_assign_task",
        "file_reader_tool_call",
        "command_tool_call"
    ]));
}

#[test]
fn test_error_detection_and_handling() {
    let log_content = r#"
2024-01-01T12:00:00.000000Z INFO hive::agent: Agent started
2024-01-01T12:00:01.000000Z ERROR hive::tools::command: Command execution failed: file not found
2024-01-01T12:00:02.000000Z WARN hive::agent: Retrying with error recovery
2024-01-01T12:00:03.000000Z DEBUG hive::tools::complete: complete_tool_call with success=false
"#;

    let parser = LogParser::parse_log_content(log_content).unwrap();
    
    // Test error detection
    let error_entries = parser.entries_by_level(LogLevel::Error);
    assert_eq!(error_entries.len(), 1);
    assert!(error_entries[0].message.contains("Command execution failed"));
    
    // Test warning detection
    let warn_entries = parser.entries_by_level(LogLevel::Warn);
    assert_eq!(warn_entries.len(), 1);
    
    // Test that completion still happened despite errors
    assert!(!parser.entries_with_message("complete_tool_call").is_empty());
    
    // Test stats
    let stats = parser.stats();
    assert_eq!(stats.error_count, 1);
    assert_eq!(stats.warn_count, 1);
    assert_eq!(stats.total_entries, 4);
}

#[test]
fn test_worker_agent_tracking() {
    let log_content = r#"
2024-01-01T12:00:00.000000Z INFO hive::agent: Main Manager started
2024-01-01T12:00:01.000000Z DEBUG hive::agent: Spawning Worker agent for task
2024-01-01T12:00:02.000000Z INFO hive::agent: Worker agent started with role=Worker
2024-01-01T12:00:03.000000Z DEBUG hive::agent: Worker executing file_reader tool
2024-01-01T12:00:04.000000Z DEBUG hive::agent: Worker calling complete tool
"#;

    let parser = LogParser::parse_log_content(log_content).unwrap();
    
    // Test worker agent detection
    let worker_entries = parser.entries_with_message("Worker");
    assert!(worker_entries.len() >= 3); // Should find at least 3 mentions of "Worker"
    
    // Test delegation patterns
    assert!(!parser.entries_with_message("Spawning Worker").is_empty());
    assert!(!parser.entries_with_message("Worker executing").is_empty());
    assert!(!parser.entries_with_message("Worker calling complete").is_empty());
}

#[tokio::test]
#[ignore] // Run with: cargo test test_end_to_end_log_analysis -- --ignored
async fn test_end_to_end_log_analysis() {
    let mut sandbox = DockerSandbox::new();
    sandbox.start().await.expect("Failed to start sandbox");

    // Run a simple task that should generate comprehensive logs
    let prompt = "Please read the file /workspace/test-files/config.txt and tell me its contents. Use the complete tool when done.";
    
    let (exit_code, stdout, stderr) = sandbox.run_hive_headless(prompt, 60).await.unwrap();
    
    println!("Exit code: {}", exit_code);
    println!("Stderr: {}", stderr);
    
    // Use the new structured verification
    let verification = sandbox.verify_log_execution(&stdout, &["file_reader", "complete"]).unwrap();
    
    // Detailed assertions with better error messages
    assert!(verification.hive_startup, "HIVE system should start up properly");
    assert!(verification.agent_started, "At least one agent should start");
    assert!(verification.actors_ready_count >= 4, "At least 4 actors should be ready");
    
    // Print comprehensive analysis
    println!("\n🔍 Comprehensive Log Analysis:");
    println!("=====================================");
    println!("System Health: {}", if verification.is_successful() { "✅ HEALTHY" } else { "❌ UNHEALTHY" });
    println!("Completion Status: {}", if verification.is_successful_with_completion() { "✅ COMPLETED" } else { "⚠️  INCOMPLETE" });
    
    if verification.error_count > 0 {
        println!("❌ Errors detected:");
        for error in &verification.errors {
            println!("  - {}", error);
        }
    }
    
    sandbox.stop().await.expect("Failed to stop sandbox");
}

#[test]
fn test_log_stats_calculation() {
    let log_content = r#"
2024-01-01T12:00:00.000000Z TRACE hive::debug: Detailed trace information
2024-01-01T12:00:01.000000Z DEBUG hive::agent: Agent debugging info
2024-01-01T12:00:02.000000Z INFO hive::system: System information
2024-01-01T12:00:03.000000Z WARN hive::agent: Warning message
2024-01-01T12:00:04.000000Z ERROR hive::system: Error occurred
2024-01-01T12:00:05.000000Z DEBUG hive::tools: Another debug message
"#;

    let parser = LogParser::parse_log_content(log_content).unwrap();
    let stats = parser.stats();
    
    assert_eq!(stats.total_entries, 6);
    assert_eq!(stats.trace_count, 1);
    assert_eq!(stats.debug_count, 2);
    assert_eq!(stats.info_count, 1);
    assert_eq!(stats.warn_count, 1);
    assert_eq!(stats.error_count, 1);
    
    // Test target counting
    assert_eq!(stats.targets.get("hive::agent").unwrap(), &2);
    assert_eq!(stats.targets.get("hive::system").unwrap(), &2);
    assert_eq!(stats.targets.get("hive::debug").unwrap(), &1);
    assert_eq!(stats.targets.get("hive::tools").unwrap(), &1);
}