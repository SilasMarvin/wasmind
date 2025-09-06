# File Interaction with Approval Actor

*Enhanced file interaction with expert review for edits above threshold*

This actor provides `edit_file` and `read_file` tools with an approval workflow. When edits exceed a configurable size threshold, it automatically spawns expert agents to review changes before applying them.

## Tools Provided

### `read_file`
- **Description**: Read file contents (same as standard file_interaction)
- **Parameters**:
  - `path`: Path to the file (required)
  - `start_line`: Optional starting line number
  - `end_line`: Optional ending line number
- **Usage**: Read files without any approval workflow

### `edit_file`
- **Description**: Edit files with expert approval for large changes
- **Parameters**:
  - `path`: Path to the file (required)
  - `edits`: Array of edit operations (required)
- **Usage**: Small edits are applied directly; large edits trigger expert review

## Configuration

Requires configuration to specify approval workflow:

```toml
[actors.file_interaction_with_approval]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/code_with_experts/crates/file_interaction_with_approval" }

[actors.file_interaction_with_approval.config]
min_diff_size = 50  # Character threshold for triggering review

[actors.file_interaction_with_approval.config.approvers]
expert_name = ["assistant", "read_file", "execute_bash", ...]
```

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*