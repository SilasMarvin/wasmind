# HIVE Multi-Agent System Architecture

## Overview

HIVE is a Rust-based multi-agent AI system that enables LLMs to collaborate on complex tasks through a hierarchical agent architecture. The system supports both GUI and headless modes, with Docker-based testing for safe tool execution.

## Core Concepts

### Agent Types
- **Main Manager**: Top-level agent that breaks down user requests and delegates to sub-agents
- **Sub-Manager**: Middle-tier agents that manage specific objectives and coordinate workers
- **Worker**: Execution agents that use tools to complete specific tasks

### Actor System
The system uses Tokio-based actors that communicate via broadcast channels:
- **Agent**: Core agent logic (Manager/Worker behavior)
- **Assistant**: Handles LLM interactions and chat requests
- **Tool Actors**: Execute specific capabilities (Command, FileReader, EditFile, MCP, etc.)
- **Context/Microphone**: GUI-only actors for screen capture and audio (feature-gated)

## Key Files & Directories

### Core System
- `src/main.rs` - Entry point, CLI argument parsing
- `src/lib.rs` - Main program runners (GUI/headless), logging setup
- `src/hive.rs` - HIVE system initialization and lifecycle management
- `src/config.rs` - Configuration loading, feature-conditional defaults
- `src/actors/` - Actor implementations and message handling

### Agents & Tools
- `src/actors/agent.rs` - Agent behavior, state management, actor coordination
- `src/actors/assistant.rs` - LLM chat interface and response handling
- `src/actors/tools/` - Tool implementations (command execution, file ops, planning, etc.)
- `src/actors/mod.rs` - Actor trait definition, message types, lifecycle

### Configuration
- `default_config.toml` - Full-featured config with GUI keybindings
- `headless_config.toml` - Minimal config for headless builds
- `src/key_bindings.rs` - Key event parsing and binding management

### Testing
- `tests/README.md` - **READ THIS** for comprehensive testing documentation
- `tests/sandboxed_integration_tests.rs` - Docker-based integration tests
- `tests/docker/` - Docker test environment setup
- `scripts/run-sandbox-tests.sh` - Test runner script

## Architecture Flow

### System Startup
1. **Config Loading**: Loads user config, merges with appropriate defaults (GUI vs headless)
2. **HIVE Initialization**: Creates broadcast channels, starts shared actors if needed
3. **Agent Creation**: Main Manager agent spawned with initial task
4. **Actor Coordination**: Agent starts required tool actors, waits for ready signals
5. **Task Processing**: Once all actors ready, agent begins processing user request

### Message Flow
```
User Input → HIVE System → Main Manager Agent → Assistant → LLM API
                ↓                ↓                ↓
         Tool Actors ← Agent Coordination ← Tool Calls
```

### Agent Lifecycle
```
Initializing → WaitingForActors → Active → [WaitingForApproval/WaitingForSubAgents] → Terminated
```

## Feature Flags

### Build Features
- `gui` - Enables screen capture, clipboard, context actors
- `audio` - Enables microphone recording and transcription
- `headless` - Minimal build for CLI-only usage

### Feature-Conditional Code
- Context/Microphone actors only exist with appropriate features
- Config loading adapts to available features
- Action enum variants are feature-gated

## Configuration System

### Config Hierarchy
1. User config file (`~/.config/hive/config.toml` or `HIVE_CONFIG_PATH`)
2. Default config (feature-dependent: `default_config.toml` vs `headless_config.toml`)
3. CLI overrides (e.g., auto-approve commands)

### Key Binding System
- String-based key combinations (`"ctrl-c"`, `"cmd-alt-w"`)
- Action mapping to enum variants
- Feature-conditional action validation
- `clear_defaults` option to disable default bindings

## Multi-Agent Coordination

### Task Delegation
- Managers use `spawn_agent_and_assign_task` tool to create sub-agents
- Parent-child communication via separate broadcast channels
- Status updates flow up the hierarchy

### Tool System
- Each agent type gets specific tools (Manager vs Worker)
- Tools are actors that register capabilities and handle requests
- MCP (Model Context Protocol) integration for external tools

## Error Handling & Recovery

### Config Validation
- Invalid actions for current build features are rejected
- Binding conflicts detected during parsing
- Missing environment variables handled gracefully

### Actor Failure Recovery
- Broadcast channel lag detection and logging
- Actor lifecycle monitoring via ready/status messages
- Graceful shutdown on system exit signals

## Logging & Observability

### Structured Tracing System
The system uses comprehensive structured logging with tracing spans for full observability:

**Key Log Environment Variables:**
- `HIVE_LOG=debug` - Enables detailed structured logging (writes to `log.txt`)
- `HIVE_LOG=info` - Standard operational logging
- `HIVE_LOG=trace` - Maximum verbosity (includes all spans)

**Important Span Hierarchies:**
```
start_headless_hive
├── agent_run (agent_id, role, agent_type)
│   ├── start_actors (agent_id, role)
│   │   └── actor_lifecycle (actor_id)
│   │       ├── tools_available (tools_count)
│   │       ├── user_input (input_length)
│   │       │   └── assist_request (tools_count)
│   │       │       └── llm_request (model, tools_count)
│   │       └── command_tool_call (call_id, function)
│   │           └── execute_command (command, args_count)
│   ├── send_initial_prompt
│   └── handle_internal_message (message_type)
```

**Critical Logging Patterns:**
- All actors send `ActorReady` messages when initialized
- Agent state transitions are logged with full context
- Tool executions include call IDs and parameters
- LLM requests show model, tool count, and network activity

### Debugging Workflows

**System Startup Issues:**
1. Check for "Starting headless HIVE multi-agent system" log entry
2. Verify all required actors send ready signals (expect 4+ for managers)
3. Look for "All actors ready, starting task" state transition
4. Confirm "Executing LLM chat request" appears

**Tool Execution Problems:**
1. Search logs for tool name patterns (e.g., "command", "file_reader") 
2. Check for `{tool}_tool_call` spans with proper call IDs
3. Verify tool registration in `tools_available` spans
4. Look for tool execution spans with parameters

**Agent Communication Issues:**
1. Monitor `handle_internal_message` spans for message flow
2. Check agent state transitions for stuck states
3. Verify parent-child agent communication patterns
4. Look for broadcast channel lag warnings

**Configuration Problems:**
1. Search for config loading entries and feature detection
2. Verify appropriate default config is loaded (GUI vs headless)
3. Check for invalid action/binding rejections
4. Confirm key binding merge logic works correctly

### Log Analysis Tools

**Built-in Verification:**
- `tests/log_verification.py` - Standalone log analysis utility
- Docker tests include automatic log verification
- Use `python tests/log_verification.py log.txt` for post-mortem analysis

**Manual Analysis Patterns:**
```bash
# Count actor readiness
grep "Actor ready" log.txt | wc -l

# Find tool usage
grep -o "tools_count=[0-9]*" log.txt | sort | uniq -c

# Check agent lifecycle
grep "agent_run.*enter\|agent_run.*exit" log.txt

# Monitor LLM interactions
grep "llm_request" log.txt
```

## Development Guidelines

### Adding New Tools
1. Implement `Actor` trait in `src/actors/tools/`
2. Add `#[tracing::instrument]` to key methods with relevant fields
3. Add to agent's required actors list in `get_required_actors()`
4. Start tool actor in agent's `start_actors()` method
5. Register tool schema for LLM usage
6. Update test verification to check for tool usage patterns

### Adding Tracing Spans
```rust
#[tracing::instrument(name = "operation_name", skip(self, large_params), fields(key_field = %value))]
async fn my_function(&self, param: String) -> Result<()> {
    tracing::info!("Starting operation");
    // ... implementation
    tracing::debug!("Operation details");
    Ok(())
}
```

**Span Best Practices:**
- Use descriptive span names that match the operation
- Include relevant fields (IDs, counts, types) but skip large data
- Add INFO level events for key milestones
- Use DEBUG for detailed flow information
- Include enter/exit events for timing analysis

### Testing Guidelines
- All Docker tests now include automatic log verification
- Test prompts should be specific enough to trigger tool usage
- Use log verification to ensure expected system behavior
- Check both positive cases (tools used) and negative cases (errors handled)

### Common Gotchas
- Headless builds need immediate Main Manager startup (no shared actors)
- Config merging must respect feature flags
- Broadcast channels require proper subscription timing
- Actor ready messages are crucial for system startup
- Tool usage requires explicit LLM prompts - generic prompts may not trigger tools
- Log verification expects specific patterns - update tests when changing log formats

## Current Status

The system successfully:
- ✅ Loads configs correctly for both GUI and headless builds
- ✅ Starts all required actors and handles ready signaling
- ✅ Processes user tasks through LLM interactions
- ✅ Executes tools and handles responses
- ✅ Builds and runs Docker integration tests

Critical fixes completed:
- Fixed config loading to prevent GUI actions in headless builds
- Fixed HIVE startup to handle empty shared actor requirements
- Fixed actor initialization and ready message handling