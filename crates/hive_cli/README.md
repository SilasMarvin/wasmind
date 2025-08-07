# Hive CLI

A command-line interface and terminal application built with Hive that demonstrates actor-based AI development workflows. The CLI provides an interactive environment for working with AI agents and multi-agent coordination systems.

## Installation

Build from source:
```bash
git clone https://github.com/SilasMarvin/hive
cd hive
cargo build --release
./target/release/hive_cli --help
```

## Basic Usage

**Interactive Mode** (default):
```bash
hive_cli
```
Launches the terminal interface for chatting with AI agents and viewing system state.

**Commands**:
```bash
hive_cli info      # Show configuration and cache information
hive_cli clean     # Clean the actor cache  
hive_cli check     # Validate and show configuration details
```

**Options**:
```bash
hive_cli -c config.toml           # Use custom configuration
hive_cli -p "Hello assistant"     # Send initial message to assistant
hive_cli --log-file /path/to.log  # Custom log file location
```

## Configuration

The CLI uses TOML configuration files to define:
- **Actor Setup**: Which actors to load and their configurations
- **TUI Settings**: Key bindings and interface options
- **Actor Overrides**: Custom settings per actor

See [default_config.toml](default_config.toml) for a complete example and the [Configuration Guide](../hive_config/) for detailed reference.

## What You Can Do

- **Chat with AI assistants** powered by various LLM providers
- **Coordinate multi-agent workflows** using the delegation network system
- **Execute system commands** through bash integration actors
- **Monitor system state** with real-time visualization of actor interactions

## Links

- **📚 [Hive Book](../../docs/)** - Complete user guides and concepts
- **⚙️ [Configuration Guide](../hive_config/)** - Detailed configuration reference  
- **🎭 [Actor Examples](../../actors/)** - Available actors and their capabilities