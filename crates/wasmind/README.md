# Wasmind Core Library

The main orchestration library for Wasmind's actor-based system. This crate provides the runtime and coordination primitives for loading and managing WebAssembly actor components.

[![docs.rs](https://docs.rs/wasmind/badge.svg)](https://docs.rs/wasmind)

## What This Crate Does

The Wasmind core library handles:
- **Actor Loading**: Dynamically loads WASM actor components and manages their lifecycle
- **Message Orchestration**: Routes messages between actors using broadcast channels
- **Scope Management**: Organizes actors into hierarchical scopes for coordination
- **System Coordination**: Manages actor readiness, spawning, and shutdown across the entire system

This is a library crate for building applications - for conceptual understanding of actors, scopes, and message passing, see the [Wasmind Book](https://silasmarvin.github.io/wasmind/concepts.html).

## Links

- **📚 [wasmind Book](https://silasmarvin.github.io/wasmind/)** - Complete user and developer guides
- **💻 [wasmind_cli](https://github.com/silasmarvin/wasmind/tree/main/crates/wasmind_cli/)** - Reference implementation showing how to use this library
- **📖 [API Documentation](https://docs.rs/wasmind)** - Complete API reference