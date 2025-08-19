# Send Manager Message Tool Actor

*Example tool actor for upward communication to managers within the Wasmind delegation network*

This tool actor enables AI agents to send messages upward to their direct manager when they need guidance, are blocked, or have critical updates. It provides a structured escalation path for subordinate agents to communicate with their managers in the delegation hierarchy.

## Actor ID
`send_manager_message`

## Tools Provided

This actor exposes the following tool to AI agents:

### `send_manager_message`
- **Description**: Send a message to your direct manager for escalation or guidance
- **Parameters**:
  - `message`: The content to send to the manager
  - `wait`: Optional boolean to pause all work until manager responds (default: false)
- **Usage**: Escalate blockers, request guidance, report critical updates, seek additional resources

## When You Might Want This Actor

Include this actor in your Wasmind configuration when you need AI agents to:

- **Escalate blockers**: Report issues that prevent task completion and need manager intervention
- **Request guidance**: Ask for clarification on requirements, priorities, or approach
- **Report critical updates**: Communicate important discoveries that affect the overall plan
- **Seek additional resources**: Request permissions, access, or capabilities beyond current scope
- **Handle exceptions**: Escalate unexpected situations that require management decisions
- **Coordinate changes**: Inform managers about scope changes or timeline impacts

This actor is essential for subordinate agents in delegation networks who need to communicate upward to their managers for support and guidance.

## Messages Listened For

- `tools::ExecuteTool` - Receives tool execution requests for sending messages to managers
  - **Scope**: Only listens to messages from its own scope (standard tool actor behavior)
  - Handles `send_manager_message` tool calls with message content and optional wait flag

## Messages Broadcast

- `tools::ToolsAvailable` - Announces the `send_manager_message` tool to AI agents when initialized
- `tools::ToolCallStatusUpdate` - Reports the results of message sending operations
- `assistant::SystemPromptContribution` - Provides usage guidance and escalation best practices
- `assistant::AddMessage` - Delivers the message content to the parent manager agent
- `assistant::RequestStatusUpdate` - Optionally pauses agent execution when waiting for response

## Configuration

No configuration required. The actor is ready to use once included in your agent list.

## How It Works

When activated in a Wasmind system, this actor:

1. **Registers the `send_manager_message` tool** with subordinate agents for upward communication
2. **Provides comprehensive guidance** about appropriate escalation scenarios and message formatting
3. **Identifies parent managers** automatically through the agent hierarchy
4. **Delivers messages upward** by sending system messages to parent agents
5. **Supports blocking wait** allowing agents to pause all work pending manager response
6. **Prevents over-escalation** through guidance about when not to contact managers

The actor facilitates effective upward communication while encouraging agent autonomy and appropriate escalation practices.

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*