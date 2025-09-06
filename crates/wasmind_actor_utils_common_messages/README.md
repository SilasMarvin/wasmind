# Wasmind Actor Utils Common Messages

Common message types for inter-actor communication in the Wasmind actor system. This crate provides a collection of standardized message types that make building actors easier.

[![docs.rs](https://docs.rs/wasmind_actor_utils_common_messages/badge.svg)](https://docs.rs/wasmind_actor_utils_common_messages)

## Important Notes

**If you're building a Rust actor, use [`Wasmind_actor_utils`](../Wasmind_actor_utils/) instead of this crate directly.** This crate is re-exported from `Wasmind_actor_utils` for convenience.

**This is not the "end all be all" of messages in Wasmind.** Wasmind can pass any message type that can be represented as `Vec<u8>` bytes - this includes images, binary data, custom formats, literally anything. These common messages are just JSON-serialized text messages that provide convenient, standardized communication patterns for actors.

## Message Modules

- **`actors`** - Core actor lifecycle messages (`ActorReady`, `Exit`, `AgentSpawned`, etc.)
- **`assistant`** - LLM assistant communication (`Request`, `Response`, `StatusUpdate`, etc.)  
- **`tools`** - Tool execution coordination (`ExecuteTool`, `ToolCallStatusUpdate`, etc.)
- **`litellm`** - LiteLLM integration messages (`BaseUrlUpdate`)

## Links

- **[Wasmind_actor_utils](../Wasmind_actor_utils/)** - Main utilities crate for Rust actor development
- **[Wasmind_llm_types](../Wasmind_llm_types/)** - LLM type definitions used by these messages
