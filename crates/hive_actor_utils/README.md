# Hive Actor Utilities

Essential utilities and types for building Hive actors. This crate provides the building blocks that actor developers need: message types, tool abstractions, and development macros.

[![docs.rs](https://docs.rs/hive_actor_utils/badge.svg)](https://docs.rs/hive_actor_utils)

## What This Crate Provides

**Message Types**: Standardized message definitions for actor communication:
- **Actor lifecycle**: `ActorReady`, `Exit`, `AllActorsReady`, `AgentSpawned`
- **Assistant coordination**: `StatusUpdate`, `AddMessage`, `Request`, `Response`
- **Tool execution**: `ExecuteTool`, `ToolCallStatusUpdate`, `ToolsAvailable`
- **LLM integration**: `BaseUrlUpdate` and other provider-specific messages

**Tool System**: Abstractions for building tool actors:
- `Tool` trait for implementing tool capabilities
- Message handling patterns for tool execution
- Result types for tool responses and UI display

**Development Macros** (with `macros` feature):
- `#[derive(Tool)]` - Auto-generate tool actor implementations
- `#[derive(Actor)]` - Generate actor boilerplate and message handling

**Core Constants**: `STARTING_SCOPE` and other system-wide identifiers

## Usage

Add to your actor's `Cargo.toml`:
```toml
[dependencies]
hive_actor_utils = { version = "0.1.0", features = ["macros"] }
```

**Building a tool actor**:
```rust
use hive_actor_utils::tools::Tool;
use hive_actor_utils::common_messages::tools::ExecuteTool;

#[derive(Tool)]
pub struct MyTool {
    // tool state
}

impl Tool for MyTool {
    fn new(scope: String, config: String) -> Self { /* ... */ }
    fn handle_call(&mut self, input: ExecuteTool) { /* ... */ }
}
```

**Using common message types**:
```rust
use hive_actor_utils::common_messages::{actors::ActorReady, assistant::StatusUpdate};
// Send standardized messages between actors
```

## Features

- `macros` - Enables procedural macros for actor generation (recommended)

## Links

- **📚 [Actor Development Guide](../../docs/developer-guide/building-actors.md)** - Complete tutorial for building actors
- **🎭 [Example Actors](../../actors/)** - Reference implementations using these utilities
- **📖 [API Documentation](https://docs.rs/hive_actor_utils)** - Complete API reference