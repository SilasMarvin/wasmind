# Send Message Tool Actor

*Example tool actor for inter-agent communication within the Wasmind delegation network*

This tool actor enables AI agents to send messages to subordinate agents they have spawned or are managing. It provides a controlled communication channel for providing guidance, asking for updates, and coordinating work between agents in a delegation hierarchy.

## Actor ID
`send_message`

## Tools Provided

This actor exposes the following tool to AI agents:

### `send_message`
- **Description**: Send a message to a subordinate agent
- **Parameters**:
  - `agent_id`: The ID/scope of the target agent to receive the message
  - `message`: The message content to send
  - `wait`: Optional boolean to pause and wait for a response (default: false)
- **Usage**: Communicate with subordinates, provide guidance, request updates, coordinate work

## When You Might Want This Actor

Include this actor in your Wasmind configuration when you need AI agents to:

- **Communicate with subordinates**: Send messages to agents they have spawned or are managing
- **Provide course corrections**: Give updated instructions or guidance when tasks change
- **Request status updates**: Check on progress of long-running operations
- **Coordinate work**: Synchronize activities between multiple agents
- **Share new information**: Pass along updates that might affect ongoing work
- **Manage delegation workflows**: Maintain oversight and communication in hierarchical agent structures

This actor is essential for building effective delegation networks where manager agents need to stay in communication with their subordinate agents. See the [Delegation Network overview](../../README.md) for hierarchy patterns and communication flows.

## Messages Listened For

- `tools::ExecuteTool` - Receives tool execution requests for sending messages to other agents
  - **Scope**: Only listens to messages from its own scope (standard tool actor behavior)
  - Handles `send_message` tool calls with target agent ID, message content, and optional wait flag

## Messages Broadcast

- `tools::ToolsAvailable` - Announces the `send_message` tool to AI agents when initialized
- `tools::ToolCallStatusUpdate` - Reports the results of message sending operations
- `assistant::SystemPromptContribution` - Provides usage guidance and best practices via system prompts
- `assistant::AddMessage` - Delivers the actual message content to target agents
- `assistant::RequestStatusUpdate` - Optionally requests status updates when wait parameter is enabled

## Configuration

No configuration required. The actor is ready to use once included in your actor list.

## How It Works

When activated in a Wasmind system, this actor:

1. **Registers the `send_message` tool** with AI agents, enabling inter-agent communication
2. **Provides usage guidance** including when to use the tool, message guidelines, and examples
3. **Handles message delivery** by converting tool calls into system messages for target agents
4. **Supports coordination patterns** by optionally pausing the sender to wait for responses
5. **Manages communication flow** by providing clear feedback about message delivery status
6. **Prevents micromanagement** through guidance about appropriate usage patterns

The actor facilitates controlled, purposeful communication between agents while encouraging good delegation practices and agent autonomy.