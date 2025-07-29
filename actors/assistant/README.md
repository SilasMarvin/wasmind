# Assistant Actor

> **⚠️ WIP README**: This documentation is a work in progress and may not be complete.

The Assistant Actor is a conversational AI agent that manages chat interactions, tool execution, and dynamic system prompt generation within the Hive actor system.

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

## Message Handling

The Assistant Actor processes several message types:

### Incoming Messages

#### `assistant::AddMessage`
Adds user or system messages to the pending conversation.
```rust
AddMessage {
    agent: "assistant-scope",
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

### Outgoing Messages

#### `assistant::Response`
Broadcasts LLM responses for other actors to process.

#### `tools::ExecuteTool`
Requests tool execution when the LLM generates tool calls.

#### `assistant::StatusUpdate`
Notifies other actors of state changes (future implementation).

## State Transitions

The Assistant Actor maintains a state machine with the following states:

### `WaitingForSystemOrUser`
**Initial state** - Ready to receive user input or system messages.
- **Triggers**: User message, system message, or initialization
- **Transitions to**: `Processing` when submitting to LLM

### `WaitingForUserInput`
Specifically waiting for user interaction.
- **Triggers**: Tool execution completed, conversation turn finished
- **Transitions to**: `Processing` when user provides input

### `WaitingForLiteLLM`
Waiting for LiteLLM base URL to become available.
- **Triggers**: No base URL available
- **Transitions to**: `WaitingForSystemOrUser` when base URL received

### `Processing`
Actively processing a request with the LLM.
- **Triggers**: `submit()` called with pending messages
- **Transitions to**: 
  - `WaitingForTools` if LLM response contains tool calls
  - `WaitingForUserInput` if response is complete
  - `WaitingForSystemOrUser` on error

### `WaitingForTools`
Waiting for tool execution to complete.
- **Triggers**: LLM response with tool calls
- **Transitions to**: `Processing` when all tools complete

## System Prompt Rendering

The Assistant Actor features a sophisticated system prompt system that allows any actor to contribute content dynamically.

### Architecture

#### Key Validation
- Format: `actor_type.contribution_name`
- Only alphanumeric characters, hyphens, and underscores
- Examples: `file_reader.open_files`, `git-status.branch_info`

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
Contributions are organized into sections (e.g., "context", "tools", "instructions") and sorted by priority within each section (higher priority appears first).

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

### Error Handling
- Invalid contributions are logged and ignored
- Template errors fall back to basic system prompt
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
    key: "shell.current_directory".to_string(),
    content: SystemPromptContent::Text("/home/user/project".to_string()),
    priority: 1000,
    section: Some("context".to_string()),
});
```

### Tool Registration
```rust
// Register available tools
Self::broadcast(tools::ToolsAvailable {
    tools: vec![
        Tool {
            name: "execute_command".to_string(),
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

This generates `target/wasm32-wasip1/debug/assistant.wasm` for use in the Hive system.

## Testing

Run the comprehensive test suite:

```bash
cargo test
```

Tests cover:
- System prompt rendering with all content types
- Message handling and state transitions
- Configuration parsing and validation
- Error handling and edge cases

---

*This README is part of the Hive actor system. For more information, see the main project documentation.*
