# Delegation Network

*A comprehensive example system demonstrating hierarchical AI agent coordination with Hive*

The Delegation Network is a collection of actors that enables sophisticated multi-agent workflows where AI agents can spawn subordinates, delegate tasks, coordinate work, and manage complex projects autonomously. This system showcases how simple, self-contained actors can combine to create powerful coordination capabilities.

## Overview

The delegation network solves the challenge of coordinating multiple AI agents working together on complex tasks. Instead of having a single agent try to handle everything, this system enables:

- **Hierarchical task delegation**: Managers can spawn specialized Workers and SubManagers
- **Autonomous coordination**: Agents communicate and coordinate without central control
- **Intelligent monitoring**: Built-in health checking and issue escalation
- **Flexible workflows**: Support for both simple delegation and complex multi-level management

The entire system is built using Hive's actor primitives, demonstrating how message passing and actor composition can create sophisticated multi-agent behaviors.

## Architecture

The delegation network supports three types of agents in a hierarchical structure:

### Agent Types

- **Root Manager**: The initial agent you interact with - the top-level coordinator of all work
- **SubManagers**: Mid-level managers spawned by the Root Manager to handle specific domains within larger projects
- **Workers**: Specialized agents that execute specific tasks (coding, research, analysis, etc.)

### Hierarchy Management

The hierarchy starts with a single **Root Manager** (the assistant you interact with). This Root Manager can spawn subordinates - either **Workers** for direct task execution or **SubManagers** for coordinating specific project domains. SubManagers can further spawn their own Workers, creating a tree-like structure with clear delegation chains. Hive's scope system naturally manages these relationships, with each agent having a unique scope that identifies its position in the hierarchy.

### Coordination

The network uses Hive's message broadcasting system to enable seamless coordination:
- Tool discovery through `ToolsAvailable` broadcasts
- Status updates and completion notifications
- Inter-agent communication and escalation
- Health monitoring and intervention

## Actors Included

The delegation network consists of 10 specialized actors, each designed to be simple and self-contained:

### Tool Actors (Provide capabilities to agents)

#### Core Delegation Tools
- **`spawn_agent`** - Create new Workers or SubManagers with specific roles and tasks
- **`send_message`** - Enable communication between agents in the hierarchy
- **`complete`** - Allow agents to formally signal task completion and report results

#### Coordination Tools  
- **`wait`** - Provide intelligent timing coordination for multi-agent workflows
- **`planner`** - Enable structured planning and progress tracking for complex projects
- **`send_manager_message`** - Allow subordinates to escalate issues to their managers

#### Health Management Tools
- **`report_normal`** - Used by health analyzers to report positive agent assessments
- **`flag_issue`** - Enable escalation of problematic agent behavior to managers

### Infrastructure Actors (Provide system services)

- **`delegation_network_coordinator`** - Tracks agent relationships and prevents invalid operations
- **`check_health`** - Monitors agent conversations and spawns health analyzers when needed

Each actor is relatively simple (~50-200 lines of core logic), demonstrating how complex systems can emerge from simple, composable building blocks using Hive's actor model.

## Usage

To use the delegation network, configure it in your Hive system:

```toml
starting_actors = ["delegation_network_coordinator"]

[actors.delegation_network_coordinator]
source = { url = "https://github.com/SilasMarvin/hive", package = "actors/delegation_network/crates/delegation_network_coordinator" }

# Actor overrides configure different assistant types with specific models
[actor_overrides.main_manager_assistant.config]
model_name = "gpt-4o"

[actor_overrides.sub_manager_assistant.config]
model_name = "gpt-4o"

[actor_overrides.worker_assistant.config]
model_name = "gpt-4o"

[actor_overrides.check_health_assistant.config]
model_name = "gpt-4o"
```

### Configuration Explained

- **starting_actors**: Only the coordinator starts initially - it manages the entire delegation network
- **Actor overrides**: Configure different assistant types for their specific roles
  - `main_manager_assistant`: The Root Manager you interact with - strategic orchestrator
  - `sub_manager_assistant`: SubManagers for domain coordination and technical architecture
  - `worker_assistant`: Workers for autonomous task execution
  - `check_health_assistant`: Health monitoring assistants

### System Prompts and Defaults

The delegation network coordinator's [Hive.toml](crates/delegation_network_coordinator/Hive.toml) contains comprehensive default system prompts for each assistant type, including:

- **Main Manager**: Detailed delegation protocols and strategic thinking patterns
- **SubManager**: Technical architecture guidelines and team coordination procedures  
- **Worker**: Autonomous implementation standards and problem-solving approaches

These extensive system prompts define the behavior and capabilities of each assistant role. The `actor_overrides` above allow you to customize these defaults (like changing the LLM model) while preserving the core delegation behaviors.

For more information about Hive configuration options, see the [Configuration README](../../crates/hive_config/README.md).

### Complete Setup

The network automatically handles tool discovery and coordination through Hive's broadcasting system. For complete working examples including LiteLLM setup and other infrastructure, see the examples directory (coming soon).

## Example Workflows

### Simple Delegation
1. **Root Manager** uses `spawn_agent` to create a **Worker** with a specific task
2. **Worker** executes the task using available tools (`execute_bash`, `file_interaction`, etc.)
3. **Worker** uses `complete` to signal finished work and report results
4. **Root Manager** receives completion notification and continues with next steps

### Complex Multi-Level Delegation  
1. **Root Manager** spawns a **SubManager** for a project domain (e.g., "Frontend Development")
2. **SubManager** uses `planner` to break down the work into phases
3. **SubManager** spawns multiple **Workers** for different components
4. **Workers** use `send_manager_message` to escalate blockers to **SubManager**
5. **SubManager** coordinates work and reports progress to **Root Manager**

### Health Monitoring and Intervention
1. **`check_health`** monitors agent conversations based on configured intervals
2. When issues are detected, health analyzers are spawned to assess agent behavior
3. Analyzers use either `report_normal` for healthy agents or `flag_issue` for problems
4. **`flag_issue`** pauses problematic agents and notifies their managers (SubManagers or Root Manager)
5. **Managers** provide guidance to resolve issues and resume work

## Actor Communication Flow

The delegation network leverages Hive's message passing system for coordination:

### Tool Discovery
- Each tool actor broadcasts `ToolsAvailable` during initialization
- AI agents automatically discover and can use available tools
- No manual tool registration required

### Task Execution  
- Agents receive `ExecuteTool` messages when AI decides to use tools
- Tool actors process requests and broadcast `ToolCallStatusUpdate` with results
- Status updates include both success/failure and rich UI display information

### Coordination Messages
- `SystemPromptContribution` messages provide usage guidance to agents
- `AddMessage` delivers communication between agents
- `RequestStatusUpdate` manages agent state transitions (waiting, done, etc.)

### Hierarchy Management
- Hive's scope system naturally tracks parent-child relationships
- The coordinator monitors `AgentSpawned` and `StatusUpdate` messages
- Invalid operations (like managers waiting with no subordinates) are prevented

## Assistant Status Integration

The delegation network works with Hive's [Assistant Actor](../assistant/README.md), which has a status system that shows what each assistant is currently doing - like working on a task, waiting for input, or coordinating with other agents. The delegation network uses these assistant statuses to coordinate work while keeping users in control. (For detailed information about how the assistant actor's status system works, see the [Assistant Actor README](../assistant/README.md#state-transitions).)

### How Assistant Statuses Enable Coordination

The delegation network uses several specific assistant actor statuses to coordinate work across the hierarchy:

- **WaitingForSystemInput**: Assistant actors pause and wait for guidance from their managers
- **WaitingForAgentCoordination**: Manager assistants wait for responses from their subordinates  
- **WaitingForUserInput**: The Root Manager assistant pauses and waits for direct user interaction
- **Processing**: Assistant actors are actively working on their assigned tasks

(These statuses are explained in detail in the [Assistant Actor README](../assistant/README.md#state-transitions).)

### Communication Patterns with Status Management

**Manager-to-Subordinate Communication:**
- When using `send_message` with `wait: true`, the manager assistant's status changes to `WaitingForAgentCoordination`
- The subordinate assistant receives the message and can respond or ask for clarification
- The manager assistant resumes working when the subordinate provides a response

**Subordinate-to-Manager Escalation:**
- When using `send_manager_message` with `wait: true`, the subordinate assistant's status changes to `WaitingForSystemInput`
- The subordinate assistant stays paused until their manager provides guidance
- This prevents assistants from getting stuck on blockers or making incorrect assumptions

**Coordination Between Operations:**
- The `wait` tool changes an assistant's status to `WaitingForSystemInput` between operations
- Assistants resume working when relevant events occur (task completions, messages, etc.)
- No need for arbitrary timeouts or polling

### User Interruptibility

**Always Available**: Users can interrupt any assistant at any time, regardless of what the assistant is currently doing:
- Assistants with `WaitingForAgentCoordination` status can be interrupted to provide new direction
- Assistants with `WaitingForSystemInput` status can receive user input instead of waiting for managers
- Even assistants with `Processing` status can be interrupted to change course

**Interrupting Delegation Chains**: When users interrupt assistants in the middle of delegation workflows:
- User input takes priority over manager-subordinate communication
- Subordinate assistants can be redirected without waiting for their original manager's response
- The hierarchy adapts to the new user direction

### Example Status Flows

**Worker Escalation Flow:**
1. **Worker assistant** encounters a blocker and uses `send_manager_message` with `wait: true`
2. **Worker assistant** status changes to `WaitingForSystemInput` (paused)
3. **SubManager assistant** receives escalation and provides guidance
4. **Worker assistant** status changes back to `Processing` and continues work

**Manager Coordination Flow:**
1. **Root Manager assistant** uses `send_message` to check on multiple subordinates with `wait: true`  
2. **Root Manager assistant** status changes to `WaitingForAgentCoordination`
3. **Subordinate assistants** respond with status updates
4. **Root Manager assistant** status changes back to `Processing` when all responses are received

**User Intervention Flow:**
1. **Any assistant** in any status can receive user input
2. User input takes priority over current waiting conditions
3. Assistant status changes to `Processing` to handle user direction
4. Original coordination resumes or is replaced based on user intent

This status system ensures that the delegation network operates smoothly while keeping users in full control of the process.

## Building Similar Systems

This delegation network demonstrates key patterns for building hierarchical agent systems with Hive:

- **Composable actors**: Each actor has a single, clear responsibility
- **Message-driven coordination**: Actors communicate through clean message interfaces  
- **Emergent behavior**: Complex workflows emerge from simple actor interactions
- **Flexible hierarchies**: The same patterns can support various organizational structures

The same architectural principles could be applied to build other coordination systems like customer service networks, research teams, or automated workflows.

---

*The delegation network showcases the power of Hive's actor model - sophisticated multi-agent coordination built from simple, self-contained building blocks.*
