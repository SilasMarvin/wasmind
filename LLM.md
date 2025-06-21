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
- **Managers**: `planner`, `spawn_agent`, `plan_approval`, `complete` (sub-managers only)
- **Workers**: `command`, `read_file`, `edit_file`, `planner`, `mcp`, `complete`

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
    TaskStatusUpdate { status: AgentTaskStatus },
    PlanApproved { plan_id },
    PlanRejected { plan_id, reason },
}

pub enum AgentTaskStatus {
    Done(Result<AgentTaskResultOk, String>),
    InProgress,
    AwaitingManager(TaskAwaitingManager),
    Waiting { tool_call_id: String },
}
```

### Message Routing
- Each message includes a `scope` (UUID) for agent isolation
- Actors filter messages based on scope to ensure proper isolation
- Broadcast channel enables pub/sub messaging pattern

## Tool System

### Tool Categories

1. **Manager Tools**:
   - `spawn_agents`: Create new agents (Worker or Manager type)
   - `planner`: Create and manage task plans
   - `approve_plan` / `reject_plan`: Review plans from subordinates
   - `complete`: Signal task completion (sub-managers and headless main manager)

2. **Worker Tools**:
   - `execute_command`: Run shell commands (with whitelisting)
   - `read_file`: Read file contents with caching
   - `edit_file`: Modify files with various operations
   - `planner`: Create plans for manager approval
   - `complete`: Signal task completion
   - MCP tools: Dynamically loaded from MCP servers

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
```

## System State Management

### SystemState (`src/system_state.rs`)
Maintains context injected into LLM prompts:
- **Files**: Currently loaded file contents with metadata
- **Plans**: Active task plans with status tracking
- **Agents**: Spawned agents and their task assignments

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
2. System updates FileRead state
3. Worker calls edit_file tool with changes
4. System validates file hasn't changed
5. System updates FileEdited state
```

### Plan Approval Pattern
```
1. Worker creates plan using planner tool
2. Status changes to AwaitingManager
3. Manager receives plan for review
4. Manager calls approve_plan or reject_plan
5. Worker proceeds based on decision
```

## Best Practices

1. **Always use the complete tool** - Every agent must signal completion
2. **Check tool availability** - Tools broadcast their presence on startup
3. **Use structured logging** - All messages are JSON-serializable
4. **Leverage templates** - Dynamic prompts adapt to available tools
5. **Test with Docker** - Safe environment for command execution
6. **Monitor scopes** - Each agent operates in its own scope
7. **Handle errors gracefully** - Use Result types and error messages

For implementation details, refer to:
- `src/actors/agent.rs` - Agent lifecycle and tool access
- `src/actors/tools/` - Individual tool implementations
- `src/system_state.rs` - Context management
- `docs/system_prompt_templates.md` - Template guide