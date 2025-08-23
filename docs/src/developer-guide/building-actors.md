# Building Actors

Let's build your first Wasmind actor! We'll create a simple "echo" actor that responds to messages.

## What You'll Build

By the end of this guide, you'll have:
- A working WebAssembly actor that responds to messages
- Understanding of the basic actor structure
- A complete project you can build on

## Prerequisites

Make sure you have:
- Completed the [developer installation](../installation.md#for-developers-building-custom-actors)
- Basic familiarity with Rust
- Understanding of Wasmind's [core concepts](../concepts.md)
- Read [Actors as WebAssembly Components](./webassembly-components.md) to understand the foundational architecture

## Project Setup

Let's create a new actor project:

```bash
# Create a new WebAssembly component project
cargo component new echo_actor
cd echo_actor
```

### Configure Cargo.toml

Edit your `Cargo.toml` to match this structure:

```toml
[package]
name = "echo_actor"
version = "0.1.0"
edition = "2024"
license = "MIT"
description = "A simple echo actor for learning Wasmind"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen-rt = { version = "0.43", features = ["bitflags"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"

wasmind_actor_utils = { git = "https://github.com/SilasMarvin/wasmind", features = ["macros"] }

[package.metadata.component.target.dependencies]
"wasmind:actor" = { git = "https://github.com/SilasMarvin/wasmind" }
```

> **💡 Note**: As covered in [WebAssembly Components](./webassembly-components.md), these macros are conveniences that handle the component plumbing for you.

### Create the Actor Manifest

Every actor **must** have a `Wasmind.toml` manifest file. Create one in your project root:

```toml
# Wasmind.toml
actor_id = "your-namespace:echo_actor"
```

The `actor_id` is a globally unique identifier for your actor type. Use your own namespace (like your GitHub username or organization name).

## Basic Actor Implementation

Now let's implement the actor. Replace the contents of `src/lib.rs`:

```rust
use serde::{Deserialize, Serialize};
use wasmind_actor_utils::{
    common_messages::assistant::{AddMessage, ChatMessage, UserChatMessage},
    messages::Message,
};

// Generated bindings from the WIT interface - created by `cargo component build`
#[allow(warnings)]
mod bindings;

// Our actor's configuration structure
#[derive(Deserialize)]
struct EchoConfig {
    prefix: Option<String>,
}

// Generate the actor trait
wasmind_actor_utils::actors::macros::generate_actor_trait!();

// Our main actor struct
#[derive(wasmind_actor_utils::actors::macros::Actor)]
pub struct EchoActor {
    scope: String,
    config: EchoConfig,
}

impl GeneratedActorTrait for EchoActor {
    fn new(scope: String, config_str: String) -> Self {
        // Parse the configuration from TOML
        let config: EchoConfig = toml::from_str(&config_str)
            .unwrap_or_else(|_| EchoConfig { prefix: None });

        // Use host-provided logging capability
        bindings::wasmind::actor::logger::log(
            bindings::wasmind::actor::logger::LogLevel::Info,
            &format!("EchoActor initialized for scope: {}", scope),
        );

        Self { scope, config }
    }

    fn handle_message(&mut self, message: bindings::exports::wasmind::actor::actor::MessageEnvelope) {
        // Only process messages intended for our scope
        if message.to_scope != self.scope {
            return;
        }

        // Try to parse as a chat message -- we ignore all other messages
        if let Some(add_message) = Self::parse_as::<AddMessage>(&message) {
            self.handle_chat_message(add_message);
        }
    }

    fn destructor(&mut self) {
        bindings::wasmind::actor::logger::log(
            bindings::wasmind::actor::logger::LogLevel::Info,
            "EchoActor shutting down",
        );
    }
}

impl EchoActor {
    fn handle_chat_message(&self, add_message: AddMessage) {
        // Only respond to user messages
        if let ChatMessage::User(user_msg) = add_message.message {
            let prefix = self.config.prefix.as_deref().unwrap_or("Echo:");
            let response_content = format!("{} {}", prefix, user_msg.content);

            // Create a response message
            let response = AddMessage {
                agent: self.scope.clone(),
                message: ChatMessage::system(&response_content),
            };

            // Broadcast the response
            let _ = Self::broadcast_common_message(response);

            bindings::wasmind::actor::logger::log(
                bindings::wasmind::actor::logger::LogLevel::Info,
                &format!("Echoed message: {}", response_content),
            );
        }
    }
}
```

## Understanding the Code

Let's break down what this actor does:

### 1. Configuration Structure
```rust
#[derive(Deserialize)]
struct EchoConfig {
    prefix: Option<String>,
}
```
Actors receive their configuration as a TOML string that's automatically passed to the `new()` function. This actor accepts an optional `prefix` setting.

### 2. Actor Struct and Macros
```rust
#[derive(wasmind_actor_utils::actors::macros::Actor)]
pub struct EchoActor {
    scope: String,
    config: EchoConfig,
}
```
The `#[derive(Actor)]` macro handles the WebAssembly component implementation for you. Every actor has a `scope` - a 6-character string that identifies which agent it belongs to.

### 3. Message Handling
```rust
fn handle_message(&mut self, message: MessageEnvelope) {
    if message.to_scope != self.scope {
        return;
    }
    // Try to parse as a chat message
    if let Some(add_message) = Self::parse_as::<AddMessage>(&message) {
        self.handle_chat_message(add_message);
    }
}
```
Actors receive all broadcast messages and can choose which ones to process. This echo actor only responds to messages with its scope, but actors can listen to any messages they want. Here we parse `AddMessage` - a common message type used for chat interactions.

> **💡 Going deeper**: `AddMessage` is just one of many common message types. Actors can also define custom message types for specialized coordination. See [Message Patterns](./message-patterns.md) for the full ecosystem.

### 4. Broadcasting Responses
```rust
let _ = Self::broadcast_common_message(response);
```
Actors communicate by broadcasting messages to all other actors in the system. The `broadcast_common_message` helper is a convenience for common message types.

> **💡 Going deeper**: Broadcasting is just one communication pattern. Actors can also send messages to specific scopes, implement request-response patterns, and coordinate complex workflows. Explore these patterns in [Message Patterns](./message-patterns.md).

## Building Your Actor

Build the WebAssembly component:

```bash
cargo component build
```

If successful, you'll find your compiled actor at:
```
target/wasm32-wasip1/debug/echo_actor.wasm
```

## Testing Your Actor

Create a simple configuration to test your actor. Create `test_echo.toml`:

```toml
starting_actors = ["echo_actor", "assistant"]

[actors.echo_actor]
source = { path = "." }

[actors.echo_actor.config]
prefix = "🔄"

[actors.assistant]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/assistant" }

[actors.assistant.config]
model_name = "openai/gpt-5-mini"

[[litellm.models]]
model_name = "openai/gpt-5-mini"

[litellm.models.litellm_params]
model = "openai/gpt-5-mini"
api_key = "os.environ/OPENAI_API_KEY"
```

Run it:
```bash
export OPENAI_API_KEY=your_api_key
wasmind_cli -c test_echo.toml
```

When you send a message in the chat, you should see a follow up of a system message respond with the configured prefix!

> **💡 Testing tip**: Notice how your echo actor and the assistant actor work together in the same scope but handle different message types. This demonstrates the foundational pattern of actor composition in Wasmind.

## Next Steps

Congratulations! You've built your first Wasmind actor. Here's what to explore next:

### Learn Message Patterns
Your echo actor uses basic message handling. Learn more sophisticated patterns in [Message Patterns](./message-patterns.md) including custom message types, coordination workflows, and advanced routing.

### Build Tool Actors
Want to create actors that provide capabilities to AI assistants? See [Tool Actors](./tool-actors.md) to learn how to build actors that extend what assistants can do.

### Add Actor Dependencies
Learn how actors can depend on other actors using `Wasmind.toml` manifests in the [Configuration documentation](../../crates/wasmind_config/README.md#the-wasmindtoml-actor-manifest) - enabling complex multi-actor systems that spawn together.

### Real Examples
Explore complete actor implementations in [Examples](./examples.md) including coordination systems and specialized tools.

## Key Takeaways

- **Every actor needs a `Wasmind.toml` manifest** with a unique `actor_id`
- **Actors communicate through message broadcasting** - they don't call each other directly
- **Scope-based routing** ensures messages reach the right actor instances
- **Configuration is automatically passed as TOML** to the actor's `new()` function
- **The `wasmind_actor_utils` macros are conveniences** - actors can be built directly against the WebAssembly interface
- **Message types define the coordination patterns** - common types exist, but you can create custom ones

Your echo actor demonstrates all the fundamental patterns you need to build more sophisticated actors. The simplicity here is intentional - real power comes from combining multiple actors with different capabilities and coordination patterns!
