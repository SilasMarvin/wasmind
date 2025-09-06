# Wasmind Actor Bindings

WebAssembly Interface Type (WIT) definitions for Wasmind actor communication. This crate contains the interface specifications that define how actors interact with the Wasmind system and each other.

[![docs.rs](https://docs.rs/wasmind_actor_bindings/badge.svg)](https://docs.rs/wasmind_actor_bindings)

No Rust is exported from this crate! It is designed to be included as a component dependency. Add the following lines to your Cargo.toml:

```
[package.metadata.component.target.dependencies]
"wasmind:actor" = "0.1" 
```

You can then import these interfaces in your WIT definition. E.G:

```
world your-world {
  import wasmind:actor/host-info@0.1.0;
  import wasmind:actor/messaging@0.1.0;
  import wasmind:actor/http@0.1.0;
  import wasmind:actor/logger@0.1.0;

  ... your exports
}
```

See the entire interface in `wit/world.wit`

## Interface Overview

The WIT definitions specify how actors:
- Receive and handle messages via `handle-message`
- Access system capabilities (HTTP, commands, spawning)

## Links

- **🔧 [Actor Development Utils](https://github.com/silasmarvin/wasmind/tree/main/crates/wasmind_actor_utils/)** - High-level abstractions for building actors
- **🎭 [Example Actors](https://github.com/silasmarvin/wasmind/tree/main/actors/)** - Reference implementations using these bindings
