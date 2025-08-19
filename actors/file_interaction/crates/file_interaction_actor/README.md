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

This actor requires no configuration. It is ready to use once included in your actor list.

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*