# File Interaction Actor

*File reading and editing with line-by-line precision*

This actor gives AI agents the ability to read and edit files with line-by-line precision. It handles everything from simple file reads to complex multi-edit operations, with built-in caching and safety features to prevent data loss.

## Architecture

This is a workspace containing two crates:
- **`file_interaction`** - Pure Rust library with all the file manipulation logic
- **`file_interaction_actor`** - WASM actor wrapper that provides the tools to AI agents

The library can be used independently by other Rust projects, while the actor provides the Wasmind integration.

## Actor ID
`file_interaction_actor`

## Tools Provided

This actor exposes two tools to AI agents:

### `read_file`
- **Description**: Reads content from files with automatic line numbering
- **Parameters**:
  - `path`: Path to the file (required) 
  - `start_line`: Optional starting line number (1-indexed)
  - `end_line`: Optional ending line number (inclusive)
- **Usage**: Read entire files or specific line ranges, with automatic caching for large files

### `edit_file`
- **Description**: Apply multiple edits to a file atomically
- **Parameters**:
  - `path`: Path to the file to edit or create (required)
  - `edits`: Array of edit operations, each containing:
    - `start_line`: Line number to start the edit (1-indexed)
    - `end_line`: Line number to end the edit (for insertions, use start_line - 1)
    - `new_content`: The content to replace with
- **Usage**: Create files, modify existing content, insert/delete lines, all in a single operation

## When You Might Want This Actor

Include this actor when you need AI agents to:

- **Read and analyze code**: Examine files with line numbers for debugging and understanding
- **Edit files safely**: Make precise changes without corrupting existing content  
- **Create new files**: Generate code, documentation, or configuration files
- **Handle large files**: Work with big files using line ranges instead of loading everything
- **Batch file operations**: Apply multiple edits to the same file in one atomic operation
- **Development workflows**: Code editing, refactoring, and file management tasks

This actor is essential for AI agents that need to interact with codebases, manage project files, or perform any file-based development tasks.

## Configuration

```toml
[actors.file_interaction_actor]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/file_interaction/crates/file_interaction_actor" }

# Optional configuration
[actors.file_interaction_actor.config]
allow_edits = true  # Default: true. Set to false for read-only mode
```

## Messages Listened For

### From Own Scope

- **`tools::ExecuteTool`** - Receives tool execution requests when AI agents want to read or edit files
  - **Scope**: Only listens to messages from its own scope (standard tool actor behavior)
  - Handles both `read_file` and `edit_file` tool calls with their respective parameters

### From Any Scope

- **`assistant::StatusUpdate`** - Monitors assistant status changes for conversation compaction
  - When status changes to `CompactingConversation`, immediately clears file cache

## Messages Broadcast

- `tools::ToolsAvailable` - Announces the `read_file` and `edit_file` tools to AI agents when initialized
- `tools::ToolCallStatusUpdate` - Reports the results of file operations back to the requesting agent
  - Includes both success and failure outcomes with detailed file information
  - Provides structured UI display for better user experience
- `assistant::SystemPromptContribution` - Provides comprehensive usage guidance and best practices

## Configuration Options

- **`allow_edits`** (default: true): Controls which tools are available to AI agents
  - When `true`: Provides both `read_file` and `edit_file` tools
  - When `false`: Provides only `read_file` tool (read-only mode)

## How It Works

When activated in a Wasmind system, this actor:

1. **Registers file tools** with AI agents, making `read_file` and `edit_file` available
2. **Provides usage guidance** through system prompt contributions with examples and best practices
3. **Handles file reads** with automatic line numbering and intelligent caching for large files
4. **Processes edit operations** by applying multiple changes atomically to prevent corruption
6. **Creates directories** automatically when editing files in non-existent directories
7. **Caches file content** to optimize repeated reads and enable efficient partial file access
8. **Handles conversation compaction** by clearing caches when the assistant begins compacting conversations

The actor ensures all file operations are safeish and reliable, with comprehensive error handling and user-friendly feedback for both successful operations and failures. It also integrates with conversation compaction to prevent stale file information from appearing in new conversation contexts.

## Building

To build the File Interaction Actor WASM component:

```bash
cd crates/file_interaction_actor
cargo component build
```

This generates the WASM file for use in the Wasmind system. You can also build the entire workspace:

```bash
cargo build
```

## Testing

Run the test suite:

```bash
cargo test
```

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*
