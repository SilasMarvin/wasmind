# Wasmind Actor Loader

Dynamic loading and dependency resolution system for Wasmind WASM actor components. This crate handles downloading, building, caching, and loading actors from various sources (local paths, Git repositories, etc.).

[![docs.rs](https://docs.rs/Wasmind_actor_loader/badge.svg)](https://docs.rs/Wasmind_actor_loader)

## What This Crate Does

**Dynamic Actor Loading**: Load WASM actors from multiple source types:
- **Local paths** - Load actors from filesystem directories
- **Git repositories** - Clone and build actors from remote Git sources
- **Cached builds** - Intelligent caching to avoid rebuilding unchanged actors

**Dependency Resolution**: Automatically resolve and build actor dependencies:
- Parse `Cargo.toml` and `Wasmind.toml` manifest files
- Handle complex dependency graphs between actors
- Ensure all required actors are available before loading

**Build Management**: Compile actors using the WebAssembly Component Model:
- Execute `cargo component rustc --crate-type="cdylib"` for each actor
- Manage build artifacts and WASM binary extraction
- Handle build errors and validation

## Language Support

**Currently supports Rust actors only.** The loader uses `cargo component rustc --crate-type="cdylib"` to compile Rust-based actors into WebAssembly components. Support for additional languages (JavaScript, Python, etc.) is planned for future releases.

## Feature Flags

- **`progress-output`** (enabled by default) - Controls whether the loader prints build progress to the console. Useful for CLI tools but can be disabled for library usage:

## Links

- **📚 [Wasmind Book](https://silasmarvin.github.io/wasmind/)** - Complete system documentation
- **⚙️ [Configuration Guide](../Wasmind_config/)** - Actor configuration reference
- **📖 [API Documentation](https://docs.rs/Wasmind_actor_loader)** - Complete API reference
