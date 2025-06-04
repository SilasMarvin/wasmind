# HIVE Multi-Agent System Architecture

## Overview

HIVE is a Rust-based multi-agent AI system that enables LLMs to collaborate on complex tasks through a hierarchical agent architecture. The system supports both GUI and headless modes, with comprehensive Docker-based testing for safe tool execution.

## Core Architecture

### Agent Hierarchy
- **Main Manager**: Top-level agent that delegates tasks and coordinates sub-agents
- **Sub-Manager**: Middle-tier agents that manage specific objectives 
- **Worker**: Execution agents that use tools (file_reader, command, edit_file, complete)

### Actor System
Tokio-based actors communicate via broadcast channels:
- **Agent**: Core agent logic with Manager/Worker behavior variants
- **Assistant**: Handles LLM interactions and chat requests
- **Tool Actors**: Execute capabilities (Command, FileReader, EditFile, Complete, MCP, etc.)
- **Context/Microphone**: GUI-only actors for screen capture and audio (feature-gated)

## Key Files & Structure

### Core System
- `src/main.rs` - Entry point, CLI parsing
- `src/lib.rs` - Program runners, **JSON logging setup**
- `src/hive.rs` - HIVE initialization, message orchestration
- `src/actors/mod.rs` - **Message types (serializable)**, Actor trait, lifecycle
- `src/actors/agent.rs` - Agent behavior, **task completion logic**

### Agent Tools & Capabilities
- `src/actors/assistant.rs` - LLM chat interface
- `src/actors/tools/complete.rs` - **Task completion signaling tool**
- `src/actors/tools/spawn_agent.rs` - Agent delegation
- `src/actors/tools/` - Command execution, file operations, planning, MCP

### Configuration
- `default_config.toml` - Full GUI config
- `headless_config.toml` - Minimal headless config
- `src/config.rs` - Feature-conditional loading

## Message Architecture

### Core Message Types (Serializable)
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    Action(Action),                    // User actions
    AssistantToolCall(ToolCall),       // LLM tool calls
    AssistantResponse(MessageContent), // LLM responses
    ToolCallUpdate(ToolCallUpdate),    // Tool execution status
    TaskCompleted { summary: String, success: bool }, // Task completion
    AgentSpawned/AgentStatusUpdate,    // Agent management
    // ... other variants
}
```

### Agent Communication
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InterAgentMessage {
    TaskStatusUpdate { task_id, status, from_agent },
    PlanApproved/PlanRejected,
    // ... delegation messages
}
```

## Task Completion System

### Complete Tool Pattern
**Critical**: All agents must use the `complete` tool to signal task completion:

```rust
// In system prompts:
"When you have finished your assigned task, you MUST call the 'complete' tool to signal completion."

// Tool call:
complete({
    "summary": "Task description of what was accomplished",
    "success": true  // or false if task failed
})
```

### Message Flow
```
User Input → Main Manager → spawn_agent_and_assign_task → Worker Agent
                ↓                                            ↓
        TaskStatusUpdate ← complete tool call ← Task Execution
```

## Logging & Debugging

### Structured JSON Logging
**Environment**: `HIVE_LOG=debug` (writes to `log.txt`)

**Message Serialization**: All Message and InterAgentMessage objects are logged as JSON:
```json
{
  "timestamp": "2024-01-01T12:00:00Z",
  "level": "DEBUG", 
  "target": "hive::agent",
  "message": "{\"TaskCompleted\":{\"summary\":\"File read\",\"success\":true}}",
  "message_type": "hive::actors::Message"
}
```

### Key Span Patterns
```
start_headless_hive → agent_run → actor_lifecycle → llm_request
                                 → complete_tool_call → task_completion_signal
```

### Debug Patterns
- System startup: `start_headless_hive` span
- Agent lifecycle: `agent_run` spans with agent_id/role
- Actor readiness: "Actor ready, sending ready signal" (expect 4+ for managers)
- Tool execution: `{tool}_tool_call` spans
- Completion: `complete_tool_call`, `task_completion_signal`, `TaskCompleted` messages

## Testing System

### Docker Integration Tests
**Location**: `tests/sandboxed_integration_tests.rs`
**Environment**: Safe Docker sandbox with whitelisted commands
**Purpose**: End-to-end testing of agent workflows, tool execution, task completion

### Structured Log Analysis
**Parser**: `tests/log_parser/mod.rs` - Deserializes Message objects from logs
**Verification**: `tests/docker_sandbox/mod.rs` - Type-safe log analysis

**Key Verification Checks**:
```rust
// Message-based verification
parser.entries_with_task_completed()       // TaskCompleted messages
parser.entries_with_assistant_tool_calls() // Tool call messages  
parser.entries_with_tool_call("complete")  // Specific tool usage
parser.contains_message_sequence(["AssistantToolCall", "TaskCompleted"])
```

### Test Best Practices
- Use explicit prompts that force tool usage: "You must use your tools to..."
- Always instruct agents to use complete tool: "When done, use the complete tool"
- Test both success and error scenarios
- Verify proper agent delegation patterns

## Development Guidelines

### Adding New Tools
1. Implement `Actor` trait in `src/actors/tools/`
2. Add `Serialize, Deserialize` to any new message types
3. Register in agent's required actors list
4. Add JSON logging for tool execution
5. Update test verification patterns

### Message Type Guidelines
- All message types must be serializable for logging
- Use `#[serde(skip)]` for non-serializable fields (like SystemTime)
- Include message_type in debug logs for filtering
- Maintain backwards compatibility in message structure

### Agent Behavior Guidelines
**Managers**: Must delegate to Workers, use spawn_agent_and_assign_task tool
**Workers**: Must use tools (file_reader, command, edit_file) AND complete tool
**Completion**: All agents must signal task completion explicitly

### Debugging Workflows

**System Won't Start**:
1. Check `start_headless_hive` span appears
2. Verify 4+ "Actor ready" messages  
3. Look for agent state transitions

**Tool Execution Issues**:
1. Search for `AssistantToolCall` messages in logs
2. Check tool registration and availability
3. Verify tool-specific debug spans

**Task Completion Problems**:
1. Look for `complete_tool_call` debug messages
2. Check `TaskCompleted` message presence
3. Verify agent completion sequence patterns

## Feature Flags & Builds

### Build Features
- `gui` - Screen capture, context actors
- `audio` - Microphone recording  
- Default - Headless CLI build

### Feature-Conditional Code
```rust
#[cfg(feature = "gui")]
Context::new(config.clone(), tx.clone()).run();
```

## Configuration System

### Config Hierarchy
1. User config (`HIVE_CONFIG_PATH` or `~/.config/hive/config.toml`)
2. Feature-appropriate defaults (GUI vs headless)
3. CLI overrides

### Model Configuration
```toml
[hive.main_manager_model]
name = "deepseek-chat"
system_prompt = "You are a Main Manager. Delegate tasks using spawn_agent_and_assign_task. Use complete tool when done."

[hive.worker_model] 
system_prompt = "You are a Worker. Use tools to complete tasks. MUST call complete tool when finished."
```

## Current Status & Critical Fixes

**✅ Completed**:
- Hierarchical agent system with proper delegation
- Complete tool implementation and verification
- Structured JSON logging with message deserialization
- Docker-based integration testing with log analysis
- Feature-conditional config loading
- Type-safe log parsing and verification

**🔑 Key Patterns**:
- All agents must use complete tool for task signaling
- Messages are serialized as JSON for precise log analysis  
- Test verification uses structured message parsing, not string matching
- Agent hierarchy enforces proper tool usage (Managers delegate, Workers execute)

The system now provides comprehensive observability and testing capabilities for reliable multi-agent task execution.