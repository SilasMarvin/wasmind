# Flag Issue Tool Actor

*Example tool actor for reporting agent issues within the Wasmind delegation network*

This tool actor enables health analyzer agents to flag problematic behavior in monitored agents. When issues are detected, it interrupts the problematic agent and escalates the issue to their manager for intervention.

## Actor ID
`flag_issue`

## Tools Provided

This actor exposes the following tool to AI agents:

### `flag_issue`
- **Description**: Flag that an analyzed agent appears stuck, looping, or having issues
- **Parameters**:
  - `issue_summary`: Brief description of the detected problem (e.g., "Agent is repeatedly trying the same failed action")
- **Usage**: Used by health analyzer agents to escalate detected problems to managers

## When You Might Want This Actor

Include this actor in your Wasmind configuration when you need:

- **Issue escalation**: Report stuck, looping, or problematic agent behavior to managers
- **Agent intervention**: Pause problematic agents before they waste resources or cause issues
- **Manager notification**: Alert parent agents when their subordinates need guidance
- **Behavioral monitoring**: Detect and respond to agents making no progress or repeating failures
- **Quality control**: Prevent agents from continuing down unproductive paths
- **Automated supervision**: Enable health checkers to take corrective action

This actor is essential for health monitoring systems that need to detect and escalate agent issues for manager intervention.

## Messages Listened For

- `tools::ExecuteTool` - Receives tool execution requests for flagging issues
  - **Scope**: Only listens to messages from its own scope (standard tool actor behavior)
  - Handles `flag_issue` tool calls with issue summary describing the problem

## Messages Broadcast

- `tools::ToolsAvailable` - Announces the `flag_issue` tool to AI agents when initialized
- `tools::ToolCallStatusUpdate` - Reports successful issue flagging
- `assistant::InterruptAndForceWaitForSystemInput` - Pauses the problematic agent
- `assistant::AddMessage` - Notifies the manager about the flagged issue
- `actors::Exit` - Signals the analyzer agent to terminate after reporting

## Configuration

```toml
[actors.flag_issue]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/delegation_network/crates/flag_issue" }
```

No configuration required. The actor is ready to use once included in your actor list. These actors are typically spawned by the delegation_network_coordinator.

## How It Works

When activated in a Wasmind system, this actor:

1. **Registers the `flag_issue` tool** with health analyzer agents
2. **Processes issue reports** including the nature of the detected problem
3. **Identifies agent hierarchy** by finding the monitored agent and their manager
4. **Interrupts problematic agents** forcing them to wait for manager guidance
5. **Escalates to managers** with detailed alerts about the issue and context
6. **Triggers analyzer exit** after successfully flagging the issue

This actor provides critical intervention capabilities, allowing automated health checks to pause problematic agents and escalate issues to human or AI managers for resolution.

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*