# Complete Tool Actor

*Example tool actor for task completion signaling within the Hive delegation network*

This tool actor enables AI agents to formally signal the completion of their assigned tasks. It provides a structured way for agents to report their results, update their status, and notify parent agents in the delegation hierarchy about task outcomes.

## Actor ID
`complete`

## Tools Provided

This actor exposes the following tool to AI agents:

### `complete`
- **Description**: Signal task completion and provide results
- **Parameters**:
  - `summary`: A brief description of what was accomplished
  - `success`: Boolean indicating whether the task completed successfully or failed
- **Usage**: Formally complete assigned tasks, report results to managers, update agent status

## When You Might Want This Actor

Include this actor in your Hive configuration when you need AI agents to:

- **Signal task completion**: Formally indicate when assigned work is finished
- **Report results**: Provide summaries of what was accomplished during task execution
- **Update status**: Change agent status from working to completed in the system
- **Notify parent agents**: Inform managers or coordinators about task outcomes
- **Maintain workflow integrity**: Ensure proper closure of delegated tasks
- **Track project progress**: Enable parent agents to monitor completion of delegated work

This actor is essential for delegation networks where agents need to formally complete tasks and report results to their managers or coordinators. See the [Delegation Network overview](../../README.md) for complete workflow examples.

## Messages Listened For

- `tools::ExecuteTool` - Receives tool execution requests for task completion
  - Handles `complete` tool calls with task summary and success status

## Messages Broadcast

- `tools::ToolsAvailable` - Announces the `complete` tool to AI agents when initialized
- `tools::ToolCallStatusUpdate` - Reports the successful use of the complete tool
- `assistant::RequestStatusUpdate` - Updates the agent's status to "Done" in the system
- `assistant::AddMessage` - Sends completion notification to parent agents in the hierarchy

## Configuration

No configuration required. The actor is ready to use once included in your agent list.

## How It Works

When activated in a Hive system, this actor:

1. **Registers the `complete` tool** with AI agents, enabling formal task completion
2. **Processes completion requests** by parsing the summary and success status
3. **Updates agent status** by broadcasting a status change to "Done" with the task results
4. **Notifies parent agents** by sending completion messages up the delegation hierarchy
5. **Provides structured feedback** with clear success/failure indicators and summaries
6. **Maintains system integrity** by ensuring proper task lifecycle management

This actor ensures that tasks are formally closed with appropriate documentation and notification, enabling effective coordination in multi-agent delegation workflows.