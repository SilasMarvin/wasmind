# Using Actors

Now that you understand how to configure Wasmind, let's explore how to choose and work with actors effectively. The key to building successful Wasmind systems is understanding actor compatibility.

## Actor Types

Actors in Wasmind typically fall into three categories:

### Assistant Actors
LLM-powered actors that handle conversation, reasoning, and decision-making.
- Manage chat history and context
- Interface with AI models through LiteLLM
- Coordinate with other actors to accomplish tasks

### Tool Actors  
Actors that provide concrete capabilities to the system.
- File system operations (read, write, edit)
- Command execution (bash, system commands)
- Web requests, database access, etc.
- Give assistant actors "hands" to interact with the world

### Coordination Actors
Actors that manage multi-agent workflows and communication.
- Spawn new agents dynamically
- Route messages between agents
- Implement delegation and approval patterns
- Enable complex multi-agent coordination

## Available Actors

The `actors/` directory in the Wasmind repository contains example actors created for demonstration and common use cases:

- `assistant` - Core LLM interaction actor
- `execute_bash` - Command execution capability
- `file_interaction` - File read/write operations
- `conversation_compaction` - Background conversation management
- `delegation_network/` - Multi-agent coordination system
- And more...

These are **reference implementations**, not built-in system components. They demonstrate common patterns and provide starting points for your own configurations.

> **Browse actors**: Check the [actors directory](https://github.com/SilasMarvin/wasmind/tree/main/actors) to see what's available and read each actor's README for capabilities and configuration options.

## Actor Compatibility

**Good news**: Most actors in the Wasmind repository are designed to work together! The actors in `actors/` follow common message patterns, so you can mix and match them freely.

### Current State: Most Actors Work Together

The actors you'll find in the repository use compatible message protocols:

```toml
# These combinations all work well
starting_actors = ["assistant", "execute_bash", "file_interaction"]
starting_actors = ["delegation_network_coordinator"]  
starting_actors = ["assistant", "conversation_compaction"]
```

You don't need to worry about detailed message compatibility when using actors from the Wasmind repository - they're designed to work together.

### When Compatibility Matters

Compatibility becomes important when:
- Using actors from different sources or repositories
- Building your own custom actors
- Mixing very new actors with older ones

**For now**: Stick with actors from the main Wasmind repository and you'll be fine!

## Common Actor Patterns

Here are some popular combinations that work well:

**Assistant + Tools Pattern**:
```toml
starting_actors = ["assistant", "execute_bash", "file_interaction"]
```
- Assistant coordinates, tools provide capabilities
- Great for interactive development workflows

**Coordination Network Pattern**:
```toml
starting_actors = ["delegation_network_coordinator"]
```
- Single coordinator spawns and manages multiple agents
- Perfect for complex multi-agent workflows

**Simple Chat Pattern**:
```toml
starting_actors = ["assistant", "conversation_compaction"]
```
- Basic conversational AI with automatic history management
- Good starting point for chat applications

### Testing Your Setup

```bash
# Validate your configuration syntax
wasmind_cli check -c your-config.toml

# Test with minimal setups first
# Add actors one at a time to identify any issues
```

## Coming Soon: Actor Registry

The Wasmind ecosystem is growing toward a comprehensive actor registry that will include:

- **Community-contributed actors** from developers worldwide
- **Compatibility metadata** showing which actors work well together
- **Standardized interfaces** for common actor types
- **Dependency resolution** to automatically include compatible actors

This will make it much easier to discover and compose compatible actors for your use cases.

## Next Steps

### Explore Real Examples
Ready to see these concepts in action? The [Examples](./examples.md) guide walks through:
- Complete working configurations
- How different actors coordinate in practice
- Building from simple to complex multi-agent systems

### Build Your Own Actors
Want to create compatible actors? The [Developer Guide](../developer-guide/README.md) covers:
- Message pattern design
- Building actors that integrate well with existing ones
- Testing actor compatibility

### Technical Deep Dive
For the complete technical specification, see the [Wasmind Configuration Documentation](../../crates/wasmind_config/README.md).