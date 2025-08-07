# Hive Core Library

The main orchestration library for Hive's actor-based system. This crate provides the runtime and coordination primitives for loading and managing WebAssembly actor components.

[![docs.rs](https://docs.rs/hive/badge.svg)](https://docs.rs/hive)

## What This Crate Does

The Hive core library handles:
- **Actor Loading**: Dynamically loads WASM actor components and manages their lifecycle
- **Message Orchestration**: Routes messages between actors using broadcast channels
- **Scope Management**: Organizes actors into hierarchical scopes for coordination
- **System Coordination**: Manages actor readiness, spawning, and shutdown across the entire system

This is a library crate for building applications - for conceptual understanding of actors, scopes, and message passing, see the [Hive Book](../../docs/concepts.md).

## Links

- **📚 [Hive Book](../../docs/)** - Complete user and developer guides
- **💻 [hive_cli](../hive_cli/)** - Reference implementation showing how to use this library
- **📖 [API Documentation](https://docs.rs/hive)** - Complete API reference