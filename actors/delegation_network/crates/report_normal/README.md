# Report Normal Tool Actor

*Example tool actor for health check reporting within the Wasmind delegation network*

This tool actor enables health analyzer agents to report that a monitored agent is healthy and making normal progress. It provides a clean exit path for health check agents when no issues are detected.

## Tools Provided

This actor exposes the following tool to AI agents:

### `report_normal`
- **Description**: Report that the analyzed agent is healthy and making normal progress
- **Parameters**: None required
- **Usage**: Used by health analyzer agents to signal positive health assessments

## When You Might Want This Actor

Include this actor in your Wasmind configuration when you need:

- **Health check completion**: Allow analyzer agents to report positive health assessments
- **Clean agent exit**: Provide a way for temporary analyzer agents to complete and exit
- **Monitoring feedback**: Signal that monitored agents are functioning correctly
- **Automated supervision**: Support health check workflows that need positive reporting
- **System stability**: Confirm agents are operating within expected parameters
- **Quality assurance**: Document that agents are making appropriate progress

This actor is typically used in conjunction with the check_health actor system to provide positive health reporting capabilities.

## Messages Listened For

- `tools::ExecuteTool` - Receives tool execution requests for reporting normal status
  - **Scope**: Only listens to messages from its own scope (standard tool actor behavior)
  - Handles `report_normal` tool calls with no parameters required

## Messages Broadcast

- `tools::ToolsAvailable` - Announces the `report_normal` tool to AI agents when initialized
- `tools::ToolCallStatusUpdate` - Reports successful health assessment completion
- `actors::Exit` - Signals the analyzer agent to terminate after reporting

## Configuration

```toml
[actors.report_normal]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/delegation_network/crates/report_normal" }
```

No configuration required. The actor is ready to use once included in your actor list. These actors are typically spawned by the delegation_network_coordinator.

## How It Works

When activated in a Wasmind system, this actor:

1. **Registers the `report_normal` tool** with health analyzer agents
2. **Processes health reports** when analyzer agents determine monitored agents are healthy
3. **Provides completion feedback** confirming the health assessment was recorded
4. **Triggers agent exit** by broadcasting an Exit message to terminate the analyzer
5. **Enables clean workflow completion** for temporary health check agents

This actor provides a simple, clean mechanism for health analyzers to report positive assessments and properly terminate, maintaining efficient resource usage in the delegation network.

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*