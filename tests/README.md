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
   docker logs copilot-test-sandbox
   
   # Rebuild container
   docker-compose -f tests/docker/docker-compose.test.yml build --no-cache
   ```

4. **Tests timeout**
   - Increase timeout values in test functions
   - Check if container has sufficient resources
   - Verify network connectivity for external dependencies

### Debug Mode

```bash
# Run tests with full Docker output
./scripts/run-sandbox-tests.sh --verbose

# Interactive debugging
docker-compose -f tests/docker/docker-compose.test.yml up -d
docker exec -it copilot-test-sandbox bash
```

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