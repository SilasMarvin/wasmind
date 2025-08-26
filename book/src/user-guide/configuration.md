# Configuration

This guide shows you how to customize Wasmind configurations for your needs. Whether you're starting fresh or building on an existing setup, you'll learn how to add actors, configure models, and create powerful multi-agent systems.

## Building on Your First Configuration

Let's start with a basic assistant and gradually add more capabilities. If you're following from Getting Started, you can enhance your existing configuration. If you're starting here, you can create these configurations from scratch.

## Adding Tool Actors

The basic assistant can only chat - let's give it some tools. Here's how to add bash execution capability:

```toml
# Enhanced Assistant Configuration
starting_actors = ["assistant", "execute_bash"]

##############################
# Actors Config ##############
##############################

[actors.assistant]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/assistant" }

[actors.assistant.config]
model_name = "openai/gpt-5-mini"

# Add a bash execution actor
[actors.execute_bash]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/execute_bash" }

##############################
# TUI Config #################
##############################

[tui.dashboard.key_bindings]
"ctrl-c" = "Exit"
"esc" = "InterruptAgent"

[tui.chat.key_bindings]
"ctrl-a" = "Assist"

##############################
# LiteLLM Config #############
##############################

[litellm]
image = "ghcr.io/berriai/litellm:main-latest"
port = 4000
container_name = "wasmind-litellm"

[[litellm.models]]
model_name = "openai/gpt-5-mini"

[litellm.models.litellm_params]
model = "openai/gpt-5-mini"
api_key = "os.environ/OPENAI_API_KEY"
```

**What changed:**
- Added `"execute_bash"` to `starting_actors` - now both actors start together
- Added `[actors.execute_bash]` definition - defines where to find the bash actor
- The assistant can now execute shell commands when you ask it to!

## Working with Different Models

Want to use a different AI model? Here's how to configure various providers:

### Using Anthropic Claude

```toml
[actors.assistant.config]
model_name = "anthropic/claude-4-sonnet"

[[litellm.models]]
model_name = "anthropic/claude-4-sonnet"

[litellm.models.litellm_params]
model = "anthropic/claude-4-sonnet"
api_key = "os.environ/ANTHROPIC_API_KEY"
```

### Using Local Models with Ollama

```toml
[actors.assistant.config]
model_name = "ollama/llama3.2"

[[litellm.models]]
model_name = "ollama/llama3.2"

[litellm.models.litellm_params]
model = "ollama/llama3.2"
api_base = "http://localhost:11434"
```

See <a href="https://docs.litellm.ai/docs/providers" target="_blank">LiteLLM's supported providers</a> for all available options.

## Actor Sources: Local vs Remote

### Using GitHub Sources (Recommended)
```toml
[actors.assistant]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/assistant" }
```
- Always gets the latest version
- Works from any directory
- Good for getting started

### Using Local Development
```toml
[actors.my_custom_actor]
source = { path = "/path/to/my/actor/directory" }
```
- Use when developing your own actors
- Points to local filesystem
- Good for testing changes

## Actor Configuration and Overrides

### Direct Configuration vs Overrides

**Important**: There are two ways to configure actors, and it's crucial to use the right one:

1. **Direct configuration** - For actors YOU define in `[actors.*]`:
```toml
[actors.assistant]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/assistant" }

# Configure it directly under the actor definition
[actors.assistant.config]
model_name = "openai/gpt-5-mini"
base_url = "https://api.openai.com/v1"
```

2. **Override configuration** - For actors that are dependencies of actors you add:
```toml
# If you add delegation_network_coordinator, it spawns assistant actors internally
[actors.delegation_network_coordinator]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/delegation_network/crates/delegation_network_coordinator" }

# Use overrides to configure the assistant actors it spawns
[actor_overrides.main_manager_assistant.config]
model_name = "openai/gpt-5-mini"

[actor_overrides.worker_assistant.config]
model_name = "anthropic/claude-4-sonnet"
```

### When to Use Overrides

Use `[actor_overrides.*]` when:
- An actor you add has dependencies that spawn other actors
- You want to configure those dependency actors

**DO NOT** use overrides for actors you directly define in `[actors.*]` - configure those directly under their definition.

### How Actor Dependencies Work

Each actor can define dependencies in its `Wasmind.toml` manifest file. These dependencies become available for you to configure via overrides.

For example, the <a href="https://github.com/SilasMarvin/wasmind/blob/main/actors/delegation_network/crates/delegation_network_coordinator/Wasmind.toml" target="_blank">delegation network coordinator's manifest</a> defines many dependencies:

```toml
# In the actor's Wasmind.toml file
[dependencies.main_manager_assistant]
source = { path = "/Users/silasmarvin/github/wasmind/actors/assistant" }

[dependencies.worker_assistant]
source = { path = "/Users/silasmarvin/github/wasmind/actors/assistant" }

[dependencies.spawn_agent]
source = { path = "../../crates/spawn_agent" }
```

These dependency names (`main_manager_assistant`, `worker_assistant`, etc.) become the names you use in `actor_overrides`:

```toml
# In your user configuration file
[actors.delegation_network_coordinator]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/delegation_network/crates/delegation_network_coordinator" }

# Configure the dependencies defined in the actor's manifest
[actor_overrides.main_manager_assistant.config]
model_name = "openai/gpt-5-mini"

[actor_overrides.worker_assistant.config]
model_name = "anthropic/claude-4-sonnet"
```

**Key insight**: Actor developers define the dependency structure in their `Wasmind.toml`, and you configure those dependencies in your user configuration via `actor_overrides`.

Each actor defines what configuration options it accepts. For a complete specification of actor override patterns, see the <a href="https://github.com/SilasMarvin/wasmind/tree/main/crates/wasmind_config" target="_blank">Wasmind Configuration Documentation</a>.

## Starting Actors vs Dynamic Spawning

### What `starting_actors` Controls

The `starting_actors` list defines which actors launch when Wasmind starts up - it's your initial bootstrap. These actors become the first agent in your system, operating under the root scope:

```toml
starting_actors = ["assistant", "execute_bash", "file_interaction"]

[actors.assistant]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/assistant" }

[actors.execute_bash]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/execute_bash" }

[actors.file_interaction]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/file_interaction" }

[actors.file_interaction.config]
allow_edits = true  # Enable both read and write operations
```

Now your assistant can chat, run commands, AND read/write files!

### The Real Power: Dynamic Agent Spawning

**Important**: `starting_actors` only controls the initial setup. The real power of Wasmind comes from **dynamic agent spawning** - coordination actors creating new agents (groups of actors) during runtime.

For example, you might start with just one actor:

```toml
starting_actors = ["delegation_network_coordinator"]

[actors.delegation_network_coordinator]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/delegation_network/crates/delegation_network_coordinator" }
```

But this single coordination actor can dynamically spawn new agents containing:
- Main manager agent (assistant + coordination actors)
- Sub-manager agents (assistant + coordination actors)
- Worker agents (assistant + tool actors)
- Review agents (specialized assistant actors)
- And many more as needed

**Key insight**: A simple `starting_actors` list doesn't limit your system's capability. Complex multi-agent networks are built through dynamic agent spawning, not by listing hundreds of actors in `starting_actors`.

> **Important**: Each actor has different configuration options. The `execute_bash` actor takes no configuration, while `file_interaction` can be configured to allow/disallow edits. Always check each actor's README for available configuration options.

## Interface Customization

The `wasmind_cli` is just one interface to the Wasmind library. You can customize its behavior:

```toml
# Brief TUI customization
[tui.chat.key_bindings]
"ctrl-a" = "Assist"
"ctrl-t" = "ToggleToolExpansion"

[tui.dashboard.key_bindings]
"ctrl-c" = "Exit"
"esc" = "InterruptAgent"
```

> **Note**: For detailed TUI configuration options, see the <a href="https://github.com/SilasMarvin/wasmind/tree/main/crates/wasmind_cli" target="_blank">wasmind_cli documentation</a>. Remember that you can also build your own interfaces using the core Wasmind library.

## Validation and Debugging

Always validate your configurations before running:

```bash
# Check your configuration for errors and see what actors will be loaded
wasmind_cli check -c my-config.toml
```
## Next Steps

### Add More Complexity
Ready for more advanced setups? The [Examples](./examples.md) guide shows:
- Multi-agent delegation networks
- Approval workflows
- Custom coordination patterns

### Understand Available Actors
Want to know what actors you can use? The [Using Actors](./using-actors.md) guide covers:
- Built-in actor types and capabilities
- When to use each actor
- How actors work together

### Technical Reference
Need the complete configuration specification? See the <a href="https://github.com/SilasMarvin/wasmind/tree/main/crates/wasmind_config" target="_blank">Wasmind Configuration Documentation</a>
