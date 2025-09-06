# File Interaction Actor

*WASM actor providing file reading and editing tools*

This is the WASM actor component that provides `read_file` and `edit_file` tools to AI agents. It wraps the file_interaction library to enable file operations within the Wasmind actor system.

## Actor ID
`file_interaction_actor`

## Tools Provided

### `read_file`
- **Description**: Read file contents with line numbering
- **Parameters**:
  - `path`: Path to the file (required)
  - `start_line`: Optional starting line number (1-indexed)
  - `end_line`: Optional ending line number (inclusive)
- **Usage**: Read entire files or specific line ranges with automatic caching

### `edit_file`
- **Description**: Apply multiple edits to a file atomically
- **Parameters**:
  - `path`: Path to the file to edit or create (required)
  - `edits`: Array of edit operations (required)
- **Usage**: Create files, modify existing content, insert/delete lines in one operation

## Configuration

```toml
[actors.file_interaction_actor]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/file_interaction/crates/file_interaction_actor" }

# Optional configuration
[actors.file_interaction_actor.config]
allow_edits = true  # Default: true. Set to false for read-only mode
```

### Configuration Options

- **`allow_edits`** (default: true): Controls which tools are available to AI agents
  - When `true`: Provides both `read_file` and `edit_file` tools
  - When `false`: Provides only `read_file` tool (read-only mode)

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*