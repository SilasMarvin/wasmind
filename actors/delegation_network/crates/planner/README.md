# Planner Tool Actor

*Example tool actor for strategic planning and progress tracking within the Wasmind delegation network*

This tool actor enables AI agents to create structured plans for complex multi-step tasks and systematically track progress. It provides a formal planning framework that helps agents break down large objectives into manageable components with clear status tracking.

## Tools Provided

This actor exposes the following tool to AI agents:

### `planner`
- **Description**: Create and update structured task plans with progress tracking
- **Parameters**:
  - `title`: Name of the overall plan
  - `tasks`: Array of task objects, each containing:
    - `description`: What needs to be done
    - `status`: Current state - 'pending', 'in_progress', 'completed', or 'skipped'
- **Usage**: Plan complex projects, track progress, manage project phases, coordinate parallel efforts

## When You Might Want This Actor

Include this actor in your Wasmind configuration when you need AI agents to:

- **Plan complex projects**: Break down large tasks into sequential phases with clear milestones
- **Track progress systematically**: Monitor task completion across multiple components
- **Coordinate parallel efforts**: Manage multiple simultaneous workstreams
- **Document workflow steps**: Create formal plans before executing complex operations
- **Identify dependencies**: Map out task relationships and prerequisites
- **Manage project phases**: Track pending, in-progress, completed, and skipped tasks

This actor is ideal for AI agents managing complex projects that benefit from structured planning and progress tracking.

## Messages Listened For

- `tools::ExecuteTool` - Receives tool execution requests for creating and updating plans
  - **Scope**: Only listens to messages from its own scope (standard tool actor behavior)
  - Handles `planner` tool calls with task plans including title and task list

## Messages Broadcast

- `tools::ToolsAvailable` - Announces the `planner` tool to AI agents when initialized
- `tools::ToolCallStatusUpdate` - Reports the results of planning operations
- `assistant::SystemPromptContribution` - Provides usage guidance and planning best practices
- Plan content contributions to system prompt for ongoing visibility

## Configuration

```toml
[actors.planner]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/delegation_network/crates/planner" }
```

No configuration required. The actor is ready to use once included in your actor list. These actors are typically spawned by the delegation_network_coordinator.

## How It Works

When activated in a Wasmind system, this actor:

1. **Registers the `planner` tool** with AI agents for structured planning capabilities
2. **Provides comprehensive guidance** including best practices and example plans for different domains
3. **Creates visual task representations** with status icons for easy progress tracking
4. **Updates system prompts** with current plan status for continuous visibility
5. **Supports iterative planning** allowing agents to update task statuses as work progresses
6. **Enables strategic thinking** by encouraging agents to plan before executing complex workflows

The actor helps agents approach complex tasks methodically, with clear planning phases and systematic progress tracking throughout the execution lifecycle.

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*