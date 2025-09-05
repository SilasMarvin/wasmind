# Actors as WebAssembly Components

Before building your first actor, it's crucial to understand what you're actually creating: **WebAssembly components that run in a sandboxed environment with controlled access to host capabilities.**

This isn't just "Rust code with some macros" - you're building self-contained, portable components that interact with the Wasmind host through a well-defined interface.

## What Are WebAssembly Components?

<a href="https://component-model.bytecodealliance.org/introduction.html" target="_blank">WebAssembly Components</a> are a new standard for building composable, portable modules that can run anywhere. Think of them as:

- **Sandboxed by default** - Can only access capabilities explicitly granted by the host
- **Language agnostic** - Can be built in Rust, JavaScript, Python, or any WASM-capable language
- **Interface-driven** - Communicate through strictly typed interfaces, not shared memory
- **Portable** - Run identically across different operating systems and architectures

In Wasmind, every actor is a WebAssembly component that:
1. **Imports** host-provided capabilities (logging, HTTP, messaging, etc.)
2. **Exports** an actor implementation that handles messages
3. **Communicates** through structured message passing

## The Actor Interface Contract

Every Wasmind actor must implement the `actor` interface defined in Wasmind's [world.wit](https://github.com/SilasMarvin/wasmind/blob/main/crates/wasmind_actor_bindings/wit/world.wit). WIT (WebAssembly Interface Types) is the interface definition language that specifies how components communicate - think of it as similar to Protocol Buffers or GraphQL schemas. Here's what that looks like:

```wit
// From world.wit - the core actor interface
resource actor {
    /// Called when the actor is created
    constructor(scope: scope, config: string);
    
    /// Called for every message broadcast in the system
    handle-message: func(message: message-envelope);
    
    /// Called when the actor is shutting down
    destructor: func();
}
```

**Key insight**: These three functions are the ONLY way the host can interact with your actor. Everything else happens through these entry points.

### Message Envelope Structure

All communication uses this standardized envelope:

```wit
record message-envelope {
    id: string,                    // Correlation ID for tracing
    message-type: string,          // Unique identifier for message type
    from-actor-id: string,         // Who sent this message
    from-scope: scope,             // Which agent sent this message
    payload: list<u8>,             // Serialized message data
}
```

**Scope** is a 6-character string that identifies which agent an actor belongs to. This enables Wasmind's multi-agent coordination - actors in different scopes represent different agents working on different tasks. For example, you might have one agent (scope `agent1`) handling user questions while another agent (scope `agent2`) processes files in the background.

## Host-Provided Capabilities

The Wasmind host provides these capabilities to all actors through imports:

### 🗣️ **Messaging**
```wit
interface messaging {
    broadcast: func(message-type: string, payload: list<u8>);
}
```
How actors communicate with each other - no direct function calls, only message passing.

Note that when called this function creates a `MessageEnvelope` with the `from-scope` as the actors scope and a random 6-character id for the message and broadcasts it to all actors.

### 📝 **Logging**
```wit
interface logger {
    enum log-level { debug, info, warn, error }
    log: func(level: log-level, message: string);
}
```
Structured logging that integrates with the host's logging system.

### 🌐 **HTTP Requests**
```wit
interface http {
    record headers {
        headers: list<tuple<string, string>>
    }
    
    variant request-error {
        network-error(string),
        timeout,
        invalid-url(string),
        builder-error(string),
    }
    
    record response {
        status: u16,
        headers: headers,
        body: list<u8>,
    }
    
    resource request {
        constructor(method: string, url: string);
        header: func(key: string, value: string) -> request;
        headers: func(headers: headers) -> request;
        body: func(body: list<u8>) -> request;
        timeout: func(seconds: u32) -> request;
        retry: func(max-attempts: u32, base-delay-ms: u64) -> request;
        retry-on-status-codes: func(codes: list<u16>) -> request;
        send: func() -> result<response, request-error>;
    }
}
```
Full HTTP client with retry logic, timeouts, error handling, and configurable retry status codes.

### ⚡ **Command Execution**
```wit
interface command {
    variant exit-status {
        exited(u8),
        signaled(u8),
        failed-to-start(string),
        timeout-expired,
    }

    record command-output {
        stdout: list<u8>,
        stderr: list<u8>,
        status: exit-status,
        stdout-truncated: bool,
        stderr-truncated: bool,
    }

    resource cmd {
        constructor(command: string);
        args: func(args: list<string>) -> cmd;
        current-dir: func(dir: string) -> cmd;
        timeout: func(seconds: u32) -> cmd;
        max-output-bytes: func(bytes: u32) -> cmd;
        env: func(key: string, value: string) -> cmd;
        env-clear: func() -> cmd;
        run: func() -> result<command-output, string>;
    }
}
```
Execute system commands with fine-grained control over execution environment, output limits, and environment variables.

### 🏗️ **Agent Management**
```wit
interface agent {
    spawn-agent: func(actor-ids: list<string>, agent-name: string) -> result<scope, string>;
    get-parent-scope: func() -> option<scope>;
    get-parent-scope-of: func(scope: scope) -> option<scope>;
}
```
Spawn new agents and navigate the agent hierarchy. Query parent relationships for any scope.

### 💻 **Host Information**
```wit
interface host-info {
    record os-info {
        os: string,
        arch: string,
    }
    
    get-host-working-directory: func() -> string;
    get-host-os-info: func() -> os-info;
}
```
Access to real host environment information including OS type and architecture.

## The Complete World Definition

The `actor-world` [brings it all together](https://component-model.bytecodealliance.org/design/worlds.html):

```wit
world actor-world {
    // What the host provides to actors
    import messaging;
    import command;
    import http;
    import logger;
    import agent;
    import host-info;

    // What actors must provide to the host
    export actor;
}
```

This defines the complete contract: actors can use any imported capability and must export an `actor` implementation.

## The Convenience of Macros

The `wasmind_actor_utils` crate provides optional macros that make building actors much simpler:

```rust
// With convenience macros - clean and simple
#[derive(wasmind_actor_utils::actors::macros::Actor)]
pub struct MyActor {
    scope: String,
}

impl GeneratedActorTrait for MyActor {
    fn new(scope: String, config: String) -> Self {
        Self { scope }
    }
    
    fn handle_message(&mut self, message: MessageEnvelope) {
        // Your actual logic here
    }
    
    fn destructor(&mut self) {}
}
```

These macros handle all the WebAssembly component plumbing for you - connecting your Rust code to the WebAssembly interface, managing the component lifecycle, and handling message serialization.

**Important**: These macros are conveniences, not requirements. You could implement the WebAssembly component interface directly if needed, but the macros make development much more pleasant by letting you focus on your actor's logic rather than low-level details. If you're curious about what the macros do or want to implement the interface yourself, check out the [macro source code](https://github.com/SilasMarvin/wasmind/blob/main/crates/wasmind_actor_utils_macros/src/lib.rs).

## Security and Sandboxing

This WebAssembly component architecture enables powerful security capabilities:

**Current State:**
- **Memory isolation** - actors can't access each other's memory
- **Interface-controlled access** - actors can only use explicitly imported capabilities
- **Message-based communication** - no direct function calls between actors

**Planned Security Features:**
- **File system restrictions** - fine-grained control over which files/directories actors can access
- **Command execution limits** - restrict which system commands can be executed
- **Resource limits** - CPU, memory, and execution time controls
- **Network access controls** - granular permissions for HTTP requests

**Current Capabilities:**
Actors currently have access to:
- Full HTTP client functionality
- System command execution with environment control
- Host file system (restrictions planned)
- Structured logging
- Message broadcasting
- Agent spawning and coordination

The sandboxing foundation is in place through WebAssembly's memory isolation and interface-based capability system, with enhanced restrictions coming in future releases.

## Key Takeaways

Understanding actors as WebAssembly components explains:

- **Why message passing is required** - actors are isolated and can't share memory
- **Where capabilities come from** - the host provides them through imports
- **How sandboxing works** - actors only have access to explicitly granted capabilities
- **Why the interface is strictly typed** - WIT ensures type safety across the component boundary
- **How portability is achieved** - the same component runs identically anywhere

When you build an actor, you're creating a portable, sandboxed component that can run in any Wasmind host environment while only accessing the capabilities it actually needs.

## Next Steps

Now that you understand the WebAssembly foundation, you're ready to [build your first actor](./building-actors.md) with full knowledge of what's happening under the hood.

The macros and utilities exist to make this easy, but you now understand they're conveniences built on top of a robust, standardized component interface.
