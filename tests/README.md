# HIVE Testing Framework

This directory contains comprehensive testing for the HIVE multi-agent AI system, with structured log analysis and Docker-based sandbox testing for safe end-to-end workflows.

## Testing Architecture

### 1. Unit Tests
- **Location**: `src/` directories with `#[cfg(test)]` modules
- **Coverage**: Agent state management, message handling, configuration
- **Status**: ✅ Core functionality coverage

### 2. Integration Tests
- **Location**: `tests/hive_integration_tests.rs`
- **Coverage**: Multi-agent communication, task delegation
- **Status**: ✅ Agent coordination workflows

### 3. **Docker Sandbox Tests**
- **Location**: `tests/sandboxed_integration_tests.rs`
- **Coverage**: Complete user workflows with real tool execution
- **Status**: ✅ Safe end-to-end testing with structured log verification

### 4. **Structured Log Analysis**
- **Location**: `tests/log_parser/mod.rs`, `tests/docker_sandbox/mod.rs`
- **Coverage**: Type-safe message parsing and workflow verification
- **Status**: ✅ Message-based log analysis with JSON deserialization

## Sandbox Testing

### Why Docker Sandbox?
HIVE executes real commands and modifies files. Docker sandbox provides:
- **Safety**: Isolated environment prevents host damage
- **Realism**: Tests actual tool execution, not mocks  
- **Consistency**: Reproducible across machines
- **Security**: Resource limits and capability restrictions

### Sandbox Environment
```
┌─────────────────────────────────────────┐
│ Host System                             │
│  ┌─────────────────────────────────────┐│
│  │ Docker Container (Ubuntu 22.04)    ││
│  │  ┌─────────────────────────────────┐││
│  │  │ HIVE Process                    │││
│  │  │  - JSON structured logging      │││
│  │  │  - Message serialization        │││
│  │  │  - Safe tool execution          │││
│  │  │  - Task completion verification  │││
│  │  └─────────────────────────────────┘││
│  │  Whitelisted Tools:                 ││
│  │  - Shell utilities (ls, cat, grep)  ││
│  │  - Dev tools (git, python, node)    ││
│  │  - File operations (mkdir, touch)   ││
│  └─────────────────────────────────────┘│
└─────────────────────────────────────────┘
```

## Running Tests

### Quick Start
```bash
# Run basic sandbox test
./scripts/run-sandbox-tests.sh

# Run all sandbox tests
./scripts/run-sandbox-tests.sh --all

# Run specific workflow
./scripts/run-sandbox-tests.sh --test file-reading --verbose

# Manual execution
cargo test --test sandboxed_integration_tests -- --ignored --nocapture
```

### Prerequisites
1. **Docker** with Docker Compose
2. **Rust** toolchain
3. **Environment**: `HIVE_LOG=debug` for detailed logging

## Test Categories & Verification

### 1. **End-to-End Workflow Tests**

**File**: `tests/sandboxed_integration_tests.rs`

All tests now include **structured log verification** that analyzes actual Message objects:

#### Test Types:
- ✅ **File Reading**: `test_sandboxed_file_reading_workflow`
- ✅ **Command Execution**: `test_sandboxed_command_execution_workflow` 
- ✅ **Error Recovery**: `test_sandboxed_error_recovery`
- ✅ **Multi-Step Tasks**: `test_sandboxed_multi_step_workflow`

#### Verification System:
```rust
// Message-based verification (not string matching)
let verification = sandbox.verify_log_execution(&stdout, &expected_tools)?;

// Checks 14 different system aspects:
verification.hive_startup                  // HIVE system initialization
verification.agent_started                // Agent lifecycle 
verification.actors_ready_count           // Actor readiness (expect 4+)
verification.task_delegation              // Manager → Worker delegation
verification.tool_calls_executed          // AssistantToolCall messages
verification.complete_tool_called         // Task completion signaling
verification.task_completed_messages      // TaskCompleted messages
verification.proper_completion_sequence   // Message flow verification
```

### 2. **Structured Log Analysis**

#### Message Deserialization
**Parser**: `tests/log_parser/mod.rs`
- Deserializes actual `Message` and `InterAgentMessage` objects from logs
- Type-safe analysis using Rust pattern matching
- No more brittle string matching

#### Key Analysis Methods:
```rust
// Precise message filtering
parser.entries_with_task_completed()       // TaskCompleted messages
parser.entries_with_assistant_tool_calls() // Tool call messages
parser.entries_with_tool_call("complete")  // Specific tool usage
parser.contains_message_sequence(["AssistantToolCall", "TaskCompleted"])

// Agent workflow verification  
parser.entries_with_hive_messages()        // All HIVE messages
parser.entries_with_inter_agent_messages() // Agent communication
```

#### Verification Results:
```
🔍 Structured Log Verification Results:
========================================
📋 System Lifecycle:
  ✅ HIVE system startup
  ✅ Agent started  
  ✅ 5 actors ready (expected >= 4)
📋 Task Management:
  ✅ LLM requests
  ✅ Task delegation
  ✅ 4 Worker agent references
📋 Tool Execution:
  ✅ Tool calls executed
  ✅ Command execution
  ✅ File operations
📋 Task Completion:
  ✅ Complete tool called - proper task completion
  ✅ Task completion signaled
  ✅ TaskCompleted messages
  ✅ Proper completion sequence
```

## Task Completion Testing

### Critical Pattern: Complete Tool Usage
All agents must use the `complete` tool to signal task completion:

```rust
// Test prompts now include completion instruction:
let prompt = "Read file /workspace/test.txt. When done, use the complete tool to signal completion.";

// Verification checks for completion patterns:
result.complete_tool_called         // Debug: complete_tool_call
result.task_completion_signaled     // Debug: task_completion_signal  
result.task_completed_messages      // Message: TaskCompleted
result.proper_completion_sequence   // Flow: ToolCall → TaskCompleted
```

### Agent System Prompts
Tests configure agents to require completion:
```toml
[hive.main_manager_model]
system_prompt = "You are a Main Manager. Delegate tasks using spawn_agent_and_assign_task. Use complete tool when done."

[hive.worker_model]
system_prompt = "You are a Worker. Use tools to complete tasks. MUST call complete tool when finished."
```

## JSON Logging & Message Analysis

### Structured Logging Format
**Environment**: `HIVE_LOG=debug` writes structured logs to `log.txt`

**Message Serialization**: All Message objects logged as JSON:
```json
{
  "timestamp": "2024-01-01T12:00:00Z",
  "level": "DEBUG",
  "target": "hive::agent", 
  "message": "{\"TaskCompleted\":{\"summary\":\"File read successfully\",\"success\":true}}",
  "message_type": "hive::actors::Message"
}
```

### Log Analysis Flow
```
Raw Logs → JSON Parser → Message Deserialization → Type-Safe Analysis → Verification Results
```

## Adding New Tests

### 1. End-to-End Test Template
```rust
#[tokio::test]
#[ignore] // Mark as sandbox test
async fn test_my_workflow() {
    let mut sandbox = DockerSandbox::new();
    sandbox.start().await.expect("Failed to start sandbox");
    
    // Include completion instruction in prompt
    let prompt = "Your specific task here. When done, use the complete tool to signal completion.";
    
    let (exit_code, stdout, stderr) = sandbox.run_hive_headless(prompt, 60).await.unwrap();
    
    // Use structured verification
    let verification = sandbox.verify_log_execution(
        &stdout,
        &["expected_tool", "complete"], // Include "complete" in expected tools
    ).unwrap();
    
    // Assert on structured results
    assert!(verification.is_successful(), "Basic system checks failed");
    assert!(verification.task_delegation, "Expected task delegation");
    assert!(verification.is_successful_with_completion(), "Task completion verification failed");
    
    sandbox.stop().await.expect("Failed to stop sandbox");
}
```

### 2. Log Analysis Test Template
```rust
#[test]
fn test_message_parsing() {
    let log_content = r#"
{"timestamp":"2024-01-01T12:00:00Z","level":"DEBUG","target":"hive::agent","message":"{\"TaskCompleted\":{\"summary\":\"test\",\"success\":true}}"}
"#;
    
    let parser = LogParser::parse_log_content(log_content).unwrap();
    let task_completed = parser.entries_with_task_completed();
    assert_eq!(task_completed.len(), 1);
}
```

## Debugging & Troubleshooting

### Common Issues

#### 1. **System Startup Failures**
```bash
# Check HIVE initialization
grep "start_headless_hive" log.txt

# Verify actor readiness (expect 4+ for managers)
grep -c "Actor ready, sending ready signal" log.txt

# Look for agent state transitions
grep "agent_run" log.txt
```

#### 2. **Task Completion Issues**
```bash
# Check for completion debug messages
grep "complete_tool_call" log.txt

# Look for TaskCompleted messages
grep "TaskCompleted" log.txt

# Verify completion sequence
grep -A5 -B5 "complete.*tool" log.txt
```

#### 3. **Tool Execution Problems**
```bash
# Check for AssistantToolCall messages
grep "AssistantToolCall" log.txt

# Look for specific tool usage
grep "file_reader\|command\|spawn_agent" log.txt

# Verify tool registration
grep "ToolsAvailable" log.txt
```

### Debug Mode
```bash
# Interactive debugging
docker-compose -f tests/docker/docker-compose.test.yml up -d
docker exec -it hive-test-sandbox bash

# Manual execution with full logging
cd /workspace
HIVE_LOG=debug hive headless --auto-approve-commands "your test prompt"

# Analyze logs with structured parser
python -c "
import json
with open('log.txt') as f:
    for line in f:
        if 'TaskCompleted' in line:
            print(json.loads(line.strip()))
"
```

### Test Best Practices

#### Effective Test Prompts
```rust
// ✅ Good - Specific, forces tool usage, includes completion
"Read the file /workspace/test.txt and show its contents. When done, use the complete tool."

// ✅ Good - Multi-step with clear completion
"Create file test.txt with content 'hello', then read it back. Use complete tool when finished."

// ❌ Avoid - Too generic, no completion instruction
"Help me with this file"

// ❌ Avoid - No specific tool requirement
"Analyze the situation"
```

#### Verification Patterns
```rust
// Check basic system health
assert!(result.is_successful(), "System lifecycle failed");

// Verify task delegation (for multi-agent workflows)
assert!(result.task_delegation, "Expected manager delegation");

// Check task completion (critical for workflow verification)
assert!(result.is_successful_with_completion(), "Task completion failed");

// Specific tool verification
assert!(!result.expected_tools["file_reader"], "File reader tool not used");
```

## Architecture Benefits

### Type-Safe Testing
- **Before**: Brittle string matching in logs
- **After**: Structured message object analysis with Rust enums

### Comprehensive Coverage
- **System Lifecycle**: Startup, actor readiness, agent states
- **Message Flow**: Tool calls, responses, completion signals
- **Task Completion**: Explicit completion verification via `complete` tool
- **Error Handling**: Proper error detection vs false positives

### Debugging Clarity
- **Structured Results**: Clear pass/fail for each system component
- **Message Tracing**: Full workflow visibility through message objects
- **Root Cause Analysis**: Precise failure point identification

The testing framework now provides **comprehensive verification** of HIVE system behavior through **message-based analysis** rather than fragile log string matching, enabling confident development and debugging of the multi-agent system.