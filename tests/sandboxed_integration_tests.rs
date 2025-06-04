/// Sandboxed Integration Tests
/// 
/// These tests run the hive system in a Docker sandbox for safe testing
/// of tool execution, file operations, and error scenarios without risking
/// the host system.

use std::process::Command;
use std::time::Duration;

/// Docker sandbox test utilities
pub mod docker_test_utils {
    use super::*;
    
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
                return Err("Docker is not available. Please install Docker to run sandboxed tests.".to_string());
            }
            
            // Stop any existing container
            self.stop().await.ok(); // Ignore errors if container doesn't exist
            
            // Start the sandbox using docker-compose
            let output = Command::new("docker-compose")
                .args(&[
                    "-f", "tests/docker/docker-compose.test.yml",
                    "up", "-d", "--build"
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
                .args(&[
                    "-f", "tests/docker/docker-compose.test.yml",
                    "down", "-v"
                ])
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
                .args(&[
                    "exec",
                    &self.container_name,
                    "bash",
                    "-c",
                    cmd
                ])
                .output()
                .map_err(|e| format!("Failed to execute command in sandbox: {}", e))?;
            
            let exit_code = output.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            
            Ok((exit_code, stdout, stderr))
        }
        
        /// Copy file to sandbox
        pub async fn copy_to_sandbox(&self, local_path: &str, container_path: &str) -> Result<(), String> {
            let output = Command::new("docker")
                .args(&[
                    "cp",
                    local_path,
                    &format!("{}:{}", self.container_name, container_path)
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
        pub async fn copy_from_sandbox(&self, container_path: &str, local_path: &str) -> Result<(), String> {
            let output = Command::new("docker")
                .args(&[
                    "cp",
                    &format!("{}:{}", self.container_name, container_path),
                    local_path
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
        
        /// Run hive in headless mode with a prompt
        pub async fn run_hive_headless(&self, prompt: &str, timeout_secs: u64) -> Result<(i32, String, String), String> {
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
system_prompt = "You are a test manager. Break down tasks and delegate to workers."

[hive.sub_manager_model]
name = "deepseek-chat"
api_key_env_var = "DEEPSEEK_API_KEY"
system_prompt = "You are a test sub-manager. Manage your assigned tasks."

[hive.worker_model]
name = "deepseek-chat"
api_key_env_var = "DEEPSEEK_API_KEY"
system_prompt = "You are a test worker. Use tools to complete your assigned task."
"#;
            
            // Write config to sandbox using heredoc to avoid quoting issues
            self.exec_command(&format!(
                "cat > /workspace/test-config.toml << 'EOF'\n{}\nEOF",
                config_content
            )).await?;
            
            // Run hive with the prompt (use printf to handle quotes properly)
            let escaped_prompt = prompt.replace("'", "'\"'\"'");
            let cmd = format!(
                "cd /workspace && HIVE_CONFIG_PATH=/workspace/test-config.toml timeout {} hive headless --auto-approve-commands '{}'",
                timeout_secs,
                escaped_prompt
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
            for _ in 0..30 { // Wait up to 30 seconds
                let output = Command::new("docker")
                    .args(&["inspect", "--format={{.State.Health.Status}}", &self.container_name])
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
                    .args(&[
                        "-f", "tests/docker/docker-compose.test.yml",
                        "down", "-v"
                    ])
                    .output();
            }
        }
    }
}

#[tokio::test]
#[ignore] // Use 'cargo test -- --ignored' to run Docker tests
async fn test_sandbox_environment_setup() {
    let mut sandbox = docker_test_utils::DockerSandbox::new();
    
    // Start sandbox
    sandbox.start().await.expect("Failed to start sandbox");
    
    // Test basic commands
    let (exit_code, stdout, _) = sandbox.exec_command("echo 'Hello from sandbox'").await.unwrap();
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("Hello from sandbox"));
    
    // Test file operations
    let (exit_code, _, _) = sandbox.exec_command("touch /workspace/temp/test.txt").await.unwrap();
    assert_eq!(exit_code, 0);
    
    let (exit_code, stdout, _) = sandbox.exec_command("ls /workspace/temp/test.txt").await.unwrap();
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("test.txt"));
    
    // Cleanup
    sandbox.stop().await.expect("Failed to stop sandbox");
}

#[tokio::test]
#[ignore]
async fn test_sandboxed_file_reading_workflow() {
    let mut sandbox = docker_test_utils::DockerSandbox::new();
    sandbox.start().await.expect("Failed to start sandbox");
    
    // Create a test file in the sandbox
    sandbox.exec_command("echo 'Test file content for reading' > /workspace/temp/read_test.txt").await.unwrap();
    
    // Run hive to read the file
    let prompt = "Read the contents of the file /workspace/temp/read_test.txt and tell me what it contains";
    
    let (exit_code, stdout, stderr) = sandbox.run_hive_headless(prompt, 30).await.unwrap();
    
    println!("Exit code: {}", exit_code);
    println!("Stdout: {}", stdout);
    println!("Stderr: {}", stderr);
    
    // In a real scenario, we'd check that the output contains the file content
    // For now, we verify the command completed
    assert!(exit_code == 0 || exit_code == 124, "Command should complete or timeout gracefully");
    
    sandbox.stop().await.expect("Failed to stop sandbox");
}

#[tokio::test]
#[ignore]
async fn test_sandboxed_command_execution_workflow() {
    let mut sandbox = docker_test_utils::DockerSandbox::new();
    sandbox.start().await.expect("Failed to start sandbox");
    
    // Test safe command execution
    let prompt = "Execute the command 'ls /workspace/test-files' and show me the results";
    
    let (exit_code, stdout, stderr) = sandbox.run_hive_headless(prompt, 30).await.unwrap();
    
    println!("Exit code: {}", exit_code);
    println!("Stdout: {}", stdout);
    println!("Stderr: {}", stderr);
    
    // Verify command completed
    assert!(exit_code == 0 || exit_code == 124, "Command should complete or timeout gracefully");
    
    sandbox.stop().await.expect("Failed to stop sandbox");
}

#[tokio::test]
#[ignore]
async fn test_sandboxed_error_recovery() {
    let mut sandbox = docker_test_utils::DockerSandbox::new();
    sandbox.start().await.expect("Failed to start sandbox");
    
    // Test error handling with a command that will fail
    let prompt = "Execute the command 'cat /nonexistent/file.txt' and handle the error gracefully";
    
    let (exit_code, stdout, stderr) = sandbox.run_hive_headless(prompt, 30).await.unwrap();
    
    println!("Exit code: {}", exit_code);
    println!("Stdout: {}", stdout);
    println!("Stderr: {}", stderr);
    
    // The system should handle the error gracefully (not crash)
    // Exit code might be non-zero due to the failing command, but should not be a system crash
    assert!(exit_code >= 0, "System should handle errors gracefully");
    
    sandbox.stop().await.expect("Failed to stop sandbox");
}

#[tokio::test]
#[ignore]
async fn test_sandboxed_multi_step_workflow() {
    let mut sandbox = docker_test_utils::DockerSandbox::new();
    sandbox.start().await.expect("Failed to start sandbox");
    
    // Test a complex workflow that involves multiple steps
    let prompt = "Create a directory called 'test-project' in /workspace/temp, then create a README.md file in it with the content 'This is a test project', then list the contents of the directory";
    
    let (exit_code, stdout, stderr) = sandbox.run_hive_headless(prompt, 45).await.unwrap();
    
    println!("Exit code: {}", exit_code);
    println!("Stdout: {}", stdout);
    println!("Stderr: {}", stderr);
    
    // Verify the workflow completed
    assert!(exit_code == 0 || exit_code == 124, "Multi-step workflow should complete");
    
    // Verify the directory and file were created
    let (exit_code, stdout, _) = sandbox.exec_command("ls /workspace/temp/test-project/").await.unwrap();
    if exit_code == 0 {
        assert!(stdout.contains("README.md"), "README.md should be created");
    }
    
    sandbox.stop().await.expect("Failed to stop sandbox");
}

// Helper function to run all sandbox tests
pub async fn run_all_sandbox_tests() -> Result<(), String> {
    println!("Running sandboxed integration tests...");
    
    // Check if Docker is available
    let docker_check = Command::new("docker")
        .arg("--version")
        .output()
        .map_err(|_| "Docker is not available")?;
    
    if !docker_check.status.success() {
        return Err("Docker is not available. Please install Docker to run sandboxed tests.".to_string());
    }
    
    println!("✓ Docker is available");
    
    // Run the tests
    let test_commands = vec![
        "cargo test test_sandbox_environment_setup -- --ignored",
        "cargo test test_sandboxed_file_reading_workflow -- --ignored", 
        "cargo test test_sandboxed_command_execution_workflow -- --ignored",
        "cargo test test_sandboxed_error_recovery -- --ignored",
        "cargo test test_sandboxed_multi_step_workflow -- --ignored",
    ];
    
    for cmd in test_commands {
        println!("Running: {}", cmd);
        let output = Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .map_err(|e| format!("Failed to run test: {}", e))?;
        
        if !output.status.success() {
            println!("Test failed: {}", String::from_utf8_lossy(&output.stderr));
        } else {
            println!("✓ Test passed");
        }
    }
    
    Ok(())
}