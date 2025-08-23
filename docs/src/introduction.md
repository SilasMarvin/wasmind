# Introduction

Welcome to **Wasmind** – a modular AI agent coordination system for building massively parallel agentic systems.

## What is Wasmind?

Wasmind is an **actor-based system** for building AI agent workflows. Instead of monolithic AI applications, you compose small, focused actors that each handle specific capabilities.

**Actors are WebAssembly components** that typically fall into three categories:
- **Assistant actors** - manage LLM interactions and conversation flow
- **Tool actors** - provide capabilities like file manipulation, code execution, and web access  
- **Coordination actors** - enable complex multi-agent workflows and delegation

**Actors communicate through structured message passing**, enabling coordination at any scale – from simple workflows to networks of thousands of coordinated agents.

## Why Choose Wasmind?

### 🏗️ **Modular by Design**
Build systems from small, focused components rather than monolithic applications. Each actor handles one thing well.

### 🚀 **Massively Parallel**
The actor model naturally supports thousands of concurrent agents with minimal overhead. Scale from simple workflows to complex multi-agent systems.

### 🔒 **Secure & Sandboxed**
WebAssembly provides sandboxed execution with capability-based security. Actors can only access what you explicitly grant them (config for this coming soon).

### 🌐 **Language Independent**
Write actors in Rust, Python, JavaScript, or any language that compiles to WebAssembly components.

### 🔄 **Message-Driven Coordination**
Actors coordinate through structured message passing, making complex multi-agent behaviors easy to reason about and debug.

## What You Can Build

Wasmind enables entirely new architectures that would be impossible to express in traditional client-server models:

- **Hierarchical delegation networks** - managers spawn and coordinate specialized workers
- **Swarms of specialized agents** - thousands of actors working on different parts of a problem
- **Interactive multi-agent systems** - like Claude Code but with manager → sub-manager → worker patterns
- **Collaborative workflows** - agents that review, approve, and coordinate each other's work

## Important Note

> **Wasmind is NOT a Claude Code alternative** – it's the infrastructure that makes projects like Claude Code possible. Our `wasmind_cli` demonstrates how to build Claude Code-style interactions using Wasmind's coordination primitives.

## How This Book is Organized

This book is divided into two main sections:

### 📚 **User Guide**
For people who want to **use** Wasmind configurations and existing actors:
- Getting started with the CLI
- Understanding configurations  
- Working with built-in actors
- Running example systems

### 🛠️ **Developer Guide**  
For people who want to **build** custom actors and extend Wasmind:
- Creating your first actor
- Understanding message patterns
- Building tool actors
- Testing and development workflows

## Prerequisites

To follow along with this book, you should have:
- **Basic command-line familiarity** - you'll be running commands and editing configuration files
- **Completed installation** - see the [Installation](./installation.md) guide for your use case

Ready to get started? First [install Wasmind](./installation.md), then explore the [Core Concepts](./concepts.md) that make Wasmind work.
