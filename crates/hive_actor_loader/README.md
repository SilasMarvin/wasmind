# Hive Actor Loader

Dynamic loading and dependency resolution system for Hive WASM actor components. This crate handles downloading, building, caching, and loading actors from various sources (local paths, Git repositories, etc.).

[![docs.rs](https://docs.rs/hive_actor_loader/badge.svg)](https://docs.rs/hive_actor_loader)

## What This Crate Does

**Dynamic Actor Loading**: Load WASM actors from multiple source types:
- **Local paths** - Load actors from filesystem directories
- **Git repositories** - Clone and build actors from remote Git sources
- **Cached builds** - Intelligent caching to avoid rebuilding unchanged actors

**Dependency Resolution**: Automatically resolve and build actor dependencies:
- Parse `Cargo.toml` and `Hive.toml` manifest files
- Handle complex dependency graphs between actors
- Ensure all required actors are available before loading

**Build Management**: Compile actors using the WebAssembly Component Model:
- Execute `cargo component build` for each actor
- Manage build artifacts and WASM binary extraction
- Handle build errors and validation

## Usage

This crate is primarily used by the Hive core library and shouldn't need direct usage in most cases:

```rust
use hive_actor_loader::ActorLoader;
use hive_config::{Actor, ActorOverride};

let loader = ActorLoader::new(None)?; // Uses default cache directory
let loaded_actors = loader.load_actors(actors, overrides).await?;
```

The `LoadedActor` struct contains the compiled WASM binary, configuration, and metadata needed by the Hive runtime.

## Architecture

- **ActorLoader** - Main interface for loading actors with caching
- **DependencyResolver** - Handles dependency analysis and resolution
- **LoadedActor** - Represents a fully loaded actor with WASM binary and config
- **Caching system** - Avoids rebuilding actors when sources haven't changed

## Links

- **📚 [Hive Book](../../docs/)** - Complete system documentation
- **⚙️ [Configuration Guide](../hive_config/)** - Actor configuration reference
- **📖 [API Documentation](https://docs.rs/hive_actor_loader)** - Complete API reference