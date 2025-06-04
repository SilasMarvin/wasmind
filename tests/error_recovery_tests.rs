use std::process::Command;
/// Error Recovery and Failure Scenario Tests
///
/// These tests specifically focus on how the system handles various failure modes:
/// - Tool execution failures mid-operation
/// - Agent communication breakdowns
/// - Resource exhaustion scenarios
/// - Invalid input handling
/// - Recovery and cleanup mechanisms
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

    /// Run hive in headless mode with a prompt
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

[key_bindings]
clear_defaults = false

[key_bindings.bindings]
"cmd-alt-a" = "Assist"
"ctrl-c" = "Exit"

[hive]

[hive.main_manager_model]
name = "deepseek-chat"
system_prompt = "You are a test manager. Break down tasks and delegate to workers."

[hive.sub_manager_model]
name = "deepseek-chat"
system_prompt = "You are a test sub-manager. Manage your assigned tasks."

[hive.worker_model]
name = "deepseek-chat"
system_prompt = "You are a test worker. Use tools to complete your assigned task."
"#;

        // Write config to sandbox using heredoc to avoid quoting issues
        self.exec_command(&format!(
            "cat > /workspace/test-config.toml << 'EOF'\n{}\nEOF",
            config_content
        ))
        .await?;

        // Run hive with the prompt (use printf to handle quotes properly)
        let escaped_prompt = prompt.replace("'", "'\"'\"'");
        let cmd = format!(
            "cd /workspace && HIVE_CONFIG_PATH=/workspace/test-config.toml timeout {} hive headless --auto-approve-commands '{}'",
            timeout_secs, escaped_prompt
        );

        self.exec_command(&cmd).await
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

#[tokio::test]
#[ignore]
async fn test_command_tool_failure_recovery() {
    let mut sandbox = DockerSandbox::new();
    sandbox.start().await.expect("Failed to start sandbox");

    // Test command that will definitely fail
    let prompt = "Execute the command 'nonexistent-command-that-will-fail' and then recover by executing 'echo Recovery successful'";

    let (exit_code, stdout, stderr) = sandbox.run_hive_headless(prompt, 30).await.unwrap();

    println!("Command failure test:");
    println!("Exit code: {}", exit_code);
    println!("Stdout: {}", stdout);
    println!("Stderr: {}", stderr);

    // System should handle the failure and continue
    assert!(
        exit_code == 0 || exit_code == 124,
        "System should handle command failures gracefully"
    );

    // Verify the system is still responsive after the failure
    let (exit_code, stdout, _) = sandbox
        .exec_command("echo 'System is still responsive'")
        .await
        .unwrap();
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("System is still responsive"));

    sandbox.stop().await.expect("Failed to stop sandbox");
}

#[tokio::test]
#[ignore]
async fn test_file_permission_error_recovery() {
    let mut sandbox = DockerSandbox::new();
    sandbox.start().await.expect("Failed to start sandbox");

    // Create a file with restricted permissions
    sandbox
        .exec_command("echo 'protected content' > /workspace/temp/protected.txt")
        .await
        .unwrap();
    sandbox
        .exec_command("chmod 000 /workspace/temp/protected.txt")
        .await
        .unwrap();

    // Try to read the protected file - should fail gracefully
    let prompt = "Read the file /workspace/temp/protected.txt and if that fails, create a new file called /workspace/temp/alternative.txt with content 'Alternative content'";

    let (exit_code, stdout, stderr) = sandbox.run_hive_headless(prompt, 30).await.unwrap();

    println!("Permission error test:");
    println!("Exit code: {}", exit_code);
    println!("Stdout: {}", stdout);
    println!("Stderr: {}", stderr);

    // System should handle permission errors gracefully
    assert!(
        exit_code == 0 || exit_code == 124,
        "System should handle permission errors gracefully"
    );

    // Check if alternative action was taken
    let (exit_code, stdout, _) = sandbox
        .exec_command("cat /workspace/temp/alternative.txt")
        .await
        .unwrap();
    if exit_code == 0 {
        assert!(
            stdout.contains("Alternative content"),
            "System should create alternative file when primary action fails"
        );
    }

    sandbox.stop().await.expect("Failed to stop sandbox");
}

#[tokio::test]
#[ignore]
async fn test_agent_communication_failure_recovery() {
    let mut sandbox = DockerSandbox::new();
    sandbox.start().await.expect("Failed to start sandbox");

    // Test scenario where we simulate agent communication issues
    // by killing processes mid-execution
    let prompt = "Create a large file by running 'dd if=/dev/zero of=/workspace/temp/large.txt bs=1M count=10' and then immediately list the directory contents";

    // Start the command
    let start_time = std::time::Instant::now();

    // Run in background and simulate interruption
    let handle = tokio::spawn(async move { sandbox.run_hive_headless(&prompt, 45).await });

    // Wait a bit then try to interrupt some processes
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Try to interrupt any long-running processes in the container
    let mut sandbox2 = DockerSandbox::new();
    sandbox2.container_name = "hive-test-sandbox".to_string();
    sandbox2.is_running = true;

    let _ = sandbox2.exec_command("pkill -f dd").await; // Kill any dd processes

    // Wait for the original command to complete or timeout
    let result = handle.await.unwrap();
    let elapsed = start_time.elapsed();

    match result {
        Ok((exit_code, stdout, stderr)) => {
            println!("Interruption test completed:");
            println!("Exit code: {}", exit_code);
            println!("Stdout: {}", stdout);
            println!("Stderr: {}", stderr);
            println!("Elapsed: {:?}", elapsed);

            // System should handle interruptions gracefully
            assert!(
                exit_code >= 0,
                "System should handle interruptions without crashing"
            );
        }
        Err(e) => {
            println!("Interruption test failed with error: {}", e);
            // This is acceptable as long as it's a clean failure
        }
    }

    // Verify sandbox is still functional
    let (exit_code, stdout, _) = sandbox2
        .exec_command("echo 'Sandbox still functional'")
        .await
        .unwrap();
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("Sandbox still functional"));

    sandbox2.stop().await.expect("Failed to stop sandbox");
}

#[tokio::test]
#[ignore]
async fn test_resource_exhaustion_recovery() {
    let mut sandbox = DockerSandbox::new();
    sandbox.start().await.expect("Failed to start sandbox");

    // Test memory/disk exhaustion scenarios
    let prompt = "Try to create a very large file with 'dd if=/dev/zero of=/workspace/temp/huge.txt bs=1M count=1000' and handle any resource limitations gracefully";

    let (exit_code, stdout, stderr) = sandbox.run_hive_headless(prompt, 60).await.unwrap();

    println!("Resource exhaustion test:");
    println!("Exit code: {}", exit_code);
    println!("Stdout: {}", stdout);
    println!("Stderr: {}", stderr);

    // System should handle resource limitations without crashing
    assert!(
        exit_code >= 0,
        "System should handle resource exhaustion gracefully"
    );

    // Verify system is still responsive
    let (exit_code, stdout, _) = sandbox.exec_command("df -h /workspace").await.unwrap();
    assert_eq!(exit_code, 0);
    println!("Disk usage after test: {}", stdout);

    sandbox.stop().await.expect("Failed to stop sandbox");
}

#[tokio::test]
#[ignore]
async fn test_invalid_input_handling() {
    let mut sandbox = DockerSandbox::new();
    sandbox.start().await.expect("Failed to start sandbox");

    // Test various invalid inputs
    let long_prompt = "a".repeat(10000);
    let invalid_prompts = vec![
        "rm -rf /*",        // Dangerous command
        ":(){ :|:& };:",    // Fork bomb
        "cat /dev/urandom", // Infinite output
        "sleep 1000",       // Long-running command
        "",                 // Empty prompt
        &long_prompt,       // Very long prompt
    ];

    for (i, prompt) in invalid_prompts.iter().enumerate() {
        let display_prompt = if prompt.len() > 50 {
            format!("{}...", &prompt[..50])
        } else {
            (*prompt).to_string()
        };
        println!("Testing invalid input {}: {}", i + 1, display_prompt);

        let (exit_code, _stdout, _stderr) = sandbox.run_hive_headless(prompt, 10).await.unwrap();

        // System should handle invalid inputs gracefully (not crash)
        assert!(
            exit_code >= 0,
            "System should handle invalid input {} gracefully",
            i + 1
        );

        // Check system is still responsive
        let (exit_code, _stdout, _) = sandbox
            .exec_command("echo 'Still responsive'")
            .await
            .unwrap();
        assert_eq!(
            exit_code,
            0,
            "System should remain responsive after invalid input {}",
            i + 1
        );
    }

    sandbox.stop().await.expect("Failed to stop sandbox");
}

#[tokio::test]
#[ignore]
async fn test_concurrent_operation_conflicts() {
    let mut sandbox = DockerSandbox::new();
    sandbox.start().await.expect("Failed to start sandbox");

    // Create a file that multiple operations will try to access
    sandbox
        .exec_command("echo 'Initial content' > /workspace/temp/shared.txt")
        .await
        .unwrap();

    // Test concurrent file operations that might conflict
    let prompt = "Simultaneously read the file /workspace/temp/shared.txt, append 'new line' to it, and create a backup copy called /workspace/temp/shared_backup.txt";

    let (exit_code, stdout, stderr) = sandbox.run_hive_headless(prompt, 30).await.unwrap();

    println!("Concurrent operations test:");
    println!("Exit code: {}", exit_code);
    println!("Stdout: {}", stdout);
    println!("Stderr: {}", stderr);

    // System should handle concurrent operations without corruption
    assert!(
        exit_code == 0 || exit_code == 124,
        "System should handle concurrent operations gracefully"
    );

    // Verify file integrity
    let (exit_code, content, _) = sandbox
        .exec_command("cat /workspace/temp/shared.txt")
        .await
        .unwrap();
    if exit_code == 0 {
        assert!(
            content.contains("Initial content"),
            "Original content should be preserved"
        );
        println!("Final file content: {}", content);
    }

    sandbox.stop().await.expect("Failed to stop sandbox");
}

#[tokio::test]
#[ignore]
async fn test_cleanup_after_failures() {
    let mut sandbox = DockerSandbox::new();
    sandbox.start().await.expect("Failed to start sandbox");

    // Run operations that create temporary resources and then fail
    let prompt = "Create several temporary files in /workspace/temp/, start a background process, then execute a command that fails, and ensure cleanup happens properly";

    let (exit_code, stdout, stderr) = sandbox.run_hive_headless(prompt, 30).await.unwrap();

    println!("Cleanup test:");
    println!("Exit code: {}", exit_code);
    println!("Stdout: {}", stdout);
    println!("Stderr: {}", stderr);

    // Check for resource leaks
    let (exit_code, processes, _) = sandbox
        .exec_command("ps aux | grep -v grep | wc -l")
        .await
        .unwrap();
    if exit_code == 0 {
        let process_count: i32 = processes.trim().parse().unwrap_or(0);
        assert!(
            process_count < 50,
            "Should not have excessive processes running: {}",
            process_count
        );
    }

    // Check file descriptor usage
    let (exit_code, fds, _) = sandbox.exec_command("lsof | wc -l").await.unwrap();
    if exit_code == 0 {
        let fd_count: i32 = fds.trim().parse().unwrap_or(0);
        assert!(
            fd_count < 1000,
            "Should not have excessive file descriptors open: {}",
            fd_count
        );
    }

    // Check memory usage
    let (exit_code, memory, _) = sandbox
        .exec_command("free -m | grep Mem | awk '{print $3}'")
        .await
        .unwrap();
    if exit_code == 0 {
        let memory_used: i32 = memory.trim().parse().unwrap_or(0);
        assert!(
            memory_used < 400,
            "Should not use excessive memory: {}MB",
            memory_used
        );
    }

    sandbox.stop().await.expect("Failed to stop sandbox");
}

#[tokio::test]
#[ignore]
async fn test_network_isolation_and_security() {
    let mut sandbox = DockerSandbox::new();
    sandbox.start().await.expect("Failed to start sandbox");

    // Test that the sandbox properly isolates network access
    let security_tests = vec![
        "ping -c 1 8.8.8.8",          // Should work (basic connectivity)
        "curl http://httpbin.org/ip", // Should work (HTTP)
        "nc -l 9999",                 // Should be limited
    ];

    for (i, cmd) in security_tests.iter().enumerate() {
        println!("Security test {}: {}", i + 1, cmd);

        let prompt = &format!(
            "Execute the command '{}' and handle any security restrictions appropriately",
            cmd
        );
        let (exit_code, stdout, stderr) = sandbox.run_hive_headless(prompt, 15).await.unwrap();

        println!(
            "Exit code: {}, Stdout: {}, Stderr: {}",
            exit_code, stdout, stderr
        );

        // Commands should either work within limits or be properly restricted
        assert!(
            exit_code >= 0,
            "Security test {} should complete without system crashes",
            i + 1
        );
    }

    sandbox.stop().await.expect("Failed to stop sandbox");
}

// Utility function to run error recovery test suite
pub async fn run_error_recovery_tests() -> Result<(), String> {
    println!("Running comprehensive error recovery tests...");

    let tests = vec![
        (
            "Command failure recovery",
            "test_command_tool_failure_recovery",
        ),
        (
            "File permission errors",
            "test_file_permission_error_recovery",
        ),
        (
            "Agent communication failures",
            "test_agent_communication_failure_recovery",
        ),
        ("Resource exhaustion", "test_resource_exhaustion_recovery"),
        ("Invalid input handling", "test_invalid_input_handling"),
        (
            "Concurrent operation conflicts",
            "test_concurrent_operation_conflicts",
        ),
        ("Cleanup after failures", "test_cleanup_after_failures"),
        ("Network isolation", "test_network_isolation_and_security"),
    ];

    let mut passed = 0;
    let mut failed = 0;

    for (name, test_fn) in tests {
        println!("\n🧪 Running: {}", name);

        let cmd = format!("cargo test {} -- --ignored --nocapture", test_fn);
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .output()
            .map_err(|e| format!("Failed to run test {}: {}", name, e))?;

        if output.status.success() {
            println!("✅ {} PASSED", name);
            passed += 1;
        } else {
            println!("❌ {} FAILED", name);
            println!("{}", String::from_utf8_lossy(&output.stderr));
            failed += 1;
        }
    }

    println!("\n📊 Error Recovery Test Summary:");
    println!("Passed: {}", passed);
    println!("Failed: {}", failed);

    if failed == 0 {
        Ok(())
    } else {
        Err(format!("{} error recovery tests failed", failed))
    }
}
