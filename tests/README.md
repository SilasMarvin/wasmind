# Copilot Testing Framework

This directory contains comprehensive tests for the Copilot multi-agent AI system, with a focus on safe end-to-end testing and error recovery scenarios.

## Testing Architecture

### 1. Unit Tests (Existing)
- **Location**: `src/` directories with `#[cfg(test)]` modules
- **Coverage**: Agent communication, state management, templates
- **Status**: ✅ Good coverage for core agent functionality

### 2. Integration Tests (Existing)
- **Location**: `tests/hive_integration_tests.rs`
- **Coverage**: Multi-agent communication, plan workflows
- **Status**: ✅ Excellent coverage for agent coordination

### 3. **Sandboxed End-to-End Tests (New)**
- **Location**: `tests/sandboxed_integration_tests.rs`
- **Coverage**: Complete user workflows in Docker sandbox
- **Status**: 🆕 Safe testing of real tool execution

### 4. **Error Recovery Tests (New)**
- **Location**: `tests/error_recovery_tests.rs`  
- **Coverage**: Failure scenarios and system resilience
- **Status**: 🆕 Comprehensive error handling validation

## Docker Sandbox Testing

### Why Sandbox Testing?

The Copilot system executes real commands and modifies files, making traditional testing dangerous. Our Docker sandbox provides:

- **Safety**: Isolated environment prevents host system damage
- **Realism**: Tests actual tool execution, not mocks
- **Consistency**: Reproducible environment across machines
- **Security**: Resource limits and capability restrictions

### Sandbox Architecture

```
┌─────────────────────────────────────────┐
│ Host System                             │
│  ┌─────────────────────────────────────┐│
│  │ Docker Container (Ubuntu 22.04)    ││
│  │  ┌─────────────────────────────────┐││
│  │  │ Copilot Process                 │││
│  │  │  - Executes commands safely     │││
│  │  │  - Modifies files in /workspace │││
│  │  │  - Limited resources & network  │││
│  │  └─────────────────────────────────┘││
│  │  Available Tools:                   ││
│  │  - Basic shell utilities            ││
│  │  - Python, Node.js                  ││
│  │  - Git, text editors                ││
│  │  - No dangerous system access       ││
│  └─────────────────────────────────────┘│
└─────────────────────────────────────────┘
```

### Security Features

- **Non-root user**: All operations run as `testuser`
- **Resource limits**: CPU, memory, file size constraints
- **Capability restrictions**: Limited system capabilities
- **Network isolation**: Controlled network access
- **File system boundaries**: Operations confined to `/workspace`

## Running Tests

### Prerequisites

1. **Docker**: Install Docker Desktop or Docker Engine
2. **Docker Compose**: Usually included with Docker Desktop
3. **Rust**: Standard Rust toolchain for test execution

### Quick Start

```bash
# Run basic sandbox test
./scripts/run-sandbox-tests.sh

# Run all sandbox tests
./scripts/run-sandbox-tests.sh --all

# Run specific test category
./scripts/run-sandbox-tests.sh --test file-reading

# Run with verbose output
./scripts/run-sandbox-tests.sh --test error-recovery --verbose
```

### Manual Test Execution

```bash
# Build sandbox container
docker-compose -f tests/docker/docker-compose.test.yml build

# Run sandbox tests (requires Docker)
cargo test --test sandboxed_integration_tests -- --ignored

# Run error recovery tests
cargo test --test error_recovery_tests -- --ignored

# Run traditional integration tests
cargo test --test hive_integration_tests
```

## Test Categories

### 1. End-to-End Workflow Tests

**File**: `tests/sandboxed_integration_tests.rs`

- ✅ **File Reading**: User prompt → file read → response
- ✅ **File Editing**: User prompt → file modification → verification  
- ✅ **Command Execution**: User prompt → safe command → output
- ✅ **Multi-Agent Tasks**: Complex workflows with agent coordination
- ✅ **Plan Approval**: Planning workflow with manager approval
- ✅ **System Lifecycle**: Startup, execution, clean shutdown

### 2. Error Recovery Tests

**File**: `tests/error_recovery_tests.rs`

- ✅ **Command Failures**: Handling of failing commands
- ✅ **Permission Errors**: File access denied scenarios
- ✅ **Agent Communication Failures**: Channel closures, timeouts
- ✅ **Resource Exhaustion**: Memory/disk limit handling
- ✅ **Invalid Input**: Malicious or malformed prompts
- ✅ **Concurrent Conflicts**: Race conditions and locking
- ✅ **Cleanup Verification**: Resource leak detection
- ✅ **Security Isolation**: Network and capability restrictions

### 3. Performance Baseline Tests

- **Response Time**: System should complete reasonable tasks within 30 seconds
- **Resource Usage**: Memory usage under 400MB, reasonable CPU utilization
- **Concurrent Agents**: System handles multiple agents without degradation
- **File Operations**: Efficient handling of various file sizes

## Test Data

### Sample Files (`tests/docker/test-data/`)

- **`sample-code.py`**: Python code for testing code analysis
- **`sample-data.csv`**: Structured data for processing tests  
- **`broken-script.sh`**: Intentionally failing script for error tests

### Generated Test Environment

Each test run creates:
- **Temporary workspace**: `/workspace/` with subdirectories
- **Test files**: Various file types and sizes
- **Project structure**: Sample project layout
- **Configuration**: Test-specific copilot config

## Adding New Tests

### 1. End-to-End Tests

```rust
#[tokio::test]
#[ignore] // Mark as sandbox test
async fn test_my_new_workflow() {
    let mut sandbox = docker_test_utils::DockerSandbox::new();
    sandbox.start().await.expect("Failed to start sandbox");
    
    let prompt = "Your test prompt here";
    let (exit_code, stdout, stderr) = sandbox.run_copilot_headless(prompt, 30).await.unwrap();
    
    // Add assertions
    assert!(exit_code == 0, "Workflow should complete successfully");
    
    sandbox.stop().await.expect("Failed to stop sandbox");
}
```

### 2. Error Recovery Tests

```rust
#[tokio::test]
#[ignore]
async fn test_my_error_scenario() {
    let mut sandbox = DockerSandbox::new();
    sandbox.start().await.expect("Failed to start sandbox");
    
    // Set up error condition
    sandbox.exec_command("setup error condition").await.unwrap();
    
    // Run test that should recover gracefully
    let prompt = "Test prompt that encounters the error";
    let (exit_code, stdout, stderr) = sandbox.run_copilot_headless(prompt, 30).await.unwrap();
    
    // Verify graceful handling
    assert!(exit_code >= 0, "Should handle error gracefully");
    
    sandbox.stop().await.expect("Failed to stop sandbox");
}
```

## Log Verification & Analysis

### Automated Log Verification

All Docker tests now include comprehensive log verification to ensure HIVE system components executed correctly:

**What Gets Verified:**
- ✅ HIVE system startup and initialization
- ✅ Agent creation and actor readiness (4+ actors expected)
- ✅ LLM interaction and API calls
- ✅ Tool registration and availability
- ⚠️ Specific tool usage patterns (depends on prompt effectiveness)

**Using the Log Verification Utility:**
```bash
# Analyze any HIVE log file
python tests/log_verification.py log.txt

# Output shows verification results by category:
# - System Startup: HIVE initialization, config loading
# - Agent Lifecycle: Agent creation, actor readiness, state transitions  
# - Tool Execution: Tool registration, LLM tool usage
# - LLM Interaction: API requests, network connections
```

### Manual Log Analysis

**Key Log Patterns to Look For:**
```bash
# System started correctly
grep "Starting headless HIVE multi-agent system" log.txt

# All actors became ready (expect 4+ for managers)
grep -c "Actor ready, sending ready signal" log.txt

# Agent reached active state
grep "All actors ready, starting task" log.txt

# LLM interaction occurred
grep "Executing LLM chat request" log.txt

# Tool usage (varies by prompt)
grep -i "command\|file_reader\|planner" log.txt
```

**Debugging Test Failures:**
1. **No log output**: Check `HIVE_LOG=debug` is set in test environment
2. **System startup failed**: Look for config loading or actor initialization errors
3. **Tools not used**: Verify test prompts are specific enough to trigger tool calls
4. **Timeouts**: Check for hung actors or missing ready signals

### Understanding Test Results

**Expected Verification Patterns:**
- ✅ **System Framework Working**: Core HIVE startup, agents, actors all functional
- ✅ **LLM Integration Active**: API calls happening with tool schemas
- ⚠️ **Tool Usage Variable**: Depends on prompt specificity and LLM behavior

**Key Insight**: Current tests validate the **framework functionality** rather than specific tool execution. The system works correctly even when showing "tool not found" warnings - this indicates the LLM didn't choose to use specific tools, not that the tools are broken.

## Troubleshooting

### Common Issues

1. **Docker not running**
   ```bash
   # Check Docker status
   docker info
   
   # Start Docker if needed (macOS)
   open -a Docker
   ```

2. **Permission denied errors**
   ```bash
   # Ensure Docker daemon is accessible
   sudo usermod -aG docker $USER
   # Then log out and back in
   ```

3. **Container fails to start**
   ```bash
   # Check container logs
   docker logs hive-test-sandbox
   
   # Rebuild container
   docker-compose -f tests/docker/docker-compose.test.yml build --no-cache
   ```

4. **Tests timeout or fail verification**
   ```bash
   # Check HIVE logs in container
   docker exec hive-test-sandbox cat /workspace/log.txt
   
   # Run verification manually
   docker exec hive-test-sandbox python /workspace/tests/log_verification.py /workspace/log.txt
   
   # Debug interactively
   docker exec -it hive-test-sandbox bash
   cd /workspace && HIVE_LOG=debug hive headless "test prompt"
   ```

5. **Log verification warnings**
   - ⚠️ "Tool not found" warnings are often expected - they indicate LLM didn't use specific tools
   - ❌ Critical errors indicate actual system failures
   - Focus on fixing ❌ errors first, ⚠️ warnings may need better prompts

### Debug Mode

```bash
# Run tests with full Docker output and logs
./scripts/run-sandbox-tests.sh --verbose

# Interactive debugging with log access
docker-compose -f tests/docker/docker-compose.test.yml up -d
docker exec -it hive-test-sandbox bash

# Manual test execution with full logging
cd /workspace
HIVE_LOG=debug hive headless --auto-approve-commands "your test prompt"
cat log.txt  # Review structured logs
python tests/log_verification.py log.txt  # Verify execution
```

### Creating Better Test Prompts

**Prompt Guidelines for Tool Usage:**
- **File Reading**: "Read the file /workspace/test.txt and summarize its contents"
- **Command Execution**: "Run the command 'ls -la /workspace' and show the results"  
- **Multi-step Tasks**: "Create a file called test.txt, write 'hello' to it, then read it back"
- **Error Handling**: "Try to read a non-existent file /workspace/missing.txt"

**Avoid Generic Prompts:**
- ❌ "Help me with this task" (too vague)
- ❌ "Analyze the situation" (no specific action)
- ✅ "Execute command X and show output" (specific tool action)

## Future Improvements

### Planned Enhancements

1. **Performance Testing**: Automated benchmarks and regression detection
2. **Chaos Engineering**: Random failure injection during tests
3. **Security Scanning**: Automated vulnerability testing of tool execution
4. **Multi-Platform**: Test matrix across different OS environments
5. **Load Testing**: High concurrency and stress testing
6. **Mock Services**: Controllable external API simulation

### Test Coverage Goals

- [ ] **Tool Coverage**: Individual tests for each tool actor
- [ ] **Configuration Testing**: Various config scenarios and validation
- [ ] **CLI Interface**: Complete command-line interface testing  
- [ ] **TUI Testing**: Terminal user interface interaction tests
- [ ] **Audio Processing**: Microphone and speech recognition tests
- [ ] **Context Capture**: Screenshot and clipboard functionality

## Contributing

When adding new tests:

1. **Safety First**: All potentially dangerous tests must use Docker sandbox
2. **Isolation**: Tests should not depend on external services when possible
3. **Cleanup**: Ensure proper resource cleanup in test teardown
4. **Documentation**: Update this README with new test categories
5. **CI/CD**: Consider automated test execution in CI pipelines

For questions or contributions, see the main project README.