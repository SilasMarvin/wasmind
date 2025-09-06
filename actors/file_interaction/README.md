# File Interaction

*File reading and editing workspace for Wasmind actors*

This workspace provides file interaction capabilities for AI agents through a clean separation of library and actor components.

## Architecture

This workspace contains two crates:
- **`file_interaction`** - Pure Rust library with all the file manipulation logic
- **`file_interaction_actor`** - WASM actor wrapper that provides tools to AI agents

The library can be used independently by other Rust projects, while the actor provides the Wasmind integration.

## Usage

For detailed configuration, usage, and implementation information, see the [File Interaction Actor README](crates/file_interaction_actor/README.md).

## Building

Build the entire workspace:

```bash
cargo build
```

Build just the WASM actor:

```bash
cd crates/file_interaction_actor
cargo component build
```

## Testing

Run the test suite:

```bash
cargo test
```

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*
