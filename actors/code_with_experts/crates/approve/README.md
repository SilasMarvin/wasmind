# Approve Actor

*Tool for approving file changes in expert review workflows*

This actor provides the `approve` tool used by expert reviewers to approve proposed file changes. It is automatically spawned as a dependency when expert review is triggered and is not intended for standalone use.

## Actor ID
`hcwe_approve`

## Tools Provided

### `approve`
- **Description**: Approve the proposed file changes
- **Parameters**: None required
- **Usage**: Used by expert agents to signal approval of file edits

## Configuration

No configuration required. Include this actor to provide the approve tool:

```toml
[actors.approve]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/code_with_experts/crates/approve" }
```

Note: This actor is typically automatically spawned as a dependency by the `file_interaction_with_approval` actor.

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*