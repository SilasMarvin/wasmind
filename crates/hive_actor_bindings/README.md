# Hive Actor Bindings

WebAssembly Interface Type (WIT) definitions for Hive actor communication. This crate contains the interface specifications that define how actors interact with the Hive system and each other.

No Rust is exported from this crate! It is designed to be included as a component dependency. Add the following lines to your Cargo.toml:

```
[package.metadata.component.target.dependencies]
"hive:actor" = "0.1" 
```

You can then import these interfaces in your WIT definition. E.G:

```
world your-world {
  import hive:actor/host-info@0.1.0;
  import hive:actor/messaging@0.1.0;
  import hive:actor/http@0.1.0;
  import hive:actor/logger@0.1.0;

  ... your exports
}
```

See the entire interface in `wit/world.wit`

## Interface Overview

The WIT definitions specify how actors:
- Receive and handle messages via `handle-message`
- Access system capabilities (HTTP, commands, spawning)

## Links

- **🔧 [Actor Development Utils](../hive_actor_utils/)** - High-level abstractions for building actors
- **🎭 [Example Actors](../../actors/)** - Reference implementations using these bindings
