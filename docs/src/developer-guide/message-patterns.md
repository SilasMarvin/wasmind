# Message Patterns

Messages are the "language" that actors use to coordinate complex workflows. Your echo actor demonstrated basic message handling - now let's explore the sophisticated communication patterns that make multi-agent systems possible.

## Building on the Echo Actor

Remember your echo actor's simple message handling:

```rust
fn handle_message(&mut self, message: MessageEnvelope) {
    if message.to_scope != self.scope {
        return;
    }
    if let Some(add_message) = Self::parse_as::<AddMessage>(&message) {
        self.handle_chat_message(add_message);
    }
}
```

This was just the beginning. Real actor coordination involves multiple message types, complex routing patterns, and sophisticated workflows.

## Message Structure Deep Dive

Every message in Wasmind uses the same envelope structure:

```rust
// From the WebAssembly interface
record message-envelope {
    id: string,                    // Correlation ID for tracing (e.g., "parent:child")
    message-type: string,          // Unique identifier (e.g., "wasmind.common.tools.ExecuteTool")
    from-actor-id: string,         // Actor ID that sent this message  
    from-scope: scope,             // 6-character scope of the sender
    payload: list<u8>,             // Serialized message data (usually JSON)
}
```

### How Message Routing Actually Works

**Key insight**: All actors receive all broadcast messages. There's no system-level filtering - actors choose which messages to process:

```rust
fn handle_message(&mut self, message: MessageEnvelope) {
    // Actor chooses which messages to handle
    match message.message_type.as_str() {
        "wasmind.common.assistant.AddMessage" => {
            // Handle chat messages
        }
        "wasmind.common.tools.ExecuteTool" => {
            // Handle tool execution requests
        }
        "wasmind.common.actors.AgentSpawned" => {
            // Maybe react to new agents being created
        }
        _ => {
            // Ignore other message types
        }
    }
}
```

This design gives actors complete flexibility in choosing what to listen to, enabling powerful coordination patterns.

## The Message Trait (Optional Convenience)

The `Message` trait is a convenience that makes message handling easier, but it's not required:

```rust
// The convenience trait
pub trait Message: Serialize + DeserializeOwned {
    const MESSAGE_TYPE: &str;
}
```

**Important**: You don't have to implement this trait. You can work directly with the raw message envelope and handle serialization yourself. However, implementing `Message` enables convenient helper methods:

```rust
// With Message trait - convenient parsing
if let Some(add_message) = Self::parse_as::<AddMessage>(&message) {
    // Automatically handles JSON deserialization and type checking
}

// With Message trait - convenient broadcasting  
Self::broadcast_common_message(add_message)?;

// Without Message trait - manual handling
if message.message_type == "wasmind.common.assistant.AddMessage" {
    if let Ok(json_str) = String::from_utf8(message.payload) {
        if let Ok(add_message) = serde_json::from_str::<AddMessage>(&json_str) {
            // Manual parsing
        }
    }
}
```

The macro-generated helper methods (`parse_as` and `broadcast_common_message`) only work with types that implement `Message`, but you can always handle messages manually if preferred.

## Common Message Types

The Wasmind ecosystem includes several common message types that actors frequently use:

### Chat and Conversation Messages

#### `AddMessage` - Chat Interactions
```rust
pub struct AddMessage {
    pub agent: Scope,
    pub message: ChatMessage,  // User, Assistant, System, or Tool message
}

impl Message for AddMessage {
    const MESSAGE_TYPE: &str = "wasmind.common.assistant.AddMessage";
}
```

**Usage pattern**: Add messages to an agent's conversation history.

```rust
// Broadcasting a user message
let user_message = AddMessage {
    agent: target_scope.clone(),
    message: ChatMessage::user("Please analyze this code"),
};
Self::broadcast_common_message(user_message)?;
```

#### `SystemPromptContribution` - Dynamic System Prompts
```rust
pub struct SystemPromptContribution {
    pub agent: Scope,
    pub key: String,           // "file_reader.open_files"
    pub content: SystemPromptContent,
    pub priority: i32,         // Higher = appears earlier
    pub section: Option<Section>,  // Tools, Guidelines, etc.
}

impl Message for SystemPromptContribution {
    const MESSAGE_TYPE: &str = "wasmind.common.assistant.SystemPromptContribution";
}
```

**Usage pattern**: Actors contribute to system prompts dynamically as capabilities change.

```rust
// Tool actor announces its capabilities
let contribution = SystemPromptContribution {
    agent: target_scope,
    key: "execute_bash.usage_guide".to_string(),
    content: SystemPromptContent::Text(BASH_USAGE_GUIDE.to_string()),
    priority: 800,
    section: Some(Section::Tools),
};
Self::broadcast_common_message(contribution)?;
```

### Tool and Capability Messages

#### `ToolsAvailable` - Capability Announcement
```rust
pub struct ToolsAvailable {
    pub tools: Vec<Tool>,  // LLM-compatible tool definitions
}

impl Message for ToolsAvailable {
    const MESSAGE_TYPE: &str = "wasmind.common.tools.ToolsAvailable";
}
```

**Usage pattern**: Tool actors broadcast their capabilities when they start up.

#### `ExecuteTool` - Tool Execution Requests
```rust
pub struct ExecuteTool {
    pub tool_call: ToolCall,           // Function name, arguments, ID
    pub originating_request_id: String, // Links back to the chat request
}

impl Message for ExecuteTool {
    const MESSAGE_TYPE: &str = "wasmind.common.tools.ExecuteToolCall";
}
```

**Usage pattern**: Assistants request tool execution; tool actors respond.

#### `ToolCallStatusUpdate` - Tool Execution Responses
```rust
pub struct ToolCallStatusUpdate {
    pub status: ToolCallStatus,  // Received, AwaitingSystem, Done
    pub id: String,              // Tool call ID
    pub originating_request_id: String,
}

impl Message for ToolCallStatusUpdate {
    const MESSAGE_TYPE: &str = "wasmind.common.tools.ToolCallStatusUpdate";
}
```

### Coordination and Status Messages

#### `StatusUpdate` - Agent State Management
```rust
pub struct StatusUpdate {
    pub status: Status,  // Processing, Wait, Done
}

impl Message for StatusUpdate {
    const MESSAGE_TYPE: &str = "wasmind.common.assistant.StatusUpdate";
}
```

**Usage pattern**: Agents communicate their current state for coordination.

#### `AgentSpawned` - Agent Lifecycle
```rust
pub struct AgentSpawned {
    pub agent_id: Scope,
    pub name: String,                // "Code Reviewer", "Worker Agent"
    pub parent_agent: Option<Scope>,
    pub actors: Vec<String>,         // ["assistant", "execute_bash"]
}

impl Message for AgentSpawned {
    const MESSAGE_TYPE: &str = "wasmind.common.actors.AgentSpawned";
}
```

**Usage pattern**: Announces when new agents are created for coordination.

## Message Patterns in Action

### Pattern 1: Broadcast Communication (One-to-Many)

The simplest pattern - one actor sends a message to all actors:

```rust
// Announce a capability to everyone
let tools_available = ToolsAvailable {
    tools: vec![my_tool_definition],
};
Self::broadcast_common_message(tools_available)?;
```

**Use cases**: 
- Tool actors announcing capabilities
- Status updates
- System-wide notifications

### Pattern 2: Scope-Targeted Communication

While all actors receive messages, you can target specific agents by checking scope:

```rust
fn handle_message(&mut self, message: MessageEnvelope) {
    // Only process messages targeted at our scope
    if message.to_scope == self.scope {
        // Handle messages meant for our agent
    }
    
    // But also listen for global announcements
    if message.message_type == "wasmind.common.tools.ToolsAvailable" {
        // Anyone can announce new tools
    }
}
```

**Use cases**:
- Agent-specific instructions
- Targeted status updates
- Private coordination between specific agents

### Pattern 3: Request-Response with Correlation

Use correlation IDs to link related messages:

```rust
// Tool execution request
let execute = ExecuteTool {
    tool_call: ToolCall {
        id: "call_123".to_string(),
        // ... other fields
    },
    originating_request_id: "req_456".to_string(),
};
Self::broadcast_common_message(execute)?;

// Later, tool responds with same IDs
let response = ToolCallStatusUpdate {
    status: ToolCallStatus::Done { result: Ok(result) },
    id: "call_123".to_string(),
    originating_request_id: "req_456".to_string(),
};
Self::broadcast_common_message(response)?;
```

**Use cases**:
- Tool execution workflows
- Multi-step coordination
- Request tracking and timeouts

### Pattern 4: Event Streaming

Actors can subscribe to event streams by listening to specific message types:

```rust
fn handle_message(&mut self, message: MessageEnvelope) {
    match message.message_type.as_str() {
        "wasmind.common.actors.AgentSpawned" => {
            // React to new agents being created
            if let Some(agent_spawned) = Self::parse_as::<AgentSpawned>(&message) {
                self.on_new_agent_created(agent_spawned);
            }
        }
        "wasmind.common.assistant.StatusUpdate" => {
            // Monitor agent status changes
            if let Some(status_update) = Self::parse_as::<StatusUpdate>(&message) {
                self.track_agent_status(message.from_scope, status_update.status);
            }
        }
        _ => {}
    }
}
```

**Use cases**:
- Monitoring and logging actors
- Dynamic system adaptation
- Coordination supervisors

### Pattern 5: Multi-Agent Workflows

Complex workflows involving multiple agents:

```rust
// Step 1: Coordinator spawns a specialized agent
let new_scope = bindings::wasmind::actor::agent::spawn_agent(
    vec!["code_reviewer".to_string()], 
    "Code Review Agent".to_string()
)?;

// Step 2: Send the agent a task
let task = AddMessage {
    agent: new_scope.clone(),
    message: ChatMessage::user("Please review this code: ..."),
};
Self::broadcast_common_message(task)?;

// Step 3: Listen for completion
fn handle_message(&mut self, message: MessageEnvelope) {
    if message.from_scope == new_scope 
        && message.message_type == "wasmind.common.assistant.StatusUpdate" {
        if let Some(status) = Self::parse_as::<StatusUpdate>(&message) {
            match status.status {
                Status::Done { result } => {
                    // Agent finished, process result
                    self.handle_review_complete(result);
                }
                _ => {}
            }
        }
    }
}
```

## Creating Custom Message Types

You can define your own message types for specialized coordination:

```rust
use serde::{Serialize, Deserialize};
use wasmind_actor_utils::messages::Message;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeReviewRequest {
    pub code: String,
    pub language: String,
    pub reviewer_scope: String,
    pub priority: u8,
}

impl Message for CodeReviewRequest {
    const MESSAGE_TYPE: &str = "mycompany.codereviewer.ReviewRequest";
}

// Usage with the convenience helpers
fn request_code_review(&self, code: String) -> Result<(), serde_json::Error> {
    let request = CodeReviewRequest {
        code,
        language: "rust".to_string(),
        reviewer_scope: self.reviewer_scope.clone(),
        priority: 5,
    };
    Self::broadcast_common_message(request)  // Uses the Message trait
}
```

**Remember**: Implementing `Message` is optional but enables the convenient helper methods. You can always work directly with the raw message envelope and handle serialization manually.

### Custom Message Best Practices

1. **Use reverse DNS naming**: `company.actor.MessageName`
2. **Make messages self-contained**: Include all needed information
3. **Version your messages**: Consider compatibility when changing structure
4. **Include correlation IDs**: For request-response patterns
5. **Add metadata**: Priority, timestamps, scope targeting
6. **Implement `Message` trait**: For convenient helper methods (optional but recommended)

## Advanced Coordination Patterns

### Approval Workflows

```rust
// Multi-step approval with different actors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub action: String,
    pub approvers: Vec<String>,  // Scopes of approval actors
    pub request_id: String,
}

impl Message for ApprovalRequest {
    const MESSAGE_TYPE: &str = "mycompany.approval.Request";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalResponse {
    pub request_id: String,
    pub approved: bool,
    pub approver_scope: String,
    pub reason: Option<String>,
}

impl Message for ApprovalResponse {
    const MESSAGE_TYPE: &str = "mycompany.approval.Response";
}
```

### Dynamic System Reconfiguration

```rust
// Actors can announce new capabilities at runtime
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityUpdate {
    pub actor_id: String,
    pub capabilities_added: Vec<String>,
    pub capabilities_removed: Vec<String>,
}

impl Message for CapabilityUpdate {
    const MESSAGE_TYPE: &str = "mycompany.system.CapabilityUpdate";
}
```

## Message Flow Debugging

Understanding message flow is crucial for debugging:

```rust
fn handle_message(&mut self, message: MessageEnvelope) {
    // Log message flow for debugging
    bindings::wasmind::actor::logger::log(
        bindings::wasmind::actor::logger::LogLevel::Debug,
        &format!(
            "Received message: {} from {} (scope: {}) with ID: {}", 
            message.message_type,
            message.from_actor_id,
            message.from_scope,
            message.id
        ),
    );
    
    // Your message handling logic...
}
```

## Key Takeaways

- **All actors receive all messages** - filtering is done by individual actors, not the system
- **The `Message` trait is optional** - it's a convenience for easier serialization/deserialization and helper methods
- **Message types enable coordination** - actors coordinate by understanding common message schemas
- **Correlation IDs link workflows** - track multi-step processes with unique identifiers
- **Scopes enable agent targeting** - send messages to specific agents while allowing global listening
- **Custom messages enable specialized coordination** - define your own message types for unique workflows
- **Broadcast is powerful** - one message can coordinate many actors simultaneously

## Next Steps

Now that you understand message patterns, you're ready to build sophisticated actors:

### Build Tool Actors
Learn how to create actors that provide capabilities to AI assistants in [Tool Actors](./tool-actors.md).

### Real Examples
See these patterns in action in [Examples](./examples.md) with complete coordination system implementations.

### Testing Message Flows
Learn strategies for testing complex message interactions in [Testing](./testing.md).

Understanding message patterns is the key to building sophisticated multi-agent systems. Messages are not just data transfer - they're the coordination language that enables emergent intelligent behavior from multiple specialized actors working together.
