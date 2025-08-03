# Hive: Actor-Based LLM Agent Orchestration Library

## For LLM Developers
This document provides context for LLMs working on the Hive codebase.

## Overview
Hive is a Rust library for orchestrating LLM agents using an actor model architecture. In this system:
- **Core Library** (`/crates/hive`): Actor orchestration engine and platform
- **CLI Binary** (`/crates/hive_cli`): Reference implementation and development interface
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
   - Common message types defined in `hive_actor_utils_common_messages`

3. **Scopes**: Hierarchical organization of actors
   - Each actor operates within a scope (UUID-based)
   - Agents share a common scope for coordination
   - Starting scope: `00000000-0000-0000-0000-000000000000`

## Directory Structure

### `/crates/hive` - Core Library
The main orchestration library providing:
- `hive.rs`: Main entry point with `start_hive()` function
- `actors/`: Actor execution traits and manager implementation
  - `manager/`: WASM component instantiation and message routing
  - `agent.rs`: Agent-level abstractions (currently in transition)
- `scope.rs`: Scope management

### `/actors` - Example Actor Implementations
Sample WASM actors demonstrating Hive's capabilities:

**Basic Examples:**
- `assistant/`: LLM chat interface with tool calling
- `execute_bash/`: System command execution with bash capabilities

**Delegation Network Example:**
- `/actors/delegation_network/`: Hierarchical agent coordination system
- Demonstrates: Manager → SubManager → Worker patterns
- Tools: `spawn_agent`, `send_message`, `send_manager_message`, `planner`, `wait`, `complete`

**Note**: These are examples of what's possible with Hive. The platform supports many different actor architectures and use cases.

### `/crates/hive_actor_*` - Actor Support Libraries
- `hive_actor_bindings/`: WIT bindings and interface definitions
- `hive_actor_loader/`: Dynamic WASM actor loading
- `hive_actor_utils/`: Common utilities, message types, and macros
- `hive_actor_utils_macros/`: Procedural macros for actor generation

### `/crates/hive_cli` - Reference Implementation
Command-line interface and terminal application demonstrating Hive usage:
- `main.rs`: Entry point that loads and starts actors
- `default_config.toml`: Configuration for example actor setup
- Serves as development interface and example of what's possible with Hive

### Other Components
- `hive_config/`: Configuration management
- `hive_llm_client/`: LLM client abstractions

## Actor Capabilities
Actors can import various capabilities through WIT interfaces:
- **messaging**: ✅ Broadcast messages to other actors
- **command**: ✅ Execute system commands with full bash capabilities
- **http**: ✅ HTTP client with automatic retry and exponential backoff
- **agent**: ✅ Spawn and manage hierarchical agent relationships
- **logger**: ✅ Structured logging across the system

## Current Implementation Status
Hive is an active development project building a flexible platform for LLM agent systems:
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
- Each actor: `Cargo.toml`, `Hive.toml`, `wit/world.wit`, `src/lib.rs`
- Build: `cargo component build` from actor directory
- Bindings: Auto-generated in `src/bindings.rs`

**Integration:**
- Add to application configuration as needed
- Tools defined using `#[derive(tools::macros::Tool)]`
- Message types in `hive_actor_utils_common_messages`

### Platform Focus
**Current Examples**: Delegation network demonstrates hierarchical coordination patterns
**Future Potential**: Many more actor types and coordination patterns possible
**Core Goal**: Building flexible actor orchestration capabilities
