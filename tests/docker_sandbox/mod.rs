/// Docker sandbox test utilities
use std::process::Command;
use std::time::Duration;

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
system_prompt = "You are a Main Manager. You MUST delegate all file reading, command execution, and file editing tasks to Worker agents using the spawn_agent_and_assign_task tool. Never try to do these tasks yourself."

[hive.sub_manager_model]
name = "deepseek-chat"
api_key_env_var = "DEEPSEEK_API_KEY"
system_prompt = "You are a Sub-Manager. You MUST delegate all file reading, command execution, and file editing tasks to Worker agents using the spawn_agent_and_assign_task tool."

[hive.worker_model]
name = "deepseek-chat"
api_key_env_var = "DEEPSEEK_API_KEY"
system_prompt = "You are a Worker agent. Use your available tools (file_reader, command, edit_file) to complete the specific task assigned to you. Always use the appropriate tool for the task."
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

    /// Verify log output contains expected execution patterns
    pub fn verify_log_execution(
        &self,
        stdout: &str,
        expected_tools: &[&str],
    ) -> Result<bool, String> {
        // Extract log content from stdout (after the marker)
        let log_marker = "=== LOG OUTPUT ===";
        let log_content = if let Some(pos) = stdout.find(log_marker) {
            &stdout[pos + log_marker.len()..]
        } else {
            return Err("Log output marker not found in stdout".to_string());
        };

        // Check for basic execution patterns
        let mut verification_results = Vec::new();

        // Check for HIVE startup (now in spans)
        if log_content.contains("start_headless_hive") {
            verification_results.push("✅ HIVE system started".to_string());
        } else {
            verification_results.push("❌ HIVE system startup not found".to_string());
        }

        // Check for agent creation (now in spans)
        if log_content.contains("agent_run") {
            verification_results.push("✅ Agent started".to_string());
        } else {
            verification_results.push("❌ Agent startup not found".to_string());
        }

        // Check for actor readiness
        let ready_count = log_content
            .matches("Actor ready, sending ready signal")
            .count();
        if ready_count >= 4 {
            verification_results.push(format!("✅ {} actors ready", ready_count));
        } else {
            verification_results.push(format!(
                "❌ Only {} actors ready, expected >= 4",
                ready_count
            ));
        }

        // Check for LLM interaction (now in spans)
        if log_content.contains("llm_request") {
            verification_results.push("✅ LLM request executed".to_string());
        } else {
            verification_results.push("❌ No LLM requests found".to_string());
        }

        // Check for Manager delegation
        if log_content.contains("spawn_agent_and_assign_task") {
            verification_results.push("✅ Manager delegated tasks to workers".to_string());
        } else {
            verification_results.push("❌ No task delegation found".to_string());
        }

        // Check for Worker agent spawning
        let worker_count = log_content.matches("Worker").count();
        if worker_count > 0 {
            verification_results
                .push(format!("✅ {} Worker agent references found", worker_count));
        } else {
            verification_results.push("❌ No Worker agents found".to_string());
        }

        // Check for tool call patterns (more specific)
        if log_content.contains("_tool_call") {
            verification_results.push("✅ Tool calls executed".to_string());
        } else {
            verification_results.push("⚠️  No tool call patterns found".to_string());
        }

        // Check for command execution patterns
        if log_content.contains("execute_command") || log_content.contains("command_tool_call")
        {
            verification_results.push("✅ Command execution found".to_string());
        } else {
            verification_results.push("⚠️  No command execution found".to_string());
        }

        // Check for file operations
        if log_content.contains("file_reader") || log_content.contains("read_file") {
            verification_results.push("✅ File reading operations found".to_string());
        } else {
            verification_results.push("⚠️  No file reading operations found".to_string());
        }

        // Check for expected tools (legacy support)
        for tool in expected_tools {
            if log_content.contains(tool) {
                verification_results.push(format!("✅ Tool '{}' found in logs", tool));
            } else {
                verification_results.push(format!("⚠️  Tool '{}' not found in logs", tool));
            }
        }

        // Print verification results
        println!("Log Verification Results:");
        for result in &verification_results {
            println!("  {}", result);
        }

        // Return success if no critical errors (❌) found
        let has_critical_errors = verification_results.iter().any(|r| r.contains("❌"));
        Ok(!has_critical_errors)
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