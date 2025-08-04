# Wait Tool Actor

*Example tool actor for coordination and timing control within the Hive delegation network*

This tool actor enables AI agents to pause execution and wait for responses from other agents or system events. It provides intelligent timing coordination for multi-agent workflows where sequential operations or synchronization is required.

## Actor ID
`wait`

## Tools Provided

This actor exposes the following tool to AI agents:

### `wait`
- **Description**: Pause and wait for system or subordinate agent messages
- **Parameters**:
  - `reason`: Optional string describing why the agent is waiting (for logging/visibility)
- **Usage**: Coordinate agent responses, synchronize operations, wait for task completion

## When You Might Want This Actor

Include this actor in your Hive configuration when you need AI agents to:

- **Coordinate agent responses**: Wait for subordinate agents to complete tasks or respond to messages
- **Synchronize operations**: Pause between dependent operations that must occur in sequence
- **Manage workflow timing**: Control the flow of multi-step processes that require coordination
- **Handle asynchronous tasks**: Wait for long-running operations to complete before proceeding
- **Enable user interruption**: Allow users to intervene during waiting periods
- **Maintain delegation flow**: Ensure proper sequencing in hierarchical agent structures

This actor is essential for building coordinated multi-agent systems where timing and synchronization between agents is important.

## Messages Listened For

- `tools::ExecuteTool` - Receives tool execution requests for initiating wait periods
  - Handles `wait` tool calls with optional reason for waiting

## Messages Broadcast

- `tools::ToolsAvailable` - Announces the `wait` tool to AI agents when initialized
- `tools::ToolCallStatusUpdate` - Reports the results of wait operations
- `assistant::SystemPromptContribution` - Provides usage guidance and best practices
- `assistant::RequestStatusUpdate` - Updates agent status to "waiting" with appropriate context

## Configuration

No configuration required. The actor is ready to use once included in your actor list.

## How It Works

When activated in a Hive system, this actor:

1. **Registers the `wait` tool** with AI agents, enabling controlled pausing
2. **Provides comprehensive guidance** about when and how to use waiting effectively
3. **Sets agent status** to "waiting" with clear reasons for the pause
4. **Enables smart waking** by allowing the system to wake agents when relevant events occur
5. **Supports user interruption** allowing users to manually resume agents if needed
6. **Prevents unnecessary waiting** through guidance that discourages arbitrary delays

The actor ensures that agents wait intelligently and purposefully, with the system automatically waking them when relevant events occur rather than using fixed timeouts.