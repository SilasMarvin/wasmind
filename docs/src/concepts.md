# Core Concepts

Understanding Wasmind requires grasping four key concepts: **Actors**, **Agents**, **Messages**, and **Scopes**. These work together to create a flexible system for building multi-agent AI workflows.

TODO: Diagram!

## Actors

**Actors are the basic building blocks** of Wasmind. Each actor is a WebAssembly component that:

- **Handles specific capabilities** - file manipulation, LLM interaction, command execution, etc.
- **Runs in isolation** - sandboxed execution with only the capabilities you grant
- **Communicates via messages** - no shared state, only message passing
- **Has a lifecycle** - constructor, message handling, destructor
- **Has an attached scope** - each actor has a scope that is attached to all messages they broadcast

### Actor Types

Actors can do anything but are typically broken down into one of three categories:

**Assistant Actors**: Handle LLM interactions and conversation flow
```
Example: assistant, conversation_compaction
Purpose: Manage AI conversations, summarize context, route requests
```

**Tool Actors**: Expose tools to assistant actors
```
Example: execute_bash, file_interaction
Purpose: Execute commands, read/write files, etc... 
```

**Coordination Actors**: Enable complex multi-agent workflows
```  
Example: spawn_agent, send_message
Purpose: Create agent hierarchies, coordinate workflows, delegate tasks
```

## Agents

**Agents are groups of actors working together** under a shared scope. Think of an agent as a "team" of specialized actors that coordinate to accomplish larger goals.

```
Agent = Collection of Actors + Shared Scope + Common Purpose

Example Agent:
├── assistant (conversation management)
├── execute_bash (command execution) 
├── file_interaction (file operations)
└── send_message (coordination)
```

Key properties of agents:
- **Shared scope** - all actors in an agent share the same scope
- **Message coordination** - actors in an agent can communicate and coordinate their actions
- **Hierarchical** - agents can spawn other agents

## Messages

**Messages are how actors communicate**. All communication in Wasmind happens through structured message passing.

### Message Structure
```rust
Message Envelope {
    message_type: String,    // What kind of message this is
    sender_id: String,       // Who sent it
    scope: String,           // Which scope it belongs to  
    payload: Vec<u8>,        // The actual message content
}
```

### Message Flow
1. **Broadcast model** - all actors receive all messages via tokio broadcast channels
2. **Filtering** - actors filter messages based on scope and message type
3. **Handling** - actors process messages they're interested in

### Common Message Types
```
"ExecuteToolCall"       - Request to execute a specific tool
"AssistantResponse"     - Response from an LLM
"AddMessage"            - Message input to an assistant
```

Most message `payload`s are JSON strings but messages can store anything! Actors typically have a predefined set of messages they look for. For example the `execute_bash` actor listens for `ExecuteToolCall` messages.

While we provide a list of commonly used messages in [wasmind_actor_utils_common_messages](https://github.com/SilasMarvin/wasmind/tree/main/crates/wasmind_actor_utils_common_messages), it is common for actors to broadcast and listen for their own unique messages.

## Scopes

**Scopes provide hierarchical organization** and coordination boundaries for actors and agents.

### Scope Hierarchy
```
Root Scope: 000000
├── Agent A Scope: a1b2c3...
│   ├── Assistant Actor
│   └── Tool Actors
├── Agent B Scope: e5f6g7-...
│   ├── Manager Actor
│   └── Worker Actors
└── Agent C Scope: i9jk12-...
    └── Coordination Actors
```

A scope is a unique 6-character string that all actors spawned in a scope are given upon initialization. It is common for actors to only listen for messages sent within their scope. For instance, the `execute_bash` actor only listens for `ExecuteToolCall` messages sent in its scope. This way it doesn't pick up tool calls from assistants in other scopes.

Actors receive messages from every scope! A health monitoring actor may choose to listen to `AssistantResponse`s from every scope and analyze their contents to ensure all assistants in the network are performing well.

Scopes are nothing more than an identifier sent to an actor when it first spawns, and attached to all messages broadcast from actors.

## How It All Works Together

Let's trace through a concrete example with message types and scopes:

1. **Human sends input** 
   - Message: `AddMessage` with payload "Help me write a Python script"
   - Scope: `000000` (root scope)
   - Broadcast to ALL actors in the system

2. **Assistant actor filters and processes**
   - Receives the message (along with all other actors)
   - Filters: Only processes `AddMessage` in scope `000000`
   - Decides it needs file manipulation capabilities

3. **Assistant requests tool execution**
   - Message: `ExecuteToolCall` with payload containing tool details
   - Scope: `000000` (same scope as assistant)
   - Broadcast to ALL actors

4. **Tool actor filters and executes**
   - `file_interaction` actor receives the message
   - Filters: Only processes `ExecuteToolCall` in scope `000000`
   - Executes file operation, sends response
   - Message: `ToolCallResponse` with file contents
   - Scope: `000000`

5. **Assistant formulates response**
   - Receives `ToolCallResponse` (filters for its scope)
   - Generates Python script based on context
   - Message: `AssistantResponse` with the script
   - Scope: `000000`

6. **For complex tasks - spawning sub-agents**
   - Coordinator spawns new agent with scope `abc123`
   - New agent's actors initialized with scope `abc123`
   - Messages within new agent use scope `abc123`
   - Parent agent can still monitor by listening to all scopes

7. **Cross-scope monitoring**
   - Health monitor listens to `AssistantResponse` from ALL scopes
   - Doesn't filter by scope - sees everything
   - Can detect issues across entire system

**Key insight**: Every actor sees every message. Scopes are just metadata for filtering - they enable coordination boundaries without limiting visibility.

## Next Steps

Now that you understand the core concepts, you can:
- **Use Wasmind** → Start with the [User Guide](./user-guide/README.md)
- **Build with Wasmind** → Jump to the [Developer Guide](./developer-guide/README.md)
