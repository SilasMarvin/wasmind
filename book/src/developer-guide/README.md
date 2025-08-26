# Developer Guide

Welcome to the Wasmind Developer Guide! This section is for people who want to **build** custom actors and extend Wasmind's capabilities.

## What You'll Learn

In this guide, you'll learn how to:

- **Understand actors as WebAssembly components** and the foundational architecture
- **Build your first actor** from scratch using Rust and WebAssembly
- **Understand message patterns** and how actors communicate effectively
- **Create tool actors** that provide capabilities to AI assistants
- **Test and debug** your actors during development
- **Build complex systems** using coordination patterns

## Who This Guide Is For

This guide assumes you want to:
- ✅ Create custom actors for specific use cases
- ✅ Understand Wasmind's internal architecture  
- ✅ Build tools and capabilities for AI agents
- ✅ Extend Wasmind with new functionality

If you just want to **use existing actors** and configurations, check out the [User Guide](../user-guide/README.md) instead.

## Prerequisites

Before starting, make sure you have completed the **developer installation** from the [Installation](../installation.md#for-developers-building-custom-actors) section.

You should also be familiar with:
- **Basic Rust programming** (you'll be writing Rust code)
- **Wasmind's [Core Concepts](../concepts.md)** (actors, agents, messages, scopes)
- **WebAssembly concepts** (helpful but not required - we'll cover what you need)

## Guide Structure

This guide is organized into focused sections:

### [Actors as WebAssembly Components](./webassembly-components.md)
**Start here!** Understand the WebAssembly component architecture, host-provided capabilities, and the actor interface contract.

### [Building Actors](./building-actors.md)
Learn the fundamentals of creating WebAssembly actors, from project setup to your first working actor.

### [Message Patterns](./message-patterns.md)
Understand how actors communicate through messages and implement common coordination patterns.

### [Tool Actors](./tool-actors.md)
Build actors that provide capabilities to AI assistants, including file operations, web access, and custom tools.

### [Testing](./testing.md)
Learn strategies for testing actors in isolation and within larger systems.

### [Examples](./examples.md)
Walk through complete examples of real-world actors including coordination systems and specialized tools.

### [Reference](./reference.md)
Links to all technical documentation, API references, and message type definitions.

## Development Philosophy

When building with Wasmind, keep these principles in mind:

- **Single Responsibility** - Each actor should do one thing well
- **Message-Driven** - Actors coordinate through structured messages, not shared state
- **Composable** - Actors should work together to create larger capabilities
- **Secure by Default** - Only grant actors the capabilities they actually need

Ready to start building? Let's begin with understanding [Actors as WebAssembly Components](./webassembly-components.md)!