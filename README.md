<div align="center">
   <picture>
     <source media="(prefers-color-scheme: dark)" srcset="">
     <source media="(prefers-color-scheme: light)" srcset="">
     <img alt="Logo" src="" width="520">
   </picture>
</div>

<p align="center">
   <p align="center"><b>A modular AI agent coordination system for building massively parallel agentic systems</b></p>
</p>

<p align="center">
| <a href="https://silasmarvin.github.io/wasmind/"><b>Documentation</b></a> | <a href="#what-is-wasmind"><b>Why Wasmind</b></a> | <a href="#faq"><b>FAQ</b></a> |
</p>

<div align="center">
  <img src="wasmind-diagram.png" alt="Wasmind Architecture: Orchestrator coordinating multiple agents with message passing" width="600">
</div>

---

## Getting Started

**Want to try Wasmind?** Check out [wasmind_cli](crates/wasmind_cli/) - a CLI application built with Wasmind that demonstrates actor-based AI development workflows (including Claude Code-style interactions).

**Want to build with Wasmind?** Continue reading or jump to the [Developer Guide](https://silasmarvin.github.io/wasmind/developer-guide/) to start building your own actors and systems.

## What is Wasmind?

Wasmind is an **actor-based system** for building AI agent workflows. Instead of monolithic AI applications, you compose small, focused actors that each handle specific capabilities.

**Actors are WebAssembly components** - they can do anything, but typically fall into three categories:
- **Assistant actors** - manage LLM interactions and conversation flow
- **Tool actors** - provide capabilities like file manipulation, code execution, and web access  
- **Coordination actors** - enable complex multi-agent workflows and delegation

**Actors communicate through structured message passing**, enabling coordination at any scale - from simple workflows to networks of thousands of coordinated agents.

> **Important**: Wasmind is NOT a Claude Code alternative—it's the infrastructure that makes projects like Claude Code possible. Our `wasmind_cli` demonstrates how to build Claude Code-style interactions using Wasmind's coordination primitives.

## What You Can Build with Wasmind

Wasmind can be used for anything but is best at building massively parallel multi-agent systems.

**Current Demos:**
- **[Delegation Network](actors/delegation_network/)** - Hierarchical multi-agent coordination system for spawning and managing specialized AI agents (think Claude Code but manager -> sub_manager -> worker agent relations).

TODO: Add demo 

Demo's can be ran with the **[wasmind_cli](crates/wasmind_cli/)**

## Repository Structure

```
/actors/           # Example actors and demos

/crates/           # Core Wasmind system libraries
├── Wasmind/          # Main coordination library  
├── Wasmind_cli/      # Command-line interface
├── Wasmind_config/   # Configuration system
├── Wasmind_actor_loader/     # Actor loading and dependency resolution
├── Wasmind_actor_utils/      # Utilities for building Rust actors
├── Wasmind_actor_utils_common_messages/  # Common shared message types
├── Wasmind_actor_utils_macros/   # Macros for Rust actor development
├── Wasmind_actor_bindings/   # WASM component definition
└── Wasmind_llm_types/        # Common LLM API request types
```

For detailed information about specific actors, see their individual READMEs.

## Documentation

- **📚 [Wasmind Book](https://silasmarvin.github.io/wasmind/)** - Comprehensive user and developer guides
- **⚙️ [Configuration Guide](crates/wasmind_config/README.md)** - Complete configuration reference
- **🎭 [Example Actor Documentation](actors/)** - Individual actor guides and APIs
- **💻 [CLI Documentation](crates/wasmind_cli/README.md)** - Command-line interface guide

## Contributing

We welcome contributions to Wasmind! Whether you're building new actors, improving the core system, or have ideas for new features:

- **🐛 Found a bug?** [Open an issue](https://github.com/SilasMarvin/Wasmind/issues)
- **💡 Have a feature idea?** [Start a discussion](https://github.com/SilasMarvin/Wasmind/issues)
- **🛠️ Want to contribute code?** See our [Developer Guide](https://silasmarvin.github.io/wasmind/developer-guide/) to get started

All contributions, big and small, are appreciated!

## FAQ

### How does Wasmind compare to MCP?

MCP (Model Context Protocol) provides a standardized way for AI assistants to connect to external tools and data sources. It's designed for a client-server model where a single AI assistant connects to multiple tool servers.

Wasmind is fundamentally different - it's a full actor-based coordination system that enables:
- **Multi-agent hierarchies**: Agents can spawn and coordinate other agents, creating delegation networks (manager → sub-manager → worker patterns)
- **Peer-to-peer coordination**: Actors communicate directly without going through a central assistant
- **Stateful actors**: Each actor maintains its own state and lifecycle, enabling long-running workflows
- **Massive parallelism**: Thousands of actors can work concurrently on different parts of a problem
- **AND MORE**: ...

MCP is great for "one assistant, many tools" architectures. Wasmind enables entirely new architectures like swarms of specialized agents, hierarchical delegation networks, and massively parallel problem-solving systems that would be impossible to express in MCP's client-server model.

**Bonus**: MCP can actually be wrapped as a Wasmind actor! This enables using an MCP tool in Wasmind: ([TODO: MCP actor implementation](actors/mcp_client/))

### Why actors?

Actors provide natural isolation, parallelism, and fault tolerance. Each actor maintains its own state and communicates only through message passing, making it easy to reason about complex systems. This model scales from simple workflows to thousands of concurrent agents without changing the programming model.

### Why WebAssembly component actors?

WebAssembly components give us:
- **Language independence** - write actors in Rust, Python, JavaScript, or any language that compiles to WASM
- **Security** - sandboxed execution with capability-based security
- **Portability** - run the same actors anywhere WASM runs
- **Performance** - near-native execution speed with minimal overhead
- **Composability** - link actors together using standard component model interfaces

### Can I use Wasmind without the CLI?

Yes! The CLI (`wasmind_cli`) is just one example of what you can build with Wasmind. The core library (`Wasmind`) can be embedded in any Rust application. You can build web services, desktop apps, or any system that needs actor-based coordination.

### What makes Wasmind good for "massively parallel" systems?

Wasmind's actor model naturally supports thousands of concurrent actors with minimal overhead. The scope system enables hierarchical coordination, message passing is async by default, and WebAssembly provides lightweight isolation. See our demo with 1000+ coordinated agents: TODO.

### Do I need to know Rust to use Wasmind?

To use `wasmind_cli` and existing actors, no. To build new actors, no - actors can be written in any language that compiles to WebAssembly components but we currently only have friendly SDKs for Rust. We're working on SDKs for other languages.

## License

MIT License - see [LICENSE](LICENSE) for details.

---
