# Wasmind: Actor-Based LLM Agent Orchestration Library

## For LLM Developers
This document provides context for LLMs working on the Wasmind codebase.

## Overview
Wasmind is a Rust library for orchestrating LLM agents using an actor model architecture. In this system:
- **Core Library** (`/crates/wasmind`): Actor orchestration engine and platform
- **CLI Binary** (`/crates/wasmind_cli`): Reference implementation and full-featured TUI
- **Example Actors** (`/actors/`): Production-ready WASM components demonstrating various capabilities
- **Actors** are WebAssembly (WASM) components that handle messages and execute specific tasks
- **Agents** are groups of actors working together under a shared scope
- Each actor runs in isolation and communicates via message passing through a central coordinator

## Architecture

### Core Components

1. **WasmindCoordinator**: Central orchestration system (`coordinator.rs`)
   - Monitors actor lifecycle and system health
   - Handles actor spawning and agent coordination
   - Broadcasts system-wide messages and manages replayable messages
   - Coordinates graceful system shutdown

2. **WasmindContext**: System state management (`context.rs`)
   - Maintains actor registry and scope tracking
   - Provides message broadcasting infrastructure
   - Manages actor spawning and agent relationships

3. **Actor System**: Based on WASM Component Model (WIT interface)
   - Actors implement the `actor` interface defined in `world.wit`
   - Each actor has lifecycle methods: constructor, handle-message, destructor
   - Actors communicate through message envelopes containing type, sender ID, scope, and payload

4. **Message Passing**: Broadcast-based communication with coordination
   - All actors receive all messages via tokio broadcast channels
   - Actors filter messages based on scope and message type
   - Central coordinator manages system-level messages (`ActorReady`, `AllActorsReady`, `Exit`)
   - Common message types defined in `wasmind_actor_utils_common_messages`

5. **Scopes**: Hierarchical organization of actors
   - Each actor operates within a scope (6-character string)
   - Agents share a common scope for coordination
   - Starting scope: `000000`
   - Coordinator tracks expected actors per scope for readiness coordination

## Directory Structure

### `/crates/wasmind` - Core Library
The main orchestration library providing:
- `lib.rs`: Main entry point and core error types
- `coordinator.rs`: Central system coordination and lifecycle management
- `context.rs`: System state management and actor registry
- `actors/`: Actor execution traits and manager implementation
  - `manager/`: WASM component instantiation and message routing
  - Actor state management for messaging, HTTP, logging, and agent capabilities
- `scope.rs`: Scope management and hierarchical organization
- `utils.rs`: Utility functions for ID generation and system operations

### `/actors` - Production Actor Implementations
Production-ready WASM actors demonstrating Wasmind's capabilities:

**Core Examples:**
- `assistant/`: LLM chat interface with comprehensive tool calling
- `execute_bash/`: System command execution with full bash capabilities
- `conversation_compaction/`: Conversation history management and compression

**File Interaction:**
- `file_interaction/`: Complete file management system with read/write capabilities

**Advanced Multi-Agent Systems:**
- `delegation_network/`: Hierarchical agent coordination system
  - Tools: `spawn_agent`, `send_message`, `send_manager_message`, `planner`, `wait`, `complete`
  - `delegation_network_coordinator`, `check_health`, `flag_issue`, `report_normal`
  - Demonstrates: Manager → SubManager → Worker patterns with health monitoring

- `code_with_experts/`: Collaborative code editing with expert validation
  - `file_interaction_with_approval`: File operations requiring expert approval
  - `approve`: Expert approval workflow management
  - `request_changes`: Change request and validation system
  - Multi-expert validation before code changes are applied

- `review_plan/`: Plan review and approval workflows
  - `review_plan`: Plan analysis and feedback system
  - `request_plan_review`: Review coordination and management

**Note**: These are production-ready actors that showcase real-world multi-agent coordination patterns.

### `/crates/wasmind_actor_*` - Actor Support Libraries
- `wasmind_actor_bindings/`: WIT bindings and interface definitions
- `wasmind_actor_loader/`: Dynamic WASM actor loading with caching system
- `wasmind_actor_utils/`: Common utilities, message types, and macros
- `wasmind_actor_utils_macros/`: Procedural macros for actor generation
- `wasmind_actor_utils_common_messages/`: Standardized message types for actor communication

### `/crates/wasmind_cli` - Production TUI Application
Full-featured terminal user interface for Wasmind:
- **Interactive TUI**: Complete terminal interface with multiple views
  - `tui/components/chat.rs`: Real-time chat interface with agents
  - `tui/components/dashboard.rs`: System overview and controls
  - `tui/components/graph/`: Real-time agent relationship visualization
  - `tui/components/chat_history.rs`: Conversation history management
- **Example Configurations**: Production-ready setups
  - `example_configs/assistant.toml`: Simple AI assistant
  - `example_configs/code_with_experts.toml`: Expert code validation workflow
  - `example_configs/delegation_network.toml`: Hierarchical agent coordination
- **Commands**: `info`, `clean`, `check` for system management and debugging
- **Configuration Management**: Flexible TOML-based actor configuration with LiteLLM integration

### Other Components
- `wasmind_config/`: Comprehensive configuration management with TOML parsing
- `wasmind_llm_types/`: LLM type definitions and abstractions for model integration

## Actor Capabilities
Actors can import various capabilities through WIT interfaces:
- **messaging**: ✅ Broadcast messages to other actors with scope-based filtering
- **command**: ✅ Execute system commands with full bash capabilities and output capture
- **http**: ✅ HTTP client with automatic retry, exponential backoff, and comprehensive error handling
- **agent**: ✅ Spawn and manage hierarchical agent relationships with health monitoring
- **logger**: ✅ Structured logging with correlation IDs and distributed tracing support
- **host-info**: ✅ Access to host system information and capabilities

## Current Implementation Status
Wasmind is a production-ready platform for building massively parallel AI agent systems:
- ✅ Full WASM-based actor system with coordinated message passing
- ✅ Complete TUI application with chat, dashboard, and graph visualization
- ✅ Production-ready actor examples with real-world coordination patterns
- ✅ LiteLLM integration for model routing and provider flexibility
- ✅ Comprehensive configuration management with TOML support
- ✅ Actor caching system for performance optimization
- ✅ Health monitoring and system coordination
- ✅ Multi-agent workflows: delegation networks, expert validation, approval systems
- ✅ Debugging and troubleshooting tools with structured logging

**Production Note**: Wasmind is complete and production-ready, demonstrated by the comprehensive CLI application and actor ecosystem.

## Key Concepts
1. **Actor Lifecycle**: Actors are instantiated with a scope, send ActorReady signals, handle messages, and coordinate graceful shutdown
2. **Message Envelopes**: Standardized message format with correlation IDs, scope information, and metadata for distributed tracing
3. **Coordinator Pattern**: Central WasmindCoordinator manages system lifecycle, health monitoring, and message replay
4. **Tool System**: Actors expose tools/capabilities that LLMs can call, with automatic serialization and error handling
5. **WASM Isolation**: Each actor runs in its own WASM sandbox with capability-based security
6. **Configuration-Driven**: Complete systems built through TOML configuration without code changes
7. **LiteLLM Integration**: Model-agnostic LLM access through standardized proxy configuration
8. **Scope-Based Organization**: Hierarchical agent coordination with automatic scope management
9. **Production TUI**: Full terminal interface with real-time visualization and interaction capabilities

## Development Context

### Building WASM Actors
To build WASM actor components:

1. **Navigate to the actor directory**: You must be in the specific actor's directory (e.g., `/actors/assistant` or `/actors/delegation_network/crates/spawn_agent`)
2. **Run the build command**: `cargo component build`

**Important**: The `cargo component build` command must be run from within the actor's directory, NOT from the project root. Each actor is built as a separate WASM component.

**Actor Caching**: Wasmind automatically caches compiled actors for performance. Use `wasmind_cli clean` to clear the cache if needed.

Example:
```bash
cd actors/assistant
cargo component build
```

This will generate the WASM component in `target/wasm32-wasip1/debug/` directory.

### Running Example Configurations
Wasmind includes production-ready example configurations:

```bash
# Simple AI assistant
wasmind_cli -c example_configs/assistant.toml

# Code editing with expert validation
wasmind_cli -c example_configs/code_with_experts.toml

# Hierarchical delegation network
wasmind_cli -c example_configs/delegation_network.toml
```

### Configuration Debugging
```bash
# Validate configuration files
wasmind_cli check -c path/to/config.toml

# Debug message flow
WASMIND_LOG=debug wasmind_cli -c config.toml

# System information
wasmind_cli info

# Clear actor cache
wasmind_cli clean
```

### Adding New Actors
**Actor Structure:**
- Each actor: `Cargo.toml`, `Wasmind.toml`, `wit/world.wit`, `src/lib.rs`
- Build: `cargo component build` from actor directory
- Bindings: Auto-generated in `src/bindings.rs`

**Integration:**
- Add to application configuration (TOML files in `example_configs/`)
- Tools defined using `#[derive(wasmind_actor_utils::tools::macros::Tool)]`
- Message types in `wasmind_actor_utils_common_messages`
- Actor capabilities defined in `Wasmind.toml` configuration

**Configuration Integration:**
```toml
[actors.my_actor]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/my_actor" }

[actors.my_actor.config]
model_name = "openai/gpt-4"
custom_param = "value"
```

### Platform Focus
**Production Examples**: 
- **Delegation networks**: Hierarchical manager/worker patterns with health monitoring
- **Expert validation workflows**: Multi-agent code review and approval systems
- **Interactive assistants**: Full-featured AI assistants with tool access
- **File management**: Comprehensive file operations with approval workflows

**Architecture Patterns**:
- Single assistant setups for simple use cases
- Multi-agent validation and approval systems
- Hierarchical delegation with manager/sub-manager/worker relationships
- Real-time collaboration between specialized agents

**Core Achievement**: Production-ready actor orchestration platform with proven scalability

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
