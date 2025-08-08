# Spawn Agent Tool Actor

*Example tool actor for creating and managing new AI agents within the Hive delegation network*

This tool actor enables AI agents to create specialized subordinate agents with specific roles and tasks. It supports creating Worker agents for direct task execution, Manager agents for coordination and delegation, and SubManager agents for mid-level project management.

## Actor ID
`spawn_agent`

## Tools Provided

This actor exposes the following tool to AI agents:

### `spawn_agent`
- **Description**: Create one or more new agents with specific roles and tasks
- **Parameters**:
  - `agents_to_spawn`: Array of agent definitions (at least one required), each containing:
    - `agent_role`: The specific role for the agent (e.g., 'Software Engineer', 'QA Tester')
    - `task_description`: Clear description of the task assigned to this agent
    - `agent_type`: Type of agent - 'Worker', 'Manager', or 'SubManager'
  - `wait`: Optional boolean to pause and wait for response from spawned agents
- **Usage**: Delegate complex tasks, create specialized workers, build management hierarchies

## When You Might Want This Actor

Include this actor in your Hive configuration when you need AI agents to:

- **Delegate complex tasks**: Break down large projects into smaller tasks for specialized agents
- **Create specialized workers**: Spawn agents with specific expertise (coding, research, analysis, etc.)
- **Build management hierarchies**: Create manager agents that can further delegate and coordinate work
- **Scale task execution**: Handle multiple parallel tasks by creating dedicated agents for each
- **Project management**: Create sub-managers for different domains within larger projects
- **Workflow automation**: Build complex multi-agent workflows where agents spawn other agents as needed

This actor is essential for building sophisticated delegation networks where agents can dynamically create and manage teams of specialized subordinate agents. See the [Delegation Network overview](../../README.md) for complete system architecture.

## Messages Listened For

- `tools::ExecuteTool` - Receives tool execution requests for creating new agents
  - **Scope**: Only listens to messages from its own scope (standard tool actor behavior)
  - Handles `spawn_agent` tool calls with agent definitions including role, task, and type

## Messages Broadcast

- `tools::ToolsAvailable` - Announces the `spawn_agent` tool to AI agents when initialized
- `tools::ToolCallStatusUpdate` - Reports the results of agent spawning operations
- `assistant::SystemPromptContribution` - Provides usage guidance and injects task descriptions into spawned agents
- `assistant::AddMessage` - Sends initial task messages to newly created agents
- `assistant::RequestStatusUpdate` - Optionally requests status updates if wait parameter is enabled

## Configuration

Requires configuration to specify which actors should be used for different agent types:

```toml
[spawn_agent]
worker_actors = ["assistant", "execute_bash", "file_interaction"]
sub_manager_actors = ["assistant", "execute_bash", "file_interaction", "spawn_agent", "send_message"]
```

## How It Works

When activated in a Hive system, this actor:

1. **Registers the `spawn_agent` tool** with AI agents, enabling them to create subordinate agents
2. **Provides comprehensive usage guidance** including examples and best practices via system prompts
3. **Processes spawn requests** by creating new agent instances with specified roles and capabilities
4. **Configures new agents** by injecting task descriptions and role information into their system prompts
5. **Manages agent communication** by sending initial task messages to newly spawned agents
6. **Supports hierarchical structures** by allowing different actor configurations for Workers vs Managers
7. **Handles coordination** by optionally waiting for initial responses from spawned agents

This actor enables the creation of dynamic, hierarchical agent networks where agents can intelligently delegate work to specialized subordinates based on task requirements.

## Building

To build the Spawn Agent Actor WASM component:

```bash
cargo component build
```

This generates `target/wasm32-wasip1/debug/spawn_agent.wasm` for use in the Hive system.