# Hive: Actor-Based LLM Agent Orchestration Library

## Overview
Hive is a Rust library for orchestrating LLM agents using an actor model architecture. In this system:
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

### `/actors` - Actor Implementations
Example WASM actors:
- `assistant/`: LLM assistant actor handling chat interactions
- `execute_bash/`: Command execution actor with bash capabilities

### `/crates/hive_actor_*` - Actor Support Libraries
- `hive_actor_bindings/`: WIT bindings and interface definitions
- `hive_actor_loader/`: Dynamic WASM actor loading
- `hive_actor_utils/`: Common utilities, message types, and macros
- `hive_actor_utils_macros/`: Procedural macros for actor generation

### `/crates/hive_tui` - CLI Interface
Command-line interface and TUI application:
- `main.rs`: Entry point that loads and starts actors
- Currently loads assistant and execute_bash actors by default

### Other Components
- `hive_config/`: Configuration management
- `hive_llm_client/`: LLM client abstractions

## Actor Capabilities
Actors can import various capabilities:
- **messaging**: Broadcast messages to other actors
- **command**: Execute system commands (implemented)
- **http**: HTTP client capabilities (interface defined, not yet implemented)

## Current State
The codebase is in transition:
- Moving from a monolithic architecture to plugin-based actors
- Some legacy code remains commented out in various files
- HTTP/request interface is defined in WIT but needs implementation in the actor manager

## Key Concepts
1. **Actor Lifecycle**: Actors are instantiated with a scope, handle messages, and clean up on destruction
2. **Message Envelopes**: Standardized message format with metadata for routing
3. **Tool System**: Actors can expose tools/capabilities to other actors
4. **WASM Isolation**: Each actor runs in its own WASM sandbox for security and stability

## Building WASM Actors

To build WASM actor components:

1. **Navigate to the actor directory**: You must be in the specific actor's directory (e.g., `/actors/assistant` or `/actors/execute_bash`)
2. **Run the build command**: `cargo component build`

**Important**: The `cargo component build` command must be run from within the actor's directory, NOT from the project root. Each actor is built as a separate WASM component.

Example:
```bash
cd actors/assistant
cargo component build
```

This will generate the WASM component in `target/wasm32-wasip1/debug/` directory.
