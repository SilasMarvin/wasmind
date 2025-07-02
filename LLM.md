# HIVE Multi-Agent System - LLM Developer Guide

## Overview

HIVE is a Rust-based multi-agent AI system that enables LLMs to collaborate on complex tasks through a hierarchical agent architecture. The system uses an actor-based model with Tokio for concurrency and supports both GUI and headless modes.

## Quick Start

**Key Concepts:**
- **Agents**: Autonomous units that can be either Managers (delegate tasks) or Workers (execute tasks)
- **Actors**: Message-passing entities that handle specific functionality (tools, UI, etc.)
- **Scopes**: UUID-based isolation boundaries between agent instances
- **Tools**: Function-like capabilities exposed to LLMs via JSON schemas

## Core Architecture

### Agent Hierarchy
```
Main Manager (ROOT_AGENT_SCOPE)
├── Sub-Manager Agents
│   ├── Worker Agents
│   └── Additional Sub-Managers
└── Worker Agents
```

- **Main Manager**: Entry point agent, handles user requests and top-level delegation
- **Sub-Manager**: Intermediate managers that can spawn and coordinate other agents
- **Worker**: Task execution agents with access to system tools

### Key System Components

1. **Entry Points** (`src/main.rs`, `src/lib.rs`):
   - CLI command parsing (run, headless, prompt-preview)
   - Runtime initialization with tokio multi-threaded executor
   - JSON logging to `log.txt` (controlled by `HIVE_LOG` env var)

2. **HIVE Orchestrator** (`src/hive.rs`):
   - `start_hive()`: GUI mode with TUI and key bindings
   - `start_headless_hive()`: CLI mode for single task execution
   - Message routing between agents via broadcast channels
   - Exit handling and agent lifecycle management

3. **Actor System** (`src/actors/mod.rs`):
   - Base `Actor` trait with message handling and lifecycle hooks
   - Scope-based message filtering for agent isolation
   - Automatic actor registration and ready signals

### Agent Implementation (`src/actors/agent.rs`)

```rust
pub struct Agent {
    pub r#type: AgentType,      // MainManager, SubManager, or Worker
    pub scope: Uuid,            // This agent's scope
    pub parent_scope: Uuid,     // Parent agent's scope
    pub role: String,           // E.g., "Software Engineer"
    pub task_description: Option<String>,
}
```

**Tool Access by Agent Type:**
- **Managers**: `planner`, `spawn_agents`, `send_message`, `wait`, `complete` (sub-managers only)
- **Workers**: `execute_command`, `read_file`, `edit_file`, `planner`, `send_manager_message`, `wait`, `complete`, MCP tools (dynamically loaded)

## Message Architecture

### Core Message Types
All messages in HIVE are serializable for logging and debugging:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    // User interactions
    Action(Action),                    // User keyboard/TUI actions
    UserContext(UserContext),          // User input, screenshots, audio
    
    // LLM interactions
    AssistantToolCall(ToolCall),       // LLM invoking a tool
    AssistantResponse(MessageContent), // LLM text responses
    
    // Tool execution
    ToolCallUpdate(ToolCallUpdate),    // Tool execution status updates
    ToolsAvailable(Vec<Tool>),         // Available tools broadcast
    
    // System state updates
    FileRead { path, content, last_modified },
    FileEdited { path, content, last_modified },
    PlanUpdated(TaskPlan),
    
    // Agent lifecycle
    Agent(AgentMessage),               // Inter-agent communication
    ActorReady { actor_id },           // Actor initialization signal
}
```

### Inter-Agent Communication
Agents communicate using specialized message types:

```rust
pub enum InterAgentMessage {
    StatusUpdate { status: AgentStatus },
    StatusUpdateRequest { status: AgentStatus },
    Message { message: String },  // Direct messages between agents
}

pub enum AgentStatus {
    Processing { id: Uuid },
    Wait { reason: WaitReason },
    Done(AgentTaskResult),
}

pub enum WaitReason {
    WaitingForUserInput,
    WaitForDuration { tool_call_id: String, timestamp: u64, duration: u64 },
    WaitingForPlanApproval { tool_call_id: String },
    WaitingForTools { tool_calls: Vec<String> },
    WaitingForActors { pending_actors: Vec<String> },
}
```

### Message Routing
- Each message includes a `scope` (UUID) for agent isolation
- Actors filter messages based on scope to ensure proper isolation
- Broadcast channel enables pub/sub messaging pattern

## Tool System

### Tool Categories

1. **Manager Tools**:
   - `spawn_agents`: Create new agents (array input for multiple agents at once)
   - `planner`: Create and manage task plans
   - `send_message`: Send messages to subordinate agents
   - `wait`: Wait for specified duration (with interrupt capability)
   - `complete`: Signal task completion (sub-managers and headless main manager)

2. **Worker Tools**:
   - `execute_command`: Run shell commands (with whitelisting, 30s default/600s max timeout)
   - `read_file`: Read file contents with caching (10MB max, 64KB auto-read limit)
   - `edit_file`: Line-based file editing (validates file hasn't changed)
   - `planner`: Create plans (automatically requests manager approval)
   - `send_manager_message`: Send messages to manager agent
   - `wait`: Wait for specified duration (with interrupt capability)
   - `complete`: Signal task completion
   - MCP tools: Dynamically loaded from configured MCP servers

### Tool Implementation Pattern
Each tool is an Actor that:
1. Broadcasts its availability on startup
2. Listens for `AssistantToolCall` messages
3. Sends `ToolCallUpdate` status messages
4. Returns results via `ToolCallStatus::Finished`

### Tool Schemas
Tools expose JSON schemas for LLM interaction:

```json
// Example: complete tool
{
  "name": "complete",
  "description": "Signal task completion",
  "schema": {
    "type": "object",
    "properties": {
      "summary": { "type": "string" },
      "success": { "type": "boolean" }
    },
    "required": ["summary", "success"]
  }
}

// Example: spawn_agents tool (array input)
{
  "name": "spawn_agents",
  "description": "Spawn new agents to help with tasks",
  "schema": {
    "type": "object",
    "properties": {
      "agents_to_spawn": {
        "type": "array",
        "items": {
          "type": "object",
          "properties": {
            "agent_role": { "type": "string" },
            "task_description": { "type": "string" },
            "agent_type": { "type": "string", "enum": ["Worker", "Manager"] }
          },
          "required": ["agent_role", "task_description", "agent_type"]
        }
      }
    },
    "required": ["agents_to_spawn"]
  }
}

// Example: read_file tool
{
  "name": "read_file",
  "description": "Reads content from a file. For small files (<64KB), it reads the entire file. For large files, it returns an error with metadata, requiring you to specify a line range.",
  "schema": {
    "type": "object",
    "properties": {
      "path": { "type": "string" },
      "start_line": { "type": "integer", "description": "Optional starting line (1-indexed)" },
      "end_line": { "type": "integer", "description": "Optional ending line (inclusive)" }
    },
    "required": ["path"]
  }
}
```

## System State Management

### SystemState (`src/system_state.rs`)
Maintains context injected into LLM prompts:
- **Files**: Managed by FileReader actor with caching (10MB limit, line-numbered format)
- **Plans**: Active task plans with status tracking (Pending, InProgress, Completed, Skipped)
- **Agents**: Spawned agents and their task assignments (tracks spawn time for ordering)

### FileReader (`src/actors/tools/file_reader.rs`)
Centralized file caching system:
- **Size Limits**:
  - Maximum file size: 10MB (`MAX_FILE_SIZE_BYTES`)
  - Automatic full read: Files ≤64KB (`SMALL_FILE_SIZE_BYTES`)
  - Large files require line range specification
- **File Format**: All content includes line numbers: `1|first line\n2|second line`
- **Error Messages**: Include JSON metadata for large files:
  ```json
  {"path": "file.txt", "size_bytes": 123456, "total_lines": 5000}
  ```
- **Caching**: Tracks modification times, supports partial content merging
- **Shared Access**: `Arc<tokio::sync::Mutex<FileReader>>` for thread-safe access

### Template System (`src/template.rs`)
Jinja2 template support for dynamic system prompts:

```jinja2
{% if task -%}
<assigned_task>{{ task }}</assigned_task>
{% endif %}

<available_tools>
{% for tool in tools -%}
- {{ tool.name }}: {{ tool.description }}
{% endfor %}
</available_tools>

<system_info>
Current time: {{ current_datetime }}
System: {{ os }} ({{ arch }})
Working directory: {{ cwd }}
</system_info>
```

Available template variables:
- `tools`, `task`, `current_datetime`, `os`, `arch`, `cwd`
- `whitelisted_commands`, `files`, `plan`, `agents`
- `id` - The unique identifier (scope) of the current agent
- `role` - The agent's role (e.g., "Software Engineer", "QA Tester")

## Configuration System

### Config Loading Priority
1. Environment variable: `HIVE_CONFIG_PATH`
2. User config: `~/.config/hive/config.toml`
3. Built-in defaults: `default_config.toml` or `headless_config.toml`

### Key Configuration Sections
```toml
# Command execution
auto_approve_commands = false
whitelisted_commands = ["ls", "git", "cargo", "npm", ...]

# Key bindings (GUI mode)
[key_bindings]
bindings = { "cmd-alt-a" = "Assist", "ctrl-c" = "Exit" }

# Agent models
[hive.main_manager_model]
name = "model-name"
system_prompt = "..." # Supports Jinja2 templates
endpoint = "optional-override"
auth = "ENV_VAR_NAME"

# MCP servers
[mcp_servers.server_name]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem"]
```

### MCP Integration (`src/actors/tools/mcp.rs`)
Model Context Protocol support for external tool providers:
- **Dynamic Tool Loading**: Tools are loaded from configured MCP servers at runtime
- **Multi-Server Support**: Can connect to multiple MCP servers simultaneously
- **Tool Mapping**: Each tool is mapped to its originating server
- **Content Types**: Supports text, image, resource, and audio content
- **Protocol**: Uses JSON-RPC over stdio for communication
- **Tool Execution**: Routes tool calls to appropriate MCP server

## Development Workflow

### Running HIVE
```bash
# GUI mode with TUI
cargo run --features gui,audio -- run

# Headless mode for single task
cargo run -- headless "implement a fibonacci function"

# Preview system prompts
cargo run -- prompt-preview --all
```

### Debugging Tips
1. **Enable debug logging**: `HIVE_LOG=debug cargo run ...`
2. **Check log.txt**: JSON-formatted messages for parsing
3. **Key log patterns**:
   - `agent_run` - Agent lifecycle events
   - `AssistantToolCall` - Tool invocations
   - `TaskCompleted` - Completion signals
   - `Actor ready` - Actor initialization
   - `file_reader_tool_call` - File read operations
   - `InterAgentMessage` - Agent-to-agent communication
4. **Common debugging scenarios**:
   - File access issues: Check FileReader cache and modification times
   - Tool failures: Verify tool availability broadcasts
   - Agent communication: Monitor scope-based message filtering
   - MCP tools: Check MCP server connection and tool loading
   - Wait timeouts: Track WaitForDuration status updates

### Testing
- Integration tests use Docker sandbox for safety
- Log parser enables message-based verification
- Test patterns verify tool usage and task completion

## Common Patterns

### Task Delegation Flow
```
1. Manager receives task
2. Manager calls spawn_agents tool
3. New agent created with specific role
4. Agent works on task using tools
5. Agent calls complete tool
6. Manager receives TaskStatusUpdate
```

### File Editing Pattern
```
1. Worker calls read_file tool
2. FileReader caches content with line numbers
3. Worker calls edit_file tool with line-based changes
4. System validates file hasn't changed (modification time)
5. Edits processed in reverse order to maintain line integrity
6. System updates FileEdited state
```

### Plan Approval Pattern
```
1. Worker creates plan using planner tool
2. Status changes to WaitingForPlanApproval
3. Manager receives InterAgentMessage with plan
4. Manager approves/rejects via StatusUpdate message
5. Worker proceeds based on decision
```

### Inter-Agent Communication Pattern
```
1. Agent calls send_message or send_manager_message tool
2. Tool sends InterAgentMessage::Message
3. Target agent receives in their message stream
4. Response flows back through normal channels
```

### Wait Pattern
```
1. Agent calls wait tool with duration (seconds)
2. Status changes to Wait with WaitForDuration reason
3. Timer runs in background
4. Agent resumes on timer completion or interruption
```

## Best Practices

1. **Always use the complete tool** - Every agent must signal completion
2. **Check tool availability** - Tools broadcast their presence on startup
3. **Use structured logging** - All messages are JSON-serializable
4. **Leverage templates** - Dynamic prompts adapt to available tools and state
5. **Test with Docker** - Safe environment for command execution
6. **Monitor scopes** - Each agent operates in its own scope
7. **Handle errors gracefully** - All tools return structured errors via `ToolCallStatus`
8. **File operations** - Always check if files exist and validate cache staleness
9. **Line ranges for large files** - Use start_line/end_line for files >64KB
10. **Inter-agent communication** - Use send_message/send_manager_message for coordination
11. **Wait interrupts** - The wait tool can be interrupted by user input or agent messages
12. **Command timeouts** - Commands default to 30s timeout, max 600s for long operations

## Error Handling Patterns

### File Access Errors
- Large files return JSON metadata with path, size, and line count
- Missing files return clear error messages
- Cache staleness detected via modification time comparison

### Tool Execution Errors
- Command timeouts return truncated output with error indication
- Permission errors include command and working directory context
- Tool unavailability errors suggest checking tool registration

### Agent Communication Errors
- Invalid agent scopes return "agent not found" errors
- Message delivery failures include retry suggestions
- Plan approval timeouts trigger automatic escalation

For implementation details, refer to:
- `src/actors/agent.rs` - Agent lifecycle and tool access
- `src/actors/tools/` - Individual tool implementations
- `src/actors/tools/file_reader.rs` - File caching and reading system
- `src/actors/tools/mcp.rs` - Model Context Protocol integration
- `src/system_state.rs` - Context management and template data
- `src/template.rs` - Jinja2 template system and context
- `docs/system_prompt_templates.md` - Template guide