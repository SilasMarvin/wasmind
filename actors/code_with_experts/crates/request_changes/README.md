# Request Changes Actor

*Tool for requesting changes to file edits in expert review workflows*

This actor provides the `request_changes` tool used by expert reviewers to request modifications to proposed file changes. It is automatically spawned as a dependency when expert review is triggered and is not intended for standalone use.

## Tools Provided

### `request_changes`
- **Description**: Request changes to the proposed file edits
- **Parameters**:
  - `changes_requested`: Clear description of what changes are needed (required)
- **Usage**: Used by expert agents to specify required modifications to file edits

## Configuration

No configuration required. Include this actor to provide the request_changes tool:

```toml
[actors.request_changes]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/code_with_experts/crates/request_changes" }
```

Note: This actor is typically automatically spawned as a dependency by the `file_interaction_with_approval` actor.

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*