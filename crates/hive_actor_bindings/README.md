# Hive Actor Bindings

WebAssembly Interface Type (WIT) definitions and bindings for Hive actor communication. This crate contains the interface specifications that define how actors interact with the Hive system and each other.

[![docs.rs](https://docs.rs/hive_actor_bindings/badge.svg)](https://docs.rs/hive_actor_bindings)

## What This Crate Contains

**WIT Interface Definitions** (`wit/world.wit`): Core actor interface specifications:
- **Actor Model** - `message-envelope`, stateful `actor` resource, lifecycle methods
- **Messaging System** - Broadcast communication patterns between actors  
- **Capability Imports** - HTTP client, command execution, agent spawning, logging
- **Tool System** - Interface for exposing actor capabilities as callable tools

**Generated Bindings**: Rust bindings are auto-generated from the WIT definitions using `wit-bindgen`. These bindings provide type-safe interfaces for actor development.

## Usage

This crate is primarily used internally by:
- **Actor developers** - Indirectly through `hive_actor_utils` which re-exports the bindings
- **Hive runtime** - For loading and executing WASM actor components
- **Build system** - As a component dependency for actor compilation

**Actor developers** should use [`hive_actor_utils`](../hive_actor_utils/) instead of importing this crate directly, as it provides higher-level abstractions and development tools.

## Interface Overview

The WIT definitions specify how actors:
- Receive and handle messages via `handle-message`
- Access system capabilities (HTTP, commands, spawning)
- Expose tools for LLM function calling
- Maintain state throughout their lifecycle

## Links

- **🔧 [Actor Development Utils](../hive_actor_utils/)** - High-level abstractions for building actors
- **🎭 [Example Actors](../../actors/)** - Reference implementations using these bindings
- **📖 [API Documentation](https://docs.rs/hive_actor_bindings)** - Generated binding documentation
