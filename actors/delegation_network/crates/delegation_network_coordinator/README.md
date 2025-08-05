# Delegation Network Coordinator Actor

*Example infrastructure actor for managing agent hierarchies within the Hive delegation network*

This infrastructure actor monitors and coordinates the delegation network by tracking active agents, their relationships, and preventing invalid operations. It ensures the integrity of the agent hierarchy and provides intelligent guardrails for manager agents. Unlike tool actors, this actor provides coordination and monitoring services rather than exposing tools to AI agents.

## Actor ID
`delegation_network_coordinator`

## When You Might Want This Actor

Include this actor in your Hive configuration when you need:

- **Agent hierarchy tracking**: Monitor parent-child relationships between agents
- **Active agent management**: Track which agents are currently running vs completed
- **Operation validation**: Prevent invalid operations like managers waiting when they have no subordinates
- **Network integrity**: Maintain consistency in the delegation network structure
- **Lifecycle coordination**: Track agent spawning and completion events
- **Smart guardrails**: Provide intelligent feedback to prevent common delegation mistakes

This actor is essential for delegation networks to maintain structural integrity and provide intelligent coordination between managers and their subordinates. See the [Delegation Network overview](../../README.md) for complete system architecture and usage examples.

## Messages Listened For

- `delegation_network::AgentSpawned` - Tracks when new agents are created in the network
  - Updates internal registry of active agents and their relationships
  
- `assistant::StatusUpdate` - Monitors agent status changes, especially completion
  - Removes completed agents from active tracking
  
- `tools::ExecuteTool` - Intercepts tool calls to validate operations
  - Prevents managers from waiting when they have no active subordinates

## Messages Broadcast

- `assistant::AddMessage` - Sends error messages to agents attempting invalid operations
  - Provides clear feedback about why an operation cannot proceed

## Configuration

No configuration required. The actor automatically coordinates any delegation network it's part of.

## How It Works

When activated in a Hive system, this actor:

1. **Tracks agent spawning** by monitoring AgentSpawned messages and building the hierarchy tree
2. **Maintains active agent registry** with parent-child relationships and agent types
3. **Monitors agent completion** and updates the registry when agents finish their tasks
4. **Validates manager operations** by checking if managers have active subordinates before allowing wait operations
5. **Provides intelligent feedback** with clear error messages when invalid operations are attempted
6. **Ensures network consistency** by maintaining accurate state across the entire delegation hierarchy

This coordinator ensures that delegation networks operate smoothly with proper tracking and validation of agent relationships and operations.

## Building

To build the Delegation Network Coordinator Actor WASM component:

```bash
cargo component build
```

This generates `target/wasm32-wasip1/debug/delegation_network_coordinator.wasm` for use in the Hive system.