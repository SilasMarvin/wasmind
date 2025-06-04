/// Docker sandbox test utilities
use std::process::Command;
use std::time::Duration;
use std::collections::HashMap;

use crate::log_parser::{LogParser, LogLevel, LogStats, LogEntry};

/// Result of structured log verification
#[derive(Debug)]
pub struct LogVerificationResult {
    // System lifecycle
    pub hive_startup: bool,
    pub agent_started: bool,
    pub actors_ready_count: usize,
    
    // LLM and task management
    pub llm_requests: bool,
    pub task_delegation: bool,
    pub worker_agents_count: usize,
    
    // Tool execution
    pub tool_calls_executed: bool,
    pub command_execution: bool,
    pub file_operations: bool,
    
    // Task completion
    pub complete_tool_called: bool,
    pub task_completion_signaled: bool,
    pub task_completed_messages: bool,
    pub proper_completion_sequence: bool,
    
    // Expected tools verification
    pub expected_tools: HashMap<String, bool>,
    
    // Error tracking
    pub error_count: usize,
    pub errors: Vec<String>,
    
    // Statistics
    pub log_stats: LogStats,
}

impl LogVerificationResult {
    pub fn new() -> Self {
        Self {
            hive_startup: false,
            agent_started: false,
            actors_ready_count: 0,
            llm_requests: false,
            task_delegation: false,
            worker_agents_count: 0,
            tool_calls_executed: false,
            command_execution: false,
            file_operations: false,
            complete_tool_called: false,
            task_completion_signaled: false,
            task_completed_messages: false,
            proper_completion_sequence: false,
            expected_tools: HashMap::new(),
            error_count: 0,
            errors: Vec::new(),
            log_stats: LogStats::new(),
        }
    }

    /// Check if verification passes with minimal requirements
    pub fn is_successful(&self) -> bool {
        self.hive_startup && 
        self.agent_started && 
        self.actors_ready_count >= 4 &&
        self.error_count == 0
    }

    /// Check if verification passes with strict requirements including completion
    pub fn is_successful_with_completion(&self) -> bool {
        self.is_successful() && 
        (self.complete_tool_called || self.task_completion_signaled || self.task_completed_messages)
    }

    /// Print detailed verification results
    pub fn print_results(&self) {
        println!("🔍 Structured Log Verification Results:");
        println!("========================================");
        
        // System lifecycle
        println!("📋 System Lifecycle:");
        println!("  {} HIVE system startup", if self.hive_startup { "✅" } else { "❌" });
        println!("  {} Agent started", if self.agent_started { "✅" } else { "❌" });
        println!("  {} {} actors ready (expected >= 4)", 
                if self.actors_ready_count >= 4 { "✅" } else { "❌" }, 
                self.actors_ready_count);
        
        // Task management
        println!("📋 Task Management:");
        println!("  {} LLM requests", if self.llm_requests { "✅" } else { "⚠️ " });
        println!("  {} Task delegation", if self.task_delegation { "✅" } else { "⚠️ " });
        println!("  {} {} Worker agent references", 
                if self.worker_agents_count > 0 { "✅" } else { "⚠️ " }, 
                self.worker_agents_count);
        
        // Tool execution
        println!("📋 Tool Execution:");
        println!("  {} Tool calls executed", if self.tool_calls_executed { "✅" } else { "⚠️ " });
        println!("  {} Command execution", if self.command_execution { "✅" } else { "⚠️ " });
        println!("  {} File operations", if self.file_operations { "✅" } else { "⚠️ " });
        
        // Task completion
        println!("📋 Task Completion:");
        println!("  {} Complete tool called", if self.complete_tool_called { "✅" } else { "⚠️ " });
        println!("  {} Task completion signaled", if self.task_completion_signaled { "✅" } else { "⚠️ " });
        println!("  {} TaskCompleted messages", if self.task_completed_messages { "✅" } else { "⚠️ " });
        println!("  {} Proper completion sequence", if self.proper_completion_sequence { "✅" } else { "⚠️ " });
        
        // Expected tools
        if !self.expected_tools.is_empty() {
            println!("📋 Expected Tools:");
            for (tool, found) in &self.expected_tools {
                println!("  {} Tool '{}'", if *found { "✅" } else { "⚠️ " }, tool);
            }
        }
        
        // Errors
        if self.error_count > 0 {
            println!("📋 Errors ({}):", self.error_count);
            for error in &self.errors {
                println!("  ❌ {}", error);
            }
        } else {
            println!("📋 Errors: ✅ No errors found");
        }
        
        // Statistics
        println!("📋 Log Statistics:");
        println!("  📊 Total entries: {}", self.log_stats.total_entries);
        println!("  📊 Debug: {}, Info: {}, Warn: {}, Error: {}", 
                self.log_stats.debug_count, 
                self.log_stats.info_count, 
                self.log_stats.warn_count, 
                self.log_stats.error_count);
        
        // Overall result
        println!("📋 Overall Result:");
        if self.is_successful_with_completion() {
            println!("  🎉 PASSED (with completion verification)");
        } else if self.is_successful() {
            println!("  ✅ PASSED (basic verification)");
        } else {
            println!("  ❌ FAILED");
        }
    }
}

/// Manages a Docker sandbox for testing
pub struct DockerSandbox {
    pub container_name: String,
    pub is_running: bool,
}

impl DockerSandbox {
    pub fn new() -> Self {
        Self {
            container_name: "hive-test-sandbox".to_string(),
            is_running: false,
        }
    }

    /// Start the Docker sandbox environment
    pub async fn start(&mut self) -> Result<(), String> {
        // Check if Docker is available
        if !self.check_docker_available() {
            return Err(
                "Docker is not available. Please install Docker to run sandboxed tests."
                    .to_string(),
            );
        }

        // Stop any existing container
        self.stop().await.ok(); // Ignore errors if container doesn't exist

        // Start the sandbox using docker-compose
        let output = Command::new("docker-compose")
            .args(&[
                "-f",
                "tests/docker/docker-compose.test.yml",
                "up",
                "-d",
                "--build",
            ])
            .output()
            .map_err(|e| format!("Failed to start Docker container: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "Failed to start Docker container: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        // Wait for container to be healthy
        self.wait_for_health().await?;

        self.is_running = true;
        Ok(())
    }

    /// Stop the Docker sandbox
    pub async fn stop(&mut self) -> Result<(), String> {
        let output = Command::new("docker-compose")
            .args(&["-f", "tests/docker/docker-compose.test.yml", "down", "-v"])
            .output()
            .map_err(|e| format!("Failed to stop Docker container: {}", e))?;

        self.is_running = false;

        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "Failed to stop Docker container: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }

    /// Execute a command in the sandbox
    pub async fn exec_command(&self, cmd: &str) -> Result<(i32, String, String), String> {
        if !self.is_running {
            return Err("Sandbox is not running".to_string());
        }

        let output = Command::new("docker")
            .args(&["exec", &self.container_name, "bash", "-c", cmd])
            .output()
            .map_err(|e| format!("Failed to execute command in sandbox: {}", e))?;

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        Ok((exit_code, stdout, stderr))
    }

    /// Copy file to sandbox
    pub async fn copy_to_sandbox(
        &self,
        local_path: &str,
        container_path: &str,
    ) -> Result<(), String> {
        let output = Command::new("docker")
            .args(&[
                "cp",
                local_path,
                &format!("{}:{}", self.container_name, container_path),
            ])
            .output()
            .map_err(|e| format!("Failed to copy file to sandbox: {}", e))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "Failed to copy file to sandbox: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }

    /// Copy file from sandbox
    pub async fn copy_from_sandbox(
        &self,
        container_path: &str,
        local_path: &str,
    ) -> Result<(), String> {
        let output = Command::new("docker")
            .args(&[
                "cp",
                &format!("{}:{}", self.container_name, container_path),
                local_path,
            ])
            .output()
            .map_err(|e| format!("Failed to copy file from sandbox: {}", e))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "Failed to copy file from sandbox: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }

    /// Run hive in headless mode with a prompt and capture logs
    pub async fn run_hive_headless(
        &self,
        prompt: &str,
        timeout_secs: u64,
    ) -> Result<(i32, String, String), String> {
        // Create a test config in the sandbox
        let config_content = r#"
auto_approve_commands = true

whitelisted_commands = [
    "ls", "pwd", "echo", "cat", "grep", "find", "git", "cargo", "npm", "node", 
    "python", "pip", "which", "date", "whoami", "uname", "head", "tail", "wc", 
    "sort", "uniq", "diff", "tree", "mkdir", "touch", "cp", "mv", "rm"
]

[model]
name = "deepseek-chat"
api_key_env_var = "DEEPSEEK_API_KEY"
system_prompt = "You are a helpful assistant integrated with various tools to help users."

[key_bindings]
clear_defaults = true

[key_bindings.bindings]
"ctrl-a" = "Assist"
"ctrl-c" = "Exit"

[hive]

[hive.main_manager_model]
name = "deepseek-chat"
api_key_env_var = "DEEPSEEK_API_KEY"
system_prompt = "You are a Main Manager. You MUST delegate all file reading, command execution, and file editing tasks to Worker agents using the spawn_agent_and_assign_task tool. Never try to do these tasks yourself. When you have completed the overall task, you MUST call the 'complete' tool to signal completion."

[hive.sub_manager_model]
name = "deepseek-chat"
api_key_env_var = "DEEPSEEK_API_KEY"
system_prompt = "You are a Sub-Manager. You MUST delegate all file reading, command execution, and file editing tasks to Worker agents using the spawn_agent_and_assign_task tool. When you have completed your assigned task, you MUST call the 'complete' tool to signal completion."

[hive.worker_model]
name = "deepseek-chat"
api_key_env_var = "DEEPSEEK_API_KEY"
system_prompt = "You are a Worker agent. Use your available tools (file_reader, command, edit_file, complete) to complete the specific task assigned to you. Always use the appropriate tool for the task. When you have finished your assigned task, you MUST call the 'complete' tool to signal completion."
"#;

        // Write config to sandbox using heredoc to avoid quoting issues
        self.exec_command(&format!(
            "cat > /workspace/test-config.toml << 'EOF'\n{}\nEOF",
            config_content
        ))
        .await?;

        // Run hive with the prompt and capture logs
        let escaped_prompt = prompt.replace("'", "'\"'\"'");
        let cmd = format!(
            "cd /workspace && rm -f log.txt && HIVE_LOG=debug HIVE_CONFIG_PATH=/workspace/test-config.toml timeout {} hive headless --auto-approve-commands '{}' ; echo '=== LOG OUTPUT ===' ; cat log.txt 2>/dev/null || echo 'No log file found'",
            timeout_secs, escaped_prompt
        );

        self.exec_command(&cmd).await
    }

    /// Verify log output using structured log parsing
    pub fn verify_log_execution(
        &self,
        stdout: &str,
        expected_tools: &[&str],
    ) -> Result<LogVerificationResult, String> {
        // Extract log content from stdout (after the marker)
        let log_marker = "=== LOG OUTPUT ===";
        let log_content = if let Some(pos) = stdout.find(log_marker) {
            &stdout[pos + log_marker.len()..]
        } else {
            return Err("Log output marker not found in stdout".to_string());
        };

        // Parse the logs using structured parser
        let parser = LogParser::parse_log_content(log_content)?;
        let mut result = LogVerificationResult::new();

        // 1. Check for HIVE system startup (multiple patterns)
        let hive_startup_spans = parser.entries_by_span("start_headless_hive");
        let hive_startup_messages = parser.entries_with_message("start_headless_hive");
        let hive_startup_target = parser.entries_by_target("start_headless_hive");
        result.hive_startup = !hive_startup_spans.is_empty() || !hive_startup_messages.is_empty() || !hive_startup_target.is_empty();

        // 2. Check for agent creation and lifecycle (multiple patterns)
        let agent_run_spans = parser.entries_by_span("agent_run");
        let agent_run_messages = parser.entries_with_message("agent_run");
        let agent_started_messages = parser.entries_with_message("Agent started");
        let agent_target = parser.entries_by_target("agent_run");
        result.agent_started = !agent_run_spans.is_empty() || !agent_run_messages.is_empty() || !agent_started_messages.is_empty() || !agent_target.is_empty();
        
        // 3. Check for actor readiness
        let ready_entries = parser.entries_with_message("Actor ready, sending ready signal");
        result.actors_ready_count = ready_entries.len();

        // 4. Check for LLM requests (multiple patterns)
        let llm_request_spans = parser.entries_by_span("llm_request");
        let llm_request_messages = parser.entries_with_message("llm_request");
        let llm_execution_messages = parser.entries_with_message("Executing LLM chat request");
        result.llm_requests = !llm_request_spans.is_empty() || !llm_request_messages.is_empty() || !llm_execution_messages.is_empty();

        // 5. Check for task delegation (multiple patterns)
        let spawn_agent_messages = parser.entries_with_message("spawn_agent_and_assign_task");
        let spawn_agent_tool_calls = parser.entries_with_tool_call("spawn_agent_and_assign_task");
        result.task_delegation = !spawn_agent_messages.is_empty() || !spawn_agent_tool_calls.is_empty();

        // 6. Check for Worker agents
        let worker_entries = parser.entries_with_message("Worker");
        result.worker_agents_count = worker_entries.len();

        // 7. Check for tool calls (both actual AssistantToolCall messages and debug patterns)
        let assistant_tool_calls = parser.entries_with_assistant_tool_calls();
        let tool_call_debug_entries = parser.entries_with_message("_tool_call");
        result.tool_calls_executed = !assistant_tool_calls.is_empty() || !tool_call_debug_entries.is_empty();

        // 8. Check for Complete tool usage (comprehensive)
        let complete_tool_calls = parser.entries_with_tool_call("complete");
        let task_completed_messages = parser.entries_with_task_completed();
        let complete_debug_entries = parser.entries_with_message("complete_tool_call");
        
        result.complete_tool_called = !complete_tool_calls.is_empty() || !complete_debug_entries.is_empty();
        result.task_completion_signaled = !parser.entries_with_message("task_completion_signal").is_empty();
        result.task_completed_messages = !task_completed_messages.is_empty();

        // 9. Check for command execution
        let command_entries = parser.entries_with_message("execute_command");
        let command_tool_entries = parser.entries_with_message("command_tool_call");
        result.command_execution = !command_entries.is_empty() || !command_tool_entries.is_empty();

        // 10. Check for file operations
        let file_reader_entries = parser.entries_with_message("file_reader");
        let read_file_entries = parser.entries_with_message("read_file");
        result.file_operations = !file_reader_entries.is_empty() || !read_file_entries.is_empty();

        // 11. Check for expected tools
        for tool in expected_tools {
            let tool_entries = parser.entries_with_message(tool);
            result.expected_tools.insert(tool.to_string(), !tool_entries.is_empty());
        }

        // 12. Check for proper task completion sequence
        let has_delegation_sequence = parser.contains_sequence(&[
            "spawn_agent_and_assign_task",
            "complete_tool_call"
        ]);
        let has_message_sequence = parser.contains_message_sequence(&[
            "AssistantToolCall",
            "TaskCompleted"
        ]);
        result.proper_completion_sequence = has_delegation_sequence || has_message_sequence;

        // 13. Capture any actual error entries (excluding debug messages that mention errors)
        let error_entries = parser.entries_by_level(LogLevel::Error);
        let actual_errors: Vec<&LogEntry> = error_entries.iter()
            .filter(|e| !e.message.contains("DOING CHAT REQUEST WITH"))
            .copied()
            .collect();
        result.error_count = actual_errors.len();
        result.errors = actual_errors.iter().map(|e| e.message.clone()).collect();

        // 14. Get log statistics
        result.log_stats = parser.stats();

        // Print structured verification results
        result.print_results();

        Ok(result)
    }

    fn check_docker_available(&self) -> bool {
        Command::new("docker")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    async fn wait_for_health(&self) -> Result<(), String> {
        for _ in 0..30 {
            // Wait up to 30 seconds
            let output = Command::new("docker")
                .args(&[
                    "inspect",
                    "--format={{.State.Health.Status}}",
                    &self.container_name,
                ])
                .output()
                .map_err(|e| format!("Failed to check container health: {}", e))?;

            if output.status.success() {
                let health_status = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if health_status == "healthy" {
                    return Ok(());
                }
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        Err("Container failed to become healthy within timeout".to_string())
    }
}

impl Drop for DockerSandbox {
    fn drop(&mut self) {
        if self.is_running {
            // Try to stop the container on drop (fire and forget)
            let _ = std::process::Command::new("docker-compose")
                .args(&["-f", "tests/docker/docker-compose.test.yml", "down", "-v"])
                .output();
        }
    }
}