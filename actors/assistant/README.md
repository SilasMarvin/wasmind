# Assistant Actor

*Conversational AI agent for chat interactions and tool execution*

The Assistant Actor is a conversational AI agent that manages chat interactions, tool execution, and dynamic system prompt generation within the Wasmind actor system.

This is just one version of an Assistant that can be used within Wasmind with sane defaults and reasonable interoperability. In reality, anyone can create their own assistant with their own state management and messages.

## Actor ID
`wasmind:assistant`

## Quick Reference

**Key Concepts:**
- **Default State**: `WaitingForSystemInput` with `interruptible_by_user: true` 
- **External Control**: Any state can be overridden via `RequestStatusUpdate` or `QueueStatusChange`
- **Tool Status Updates**: Tools can request status changes and have their results queued until coordination completes
- **System Prompt**: Dynamic contributions from any actor using `SystemPromptContribution` messages

**Common States:**
- `WaitingForSystemInput` - Default waiting state (accepts user + system messages)
- `Processing` - Actively communicating with LLM
- `WaitingForTools` - Tools executing in parallel
- `WaitingForAgentCoordination` - Coordination with other agents (external only)
- `Done` - Task completion and shutdown (external only)

**Key Messages:**
- `AddMessage` - Add user/system messages to conversation
- `SystemPromptContribution` - Contribute content to system prompt
- `QueueStatusChange` - External state control (queued for next submit)
- `ToolCallStatusUpdate` - Tool execution results

## Overview

The Assistant Actor serves as the primary interface for LLM-based interactions, handling:
- Chat message management and history
- Tool call orchestration
- Dynamic system prompt rendering
- State management for conversation flow
- Integration with LiteLLM for model inference

## Configuration

```toml
[assistant]
model_name = "gpt-4"
base_url = "http://localhost:4000"  # Optional, can be provided via LiteLLM

[assistant.system_prompt]
base_template = """
You are a helpful AI assistant.

{% for section_name, contributions in sections -%}
## {{ section_name | title }}

{% for contribution in contributions -%}
{{ contribution }}

{% endfor -%}
{% endfor -%}
""" # Optional, can be written to by actors

# Override specific contribution templates
[assistant.system_prompt.overrides]
"file_reader.open_files" = "Currently open: {{ data.files | length }} files"

# Exclude unwanted contributions
exclude = ["debug.verbose_info"]
```

## Tools Provided

This actor does not provide tools to other agents. Instead, it consumes tools provided by other actors and coordinates their execution during LLM interactions.

## Message Handling

The Assistant Actor processes several message types:

### Incoming Messages

**Scope Behavior**: This actor listens to messages that target its own scope via the `agent` field in message structures.

#### `assistant::AddMessage`
Adds user or system messages to the pending conversation.
```rust
AddMessage {
    agent: "assistant-scope",  // Must match this assistant's scope
    message: ChatMessage::User(UserChatMessage {
        content: "Hello, world!".to_string(),
    }),
}
```

#### `assistant::Response`
Self-sent message containing LLM responses with potential tool calls.
```rust
Response {
    request_id: uuid::Uuid,
    message: AssistantChatMessage {
        content: Some("I'll help you with that...".to_string()),
        tool_calls: Some(vec![...]),
    },
}
```

#### `tools::ToolCallStatusUpdate`
Updates on tool execution progress and results.
```rust
ToolCallStatusUpdate {
    id: "tool-call-123".to_string(),
    status: ToolCallStatus::Done {
        result: Ok(ToolCallResult {
            content: "Command executed successfully".to_string(),
            ui_display_info: UIDisplayInfo { ... },
        }),
    },
}
```

#### `litellm::BaseUrlUpdate`
Provides the base URL for LiteLLM inference.
```rust
BaseUrlUpdate {
    base_url: "http://localhost:4000".to_string(),
    models_available: vec!["gpt-4".to_string()],
}
```

#### `assistant::SystemPromptContribution`
Dynamic contributions to the system prompt from any actor.
```rust
SystemPromptContribution {
    agent: "assistant-scope".to_string(),
    key: "file_reader.open_files".to_string(),
    content: SystemPromptContent::Data {
        data: json!({"files": [{"name": "main.rs", "size": 1024}]}),
        default_template: "Open files:\n{% for file in data.files %}- {{ file.name }}\n{% endfor %}".to_string(),
    },
    priority: 100,
    section: Some("context".to_string()),
}
```

#### `assistant::QueueStatusChange`
Queues a status change for the assistant's next submit operation, providing external control over state without interrupting current operations. See [State Management](#state-management) for details on available states.
```rust
QueueStatusChange {
    agent: "assistant-scope".to_string(), // Must match this assistant's scope
    status: Status::Wait {
        reason: WaitReason::WaitingForSystemInput {
            required_scope: Some("manager-scope".to_string()),
            interruptible_by_user: true,
        },
    },
}
```

#### `assistant::CompactedConversation`
The conversation has been compacted. Replace the old chat message history with the new
```rust
CompactedConversation {
    agent: Scope,
    messages: Vec<ChatMessageWithRequestId>,
}
```

### Outgoing Messages

#### `assistant::Response`
Broadcasts LLM responses for other actors to process.

#### `tools::ExecuteTool`
Requests tool execution when the LLM generates tool calls.

#### `assistant::StatusUpdate`
Notifies other actors of state changes (future implementation).

## State Management

The Assistant Actor uses a state machine to manage conversation flow and coordination. States are organized into logical groups based on their purpose.

### State Overview

| State | Purpose | Set By | Can Be Forced |
|-------|---------|--------|---------------|
| `WaitingForAllActorsReady` | Initial startup coordination | System | ✓ |
| `WaitingForLiteLLM` | Wait for LLM service | Assistant | ✓ |
| `WaitingForSystemInput` | Default waiting (system/user messages) | Assistant | ✓ |
| `WaitingForUserInput` | Force user interaction only | External only | ✓ |
| `WaitingForAgentCoordination` | Inter-agent communication | External only | ✓ |
| `WaitingForTools` | Tool execution in progress | Assistant | ✓ |
| `Processing` | LLM request/response cycle | Assistant | ✓ |
| `Done` | Task completion & shutdown | External only | ✓ |

**External State Control**: Any state can be overridden using `RequestStatusUpdate` or `QueueStatusChange` messages from other actors. See [Tool-Initiated Status Updates](#tool-initiated-status-updates) for implementation details.

### Initialization States

#### `WaitingForAllActorsReady`
The assistant's starting state, ensuring system-wide readiness before operation.
- **Purpose**: Prevents processing before all required actors are initialized
- **Enters from**: Initial creation
- **Exits to**:
  - `Processing` - if messages are already queued and LiteLLM is available
  - `WaitingForSystemInput` - if ready to accept input and LiteLLM is available
  - `WaitingForLiteLLM` - if the LLM service isn't available yet

#### `WaitingForLiteLLM`
Waits for the language model service to become available.
- **Purpose**: Handles graceful startup when LiteLLM service is still initializing
- **Enters from**: 
  - `WaitingForAllActorsReady` - when actors are ready but LLM isn't
- **Exits to**:
  - `Processing` - if messages are queued when LLM becomes available
  - `WaitingForSystemInput` - when LLM is ready and waiting for input

### Waiting States

#### `WaitingForSystemInput`
- **Purpose**: The primary waiting state for the assistant, enabling both system-level coordination and user interaction
- **Note**: In Wasmind, events are typically represented as system messages, making this the natural default state
- **Configuration**:
  - `required_scope` - if set, only accepts messages from that specific actor
  - `interruptible_by_user` - whether user messages can interrupt the wait
- **Enters from**:
  - `WaitingForAllActorsReady` - standard ready state after initialization
  - `WaitingForLiteLLM` - after LLM becomes available
  - `Processing` - after LLM responds without tool calls (automatically with `interruptible_by_user: true`)
  - `Processing` - after any errors occur during LLM communication (this may change)
  - External control via `RequestStatusUpdate` or `QueueStatusChange` messages
- **Exits to**:
  - `Processing` - when appropriate message is received (system message from required scope, or user message if interruptible)
- **Default configuration**: The assistant typically uses `required_scope: None, interruptible_by_user: true`, allowing it to respond to both users and any system actor

#### `WaitingForUserInput`
A specialized state that only accepts user messages, queuing but not immediately processing all system messages.
- **Purpose**: Forces user interaction before continuing, useful for explicit user confirmation scenarios
- **Note**: This state is never set by the assistant itself - only by external actors
- **Enters from**:
  - External `RequestStatusUpdate` message
  - External `QueueStatusChange` message
- **Exits to**:
  - `Processing` - when user sends a message

#### `WaitingForAgentCoordination`
Manages inter-agent communication through tool-based coordination.
- **Purpose**: Allows agents to communicate and coordinate through the assistant
- **Note**: This state is never set by the assistant itself - only by external actors (typically tools). See [Tool-Initiated Status Updates](#tool-initiated-status-updates) for examples.
- **Configuration**:
  - `coordinating_tool_call_id` - tracks which tool initiated coordination
  - `target_agent_scope` - specific agent being waited for (if any)
  - `user_can_interrupt` - whether users can interrupt the coordination
- **Enters from**:
  - External `RequestStatusUpdate` message from tools that need agent coordination
  - External `QueueStatusChange` message
- **Exits to**:
  - `Processing` - when coordination completes (tool call finishes)

#### `WaitingForTools`
Tracks execution of multiple tool calls from an LLM response.
- **Purpose**: Manages parallel tool execution and result collection
- **Enters from**:
  - `Processing` - when LLM response includes tool calls
  - External control via `RequestStatusUpdate` or `QueueStatusChange` messages
- **Exits to**:
  - `Processing` - automatically when all tools complete (resubmits to LLM with results)

### Active States

#### `Processing`
Waiting on an API request to the model provider
- **Purpose**: Handles the LLM request/response cycle
- **Enters from**:
  - Any waiting state when appropriate input is received (user or system messages)
  - `WaitingForTools` - automatically after all tools complete
- **Exits to**:
  - `WaitingForTools` - if LLM response includes tool calls
  - `WaitingForSystemInput` (with `interruptible_by_user: true`) - after successful text response or on any error
  - `Processing` - if `require_tool_call` is enabled but LLM didn't use tools (retries with error message)

### Terminal State

#### `Done`
Final state indicating the conversation or task has completed.
- **Purpose**: Signals task completion and triggers system shutdown
- **Special behavior**: Broadcasts `Exit` message to shutdown all actors in the current scope
- **Note**: This state is never set by the assistant itself - only by external actors
- **Enters from**:
  - External `RequestStatusUpdate` message with `Done` status
  - External `QueueStatusChange` message with `Done` status
  - Typically sent by tools that determine task completion (e.g., task completion tools, user exit commands)
- **Exits to**: None (terminal state, triggers shutdown)

## Common State Flows

### Normal Conversation
```
WaitingForSystemInput → Processing → WaitingForSystemInput
```
User or system sends message → LLM processes → Assistant responds → Return to default waiting state

### Tool Execution Flow
```
WaitingForSystemInput → Processing → WaitingForTools → Processing → WaitingForSystemInput
```
Input received → LLM generates tool calls → Tools execute in parallel → Results collected and sent to LLM → Final response → Return to waiting

### System Startup Flow
```
WaitingForAllActorsReady → WaitingForLiteLLM → WaitingForSystemInput
```
Initial state → Wait for LLM service → Ready for operation

**Alternative startup** (if LLM already available):
```
WaitingForAllActorsReady → WaitingForSystemInput
```

### External State Control Examples
```
WaitingForSystemInput → [QueueStatusChange] → WaitingForUserInput
WaitingForTools → [RequestStatusUpdate] → WaitingForAgentCoordination
Processing → [QueueStatusChange] → Done
```
External actors can override any state using `RequestStatusUpdate` or `QueueStatusChange`

### Error Recovery
```
Processing → WaitingForSystemInput (interruptible_by_user: true)
```
Any LLM communication error → Return to default waiting state with user interruption enabled

### Task Completion Flow
```
Any State → [External QueueStatusChange] → Done → [Exit Broadcast] → [System Shutdown]
```
External completion signal → Terminal state → Shutdown message → All actors in scope terminate

### Agent Coordination Flow
```
WaitingForTools → [Tool RequestStatusUpdate] → WaitingForAgentCoordination → [External event] → Processing
```
Tool execution → Tool requests coordination state → Tool result queued → External event completes coordination → Resume with all tool results

**Example with [`send_message`](../delegation_network/crates/send_message/src/lib.rs) tool**:
1. LLM calls `send_message` with `wait: true`
2. Tool sends message to target agent
3. Tool sends `RequestStatusUpdate` → Assistant enters `WaitingForAgentCoordination` 
4. Tool result ("Message sent, waiting for response") is queued
5. Target agent eventually responds with system message
6. Assistant returns to `Processing` and submits tool result to LLM

## Tool-Initiated Status Updates

Tools can send `RequestStatusUpdate` messages during execution to change the assistant's state, enabling advanced coordination patterns. This is commonly used when tools need the assistant to wait for external events or agent responses.

### How Tool Status Updates Typically Work

1. **Tool Execution Begins**: Assistant enters `WaitingForTools` when LLM response includes tool calls
2. **Tool Requests Status Change**: During execution, tool sends `RequestStatusUpdate` to change assistant state
3. **Assistant State Changes**: Assistant immediately transitions to the requested state (e.g., `WaitingForAgentCoordination`)
4. **Tool Completes**: Tool sends `ToolCallStatusUpdate` with success/error result
5. **Tool Response Queued**: The tool result is stored but **not immediately submitted to the LLM**
6. **Coordination Continues**: Assistant remains in the requested state until coordination completes
7. **Resume Processing**: When coordination finishes, assistant returns to `Processing` and submits **all queued tool results** to the LLM

**Key Point**: When a tool requests a status update, its response is held until the coordination completes. This prevents the LLM from receiving partial results and ensures proper sequencing of operations.

### Common Patterns

#### Pattern 1: Simple Wait State
**Scenario**: Assistant needs to pause execution and wait for external events, user input, or other agents to complete tasks.

**When to use**: The LLM determines it should wait before proceeding, but you want to complete the tool call immediately.

**Implementation** ([`wait`](../delegation_network/crates/wait/src/lib.rs) tool):
```rust
// 1. Request status change
RequestStatusUpdate {
    agent: self.scope.clone(),
    status: Status::Wait {
        reason: WaitReason::WaitingForAgentCoordination {
            coordinating_tool_call_id: tool_call.tool_call.id.clone(),
            coordinating_tool_name: "wait".to_string(),
            target_agent_scope: None,
            user_can_interrupt: true,
        },
    },
    tool_call_id: Some(tool_call.tool_call.id.clone()),
}

// 2. Send tool result (gets queued)
ToolCallStatusUpdate {
    id: tool_call.tool_call.id,
    status: ToolCallStatus::Done {
        result: Ok(ToolCallResult {
            content: "Waiting...".to_string(),
            ui_display_info: UIDisplayInfo {
                collapsed: "Waiting: {reason}".to_string(),
                expanded: Some("Waiting for system input\n\nReason: {reason}".to_string()),
            },
        }),
    },
}
```

**Flow**: Tool completes → Assistant enters `WaitingForAgentCoordination` → User or system can send messages to wake assistant → Assistant resumes with "Waiting..." result included.

#### Pattern 2: Send and Wait for Response
**Scenario**: Send a message to another agent and wait for their response before continuing.

**When to use**: You need synchronous communication with other agents where the assistant should pause until receiving a reply.

**Implementation** ([`send_message`](../delegation_network/crates/send_message/src/lib.rs) tool with `wait: true`):
```rust
// 1. Send the message
AddMessage { agent: target_agent, message: ChatMessage::system(&content) }

// 2. Request wait state
RequestStatusUpdate {
    agent: self.scope.clone(),
    status: Status::Wait {
        reason: WaitReason::WaitingForAgentCoordination {
            coordinating_tool_call_id: tool_call.tool_call.id.clone(),
            coordinating_tool_name: "send_message".to_string(),
            target_agent_scope: Some(target_agent.clone()),
            user_can_interrupt: true,
        },
    },
    tool_call_id: Some(tool_call.tool_call.id.clone()),
}

// 3. Send tool result (gets queued)
ToolCallStatusUpdate {
    id: tool_call.tool_call.id,
    status: ToolCallStatus::Done {
        result: Ok(ToolCallResult {
            content: format!("Message sent to {}, waiting for response", target_agent),
            ui_display_info: UIDisplayInfo {
                collapsed: "Message sent, waiting for response".to_string(),
                expanded: Some(format!("Sent message to {}\n\nWaiting for their response...", target_agent)),
            },
        }),
    },
}
```

**Flow**: Message sent to target agent → Tool completes → Assistant enters `WaitingForAgentCoordination` → Target agent responds → Assistant resumes with "Message sent..." result included.

## Startup Coordination Flow

The Assistant Actor uses startup coordination to ensure all actors are ready before beginning operations:

1. **Initial State**: `WaitingForAllActorsReady`
   - Assistant starts in this state and ignores all input
   - Waits for system to broadcast `AllActorsReady` message

2. **All Actors Ready**: When `AllActorsReady` is received
   - **If LiteLLM available**: Transitions to `WaitingForSystemInput` (ready for operation)
   - **If no LiteLLM**: Transitions to `WaitingForLiteLLM` (wait for LiteLLM startup)

3. **LiteLLM Ready**: When `BaseUrlUpdate` is received while in `WaitingForLiteLLM`
   - Transitions to `WaitingForSystemInput` (now fully ready)

This ensures the assistant doesn't attempt to process requests before all necessary actors and services are available, ensuring proper startup order.

## System Prompt Rendering

The Assistant Actor features a system prompt system that allows any actor to contribute content dynamically.

### How It Works

Actors contribute content to the system prompt by sending `SystemPromptContribution` messages. Each contribution has a unique key, content, priority, and optional section. The assistant collects these contributions, applies user customizations, and renders the final prompt using Jinja2 templates.

### Architecture

#### Key Validation
- Format: `actor_type:contribution_name` (note the colon separator)
- **actor_type** (before colon): lowercase letters, numbers, hyphens, and underscores only
- **contribution_name** (after colon): any characters allowed
- Examples: `file_reader:open_files`, `git-status:branch_info`, `file_interaction:/path/to/file.txt`

#### Content Types

**Text Contributions:**
```rust
SystemPromptContent::Text("Current directory: /home/user/project".to_string())
```

**Data with Template:**
```rust
SystemPromptContent::Data {
    data: json!({"files": [{"name": "main.rs", "lines": 100}]}),
    default_template: r#"Files:
{% for file in data.files -%}
- {{ file.name }} ({{ file.lines }} lines)
{% endfor %}"#.to_string(),
}
```

#### Section Organization
Contributions are organized into sections and sorted by priority within each section (higher priority appears first).

**Built-in Sections:**
- `Identity` - Who the assistant is and its role
- `Context` - Current state and environmental information  
- `Capabilities` - What the assistant can do
- `Guidelines` - Rules and behavioral instructions
- `Tools` - Available tool descriptions and usage
- `Instructions` - Specific task instructions
- `System Context` - Internal system information
- `Custom(name)` - User-defined sections

**Section Priority Order**: Sections appear in the final prompt in a consistent order, with contributions within each section sorted by priority (highest first).

#### User Customization

**Template Overrides:**
```toml
[assistant.system_prompt.overrides]
"file_reader.files" = "{{ data.files | length }} files currently open"
```

**Exclusions:**
```toml
[assistant.system_prompt]
exclude = ["debug.verbose_logging", "experimental.beta_features"]
```

### Template Engine
Uses Jinja2 templating via `minijinja` for flexible template rendering with features like:
- Variable substitution: `{{ variable }}`
- Loops: `{% for item in list %}`
- Filters: `{{ list | length }}`
- Conditionals: `{% if condition %}`

### Rendering Process

1. **Collection**: Assistant receives `SystemPromptContribution` messages from actors
2. **Validation**: Keys are validated, malformed keys are rejected with errors
3. **Organization**: Contributions are grouped by section and sorted by priority
4. **Customization**: User overrides are applied, excluded contributions are filtered out
5. **Template Rendering**: Jinja2 processes data contributions with their templates
6. **Final Assembly**: All sections are combined into the complete system prompt

### Error Handling
- Invalid contributions are logged and ignored
- Template rendering errors fall back to basic system prompt
- Malformed keys are rejected with validation errors

## Implementation Details

### Retry Logic
- Automatic retry on LLM failures with exponential backoff
- Maximum 3 attempts before giving up
- Base delay of 1 second, doubling each retry

### Chat History Management
- Maintains full conversation history
- Supports system, user, assistant, and tool messages
- Automatic message ordering and validation

### Tool Integration
- Discovers available tools via `tools::ToolsAvailable` messages
- Manages tool execution state and results
- Handles tool errors and timeouts

## Example Usage

### Basic Chat Interaction
```rust
// Send user message
Self::broadcast(assistant::AddMessage {
    agent: "assistant-scope".to_string(),
    message: ChatMessage::User(UserChatMessage {
        content: "What files are currently open?".to_string(),
    }),
});
```

### Contributing to System Prompt
```rust
// Any actor can contribute context
Self::broadcast(assistant::SystemPromptContribution {
    agent: "assistant-scope".to_string(),
    key: "shell:current_directory".to_string(),
    content: SystemPromptContent::Text("/home/user/project".to_string()),
    priority: 1000,
    section: Some(Section::Context),
});

// Example with file path in key
Self::broadcast(assistant::SystemPromptContribution {
    agent: "assistant-scope".to_string(),
    key: "file_reader:/path/to/important_file.txt".to_string(),
    content: SystemPromptContent::Text("File content here...".to_string()),
    priority: 500,
    section: Some(Section::Context),
});
```

### Tool Registration
```rust
// Register available tools
Self::broadcast(tools::ToolsAvailable {
    tools: vec![
        Tool {
            name: "execute_bash".to_string(),
            description: "Execute shell commands".to_string(),
            // ... tool definition
        }
    ],
});
```

## Building

To build the Assistant Actor WASM component:

```bash
cargo component build
```

This generates `target/wasm32-wasip1/debug/assistant.wasm` for use in the Wasmind system.

## Testing

Run the comprehensive test suite:

```bash
cargo test
```

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*
