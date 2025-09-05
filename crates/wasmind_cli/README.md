# Wasmind CLI

**A command-line interface and terminal application for the Wasmind library**

This CLI provides an interactive terminal user interface for running and managing Wasmind actor configurations. It serves as a general-purpose interface to wasmind's actor-based AI coordination capabilities, allowing you to run any actor setup through an intuitive TUI.

> **Note**: This is a reference implementation showing how to build user interfaces with wasmind. You can run any wasmind actor configuration - we've included some sample configurations to get you started.

## What You Can Build

The wasmind_cli provides a flexible TUI for running any wasmind actor configuration. You can:

### 🖥️ **Interactive Terminal Interface**
- **Chat view** - communicate directly with AI agents in your configuration
- **Dashboard** - system overview and controls for your actor setup
- **Graph view** - visualize agent relationships and message flow in real-time
- **Configuration management** - easily switch between different actor setups

### ⚙️ **Any Actor Configuration**
- **Multi-agent workflows** - coordinate any number of specialized AI agents
- **Custom tool integration** - run actors with file interaction, bash execution, or custom capabilities  
- **Hierarchical systems** - build manager/worker patterns or delegation networks
- **Custom actors** - create your own specialized actors for domain-specific tasks
- **Model-agnostic** - works with any LLM provider through LiteLLM proxy

## Included Example Configurations

We've included sample configurations to help you get started:

### 🔍 **Code Edit Approval Workflow** (`example_configs/code_with_experts.toml`)
A collaborative code editing system where any code edit request triggers validation by configurable expert agents:
- **Type checking expert** - validates Python typing standards before code changes are applied
- **Best practices expert** - validates PEP 8 and Python idioms before code changes are applied  
- **Architecture expert** - validates code organization and structure before code changes are applied
- **Multi-agent approval** - code edits only proceed if all expert agents approve the changes

### 🏗️ **Delegation Network** (`example_configs/delegation_network.toml`)  
A hierarchical agent coordination system demonstrating:
- **Dynamic task delegation** - managers spawn and coordinate specialized workers
- **Multi-level communication** - manager → sub-manager → worker message patterns
- **Health monitoring** - system-wide agent status and coordination
- **Scalable architecture** - easily spawn additional agents as needed

## Quick Start

### Prerequisites

- **Rust/Cargo** - Required to build and install the CLI
- **Docker** - Required to run the LiteLLM model proxy for AI model routing  
- **cargo-component** - Required to build WASM actor components (`cargo install cargo-component`)

### Installation

```bash
cargo install wasmind_cli --locked
```

### Run Example Configurations

```bash
# Code edit approval workflow
wasmind_cli -c example_configs/code_with_experts.toml

# Delegation network  
wasmind_cli -c example_configs/delegation_network.toml

# Or use your own configuration
wasmind_cli -c path/to/your/config.toml
```

### Create Your Own Actor Configurations

- Study `example_configs/` - Ready-to-run sample configurations
- Explore `../../actors/` - Available actor implementations you can use
- Build custom actors - see [Creating Actors Guide](https://silasmarvin.github.io/wasmind/developer-guide/building-actors.html)
- See the [Configuration Guide](../wasmind_config/) for creating custom setups

### Debugging Configurations

Use the `check` command to validate and debug configuration files before running them:

```bash
wasmind_cli check -c path/to/your/config.toml
```

This will:
- Validate TOML syntax and structure
- Verify actor paths and dependencies
- Check for missing or circular dependencies
- Display resolved configuration with all defaults applied
- Show any configuration errors or warnings

**Debug Message Flow**:

To see all messages being sent through the actor system, run with debug logging:

```bash
WASMIND_LOG=debug wasmind_cli -c your_config.toml
```

This is especially helpful when:
- Debugging actor communication issues
- Understanding message routing between agents
- Troubleshooting why actors aren't responding as expected

## Commands & Options

**Interactive Mode** (default):
```bash
wasmind_cli -c path/to/config.toml             # Use specific configuration
wasmind_cli -p "Hello assistant"               # Send initial message to agents  
wasmind_cli --log-file /path/to.log            # Custom log file location
```

**Utility Commands**:
```bash
# Show default config location, cache paths, and system information
wasmind_cli info      

# Clean the actor cache (removes compiled WASM components)
# Actors are compiled and cached on first use for faster subsequent loads
wasmind_cli clean     
# See [wasmind_actor_loader](../wasmind_actor_loader/) for details on caching

# Validate and debug configuration files
wasmind_cli check -c example_configs/code_with_experts.toml
```

**Environment Variables**:
```bash
# Set log level (error, warn, info, debug, trace)
WASMIND_LOG=debug wasmind_cli  # Debug level shows all messages sent through the system
WASMIND_LOG=info wasmind_cli   # Default level for general information
```

**Default Key Bindings** (in TUI):
- `Ctrl+a` - Assist (send message to agents)
- `Ctrl+t` - Toggle expanded tool displays
- `esc` - Cancel the Agent's current action and force it to wait for your input
- `Ctrl+c` - Exit
- `Shift+Up/Down` - Navigate graph view

NOTE: The cancel feature is  WIP and if the Agent is making a request it will finish making it before cancelling.

## Configuration

The CLI uses TOML configuration files to define your actor setup. Configurations specify:
- Which actors to load and their settings
- TUI key bindings and interface options  
- Actor-specific overrides
- LLM provider configuration via LiteLLM

The example configurations show different patterns you can use, but you're free to create any actor configuration that suits your needs. See the [Configuration Guide](../wasmind_config/) for detailed reference.

## Links

- **📚 [Wasmind Book](https://silasmarvin.github.io/wasmind/)** - Complete user guides and concepts
- **⚙️ [Configuration Guide](../wasmind_config/)** - Detailed configuration reference  
- **🎭 [Actor Examples](../../actors/)** - Available actors and their capabilities
