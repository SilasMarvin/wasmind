# Wasmind: Actor-Based LLM Agent Orchestration Library

## For LLM Developers
This document provides context for LLMs working on the Wasmind codebase.

## Overview
Wasmind is a Rust library for orchestrating LLM agents using an actor model architecture. In this system:
- **Core Library** (`/crates/Wasmind`): Actor orchestration engine and platform
- **CLI Binary** (`/crates/Wasmind_cli`): Reference implementation and development interface
- **Example Actors** (`/actors/`): Sample WASM components demonstrating various capabilities
- **Actors** are WebAssembly (WASM) plugins that handle messages and execute specific tasks
- **Agents** are groups of actors working together under a shared scope
- Each actor runs in isolation and communicates via message passing

## Architecture

### Core Components

1. **Actor System**: Based on WASM Component Model (WIT interface)
   - Actors implement the `actor` interface defined in `world.wit`
   - Each actor has lifecycle methods: constructor, handle-message, destructor
   - Actors communicate through message envelopes containing type, sender ID, scope, and payload

2. **Message Passing**: Broadcast-based communication
   - All actors receive all messages via tokio broadcast channels
   - Actors filter messages based on scope and message type
   - Common message types defined in `Wasmind_actor_utils_common_messages`

3. **Scopes**: Hierarchical organization of actors
   - Each actor operates within a scope (UUID-based)
   - Agents share a common scope for coordination
   - Starting scope: `00000000-0000-0000-0000-000000000000`

## Directory Structure

### `/crates/Wasmind` - Core Library
The main orchestration library providing:
- `Wasmind.rs`: Main entry point with `start_Wasmind()` function
- `actors/`: Actor execution traits and manager implementation
  - `manager/`: WASM component instantiation and message routing
  - `agent.rs`: Agent-level abstractions (currently in transition)
- `scope.rs`: Scope management

### `/actors` - Example Actor Implementations
Sample WASM actors demonstrating Wasmind's capabilities:

**Basic Examples:**
- `assistant/`: LLM chat interface with tool calling
- `execute_bash/`: System command execution with bash capabilities

**Delegation Network Example:**
- `/actors/delegation_network/`: Hierarchical agent coordination system
- Demonstrates: Manager → SubManager → Worker patterns
- Tools: `spawn_agent`, `send_message`, `send_manager_message`, `planner`, `wait`, `complete`

**Note**: These are examples of what's possible with Wasmind. The platform supports many different actor architectures and use cases.

### `/crates/Wasmind_actor_*` - Actor Support Libraries
- `Wasmind_actor_bindings/`: WIT bindings and interface definitions
- `Wasmind_actor_loader/`: Dynamic WASM actor loading
- `Wasmind_actor_utils/`: Common utilities, message types, and macros
- `Wasmind_actor_utils_macros/`: Procedural macros for actor generation

### `/crates/Wasmind_cli` - Reference Implementation
Command-line interface and terminal application demonstrating Wasmind usage:
- `main.rs`: Entry point that loads and starts actors
- `default_config.toml`: Configuration for example actor setup
- Serves as development interface and example of what's possible with Wasmind

### Other Components
- `Wasmind_config/`: Configuration management
- `Wasmind_llm_client/`: LLM client abstractions

## Actor Capabilities
Actors can import various capabilities through WIT interfaces:
- **messaging**: ✅ Broadcast messages to other actors
- **command**: ✅ Execute system commands with full bash capabilities
- **http**: ✅ HTTP client with automatic retry and exponential backoff
- **agent**: ✅ Spawn and manage hierarchical agent relationships
- **logger**: ✅ Structured logging across the system

## Current Implementation Status
Wasmind is an active development project building a flexible platform for LLM agent systems:
- ✅ WASM-based actor system with message passing
- ✅ HTTP client with automatic retry and exponential backoff
- ✅ Command execution capabilities
- ✅ Agent spawning and hierarchical relationships
- ✅ Tool system for actor capabilities
- 🚧 Growing ecosystem of example actors

**Development Note**: Active codebase focused on building the core platform. APIs may evolve as development continues.

## Key Concepts
1. **Actor Lifecycle**: Actors are instantiated with a scope, handle messages, and clean up on destruction
2. **Message Envelopes**: Standardized message format with metadata for routing
3. **Tool System**: Actors can expose tools/capabilities that LLMs can call
4. **WASM Isolation**: Each actor runs in its own WASM sandbox for security and stability
5. **Platform Flexibility**: Supports many different actor architectures and coordination patterns

## Development Context

### Building WASM Actors
To build WASM actor components:

1. **Navigate to the actor directory**: You must be in the specific actor's directory (e.g., `/actors/assistant` or `/actors/delegation_network/crates/spawn_agent`)
2. **Run the build command**: `cargo component build`

**Important**: The `cargo component build` command must be run from within the actor's directory, NOT from the project root. Each actor is built as a separate WASM component.

Example:
```bash
cd actors/assistant
cargo component build
```

This will generate the WASM component in `target/wasm32-wasip1/debug/` directory.

### Adding New Actors
**Actor Structure:**
- Each actor: `Cargo.toml`, `Wasmind.toml`, `wit/world.wit`, `src/lib.rs`
- Build: `cargo component build` from actor directory
- Bindings: Auto-generated in `src/bindings.rs`

**Integration:**
- Add to application configuration as needed
- Tools defined using `#[derive(tools::macros::Tool)]`
- Message types in `Wasmind_actor_utils_common_messages`

### Platform Focus
**Current Examples**: Delegation network demonstrates hierarchical coordination patterns
**Future Potential**: Many more actor types and coordination patterns possible
**Core Goal**: Building flexible actor orchestration capabilities

## Writing Good Comments

### Comment Quality Guidelines

When working on the Wasmind codebase, follow these principles for writing valuable comments:

#### What TO Comment
- **Why, not what**: Explain the reasoning behind non-obvious code decisions
- **Business logic**: Document domain-specific rules or requirements
- **Complex algorithms**: Break down intricate logic into understandable steps
- **API contracts**: Document expected behavior, preconditions, and edge cases
- **TODOs and FIXMEs**: Track known issues and future improvements
- **Workarounds**: Explain why unusual approaches were necessary

#### What NOT to Comment
- **Obvious operations**: Don't describe what the code clearly shows
- **Language constructs**: Avoid explaining basic Rust syntax
- **Simple assignments**: `let x = 5;` doesn't need a comment
- **Standard patterns**: Common iterator chains, error handling, etc.
- **Getters/setters**: Simple property access doesn't need explanation

#### Examples

**Good Comments:**
```rust
// Use exponential backoff to avoid overwhelming the API during transient failures
let delay = Duration::from_millis(100 * (2_u64.pow(attempt)));

// Actor IDs must be unique within a scope to prevent message routing conflicts
if self.scope_actors.contains(&actor_id) {
    return Err(DuplicateActorError);
}

// TODO: Replace this with proper async I/O once tokio 1.35+ is available
let result = std::thread::spawn(move || blocking_operation()).join();
```

**Poor Comments:**
```rust
// Set the port to 8080
let port = 8080;

// Loop through the actors
for actor in actors {
    // Call the method
    actor.handle_message();
}

// Get the length
let len = vec.len();
```

#### Comment Style
- Use `//` for single-line comments
- Use `///` for public API documentation
- Keep comments up-to-date with code changes
- Write in complete sentences with proper grammar
- Focus on clarity and conciseness
