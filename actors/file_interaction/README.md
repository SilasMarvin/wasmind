# File Interaction Tool Actor

*Example tool actor providing file system operations for the Hive library*

This tool actor provides AI agents with comprehensive file reading and editing capabilities, including intelligent caching, atomic operations, and automatic workspace management. It supports both reading existing files and creating new ones with built-in safety features.

## Actor ID
`file_interaction`

## Tools Provided

This actor exposes the following tools to AI agents:

### `read_file`
- **Description**: Reads content from a file with automatic line numbering
- **Parameters**:
  - `path`: Absolute path to the file (must start with `/`)
  - `start_line`: Optional starting line to read (1-indexed)
  - `end_line`: Optional ending line to read (inclusive)
- **Usage**: Read source code, configuration files, documentation, or any text files

### `edit_file`
- **Description**: Apply atomic edits to a file or create new files
- **Parameters**:
  - `path`: Absolute path to the file to edit or create
  - `edits`: Array of edit operations, each containing:
    - `start_line`: Line number to start the edit (1-indexed)
    - `end_line`: Line number to end the edit (inclusive)
    - `new_content`: Content to replace the specified lines
- **Usage**: Modify existing files, create new files, apply multiple edits atomically

## When You Might Want This Actor

Include this actor in your Hive configuration when you need your AI agents to:

- **Read and analyze files**: Load source code, configuration files, documentation, or data files
- **Edit existing files**: Modify code, update configurations, or make content changes
- **Create new files**: Generate new source files, documentation, or configuration files  
- **Code development**: Work with codebases by reading, modifying, and creating files
- **Content management**: Handle text files, documentation, and structured data
- **Configuration management**: Read and update application settings and configuration files
- **Data processing**: Work with text-based data files like JSON, CSV, XML, etc.
- **Development workflows**: Support any task that requires file system interaction

This actor is essential for AI agents that need to work with files as part of development, content creation, or data management tasks.

## Messages Listened For

- `tools::ExecuteTool` - Receives tool execution requests for file operations
  - **Scope**: Only listens to messages from its own scope (standard tool actor behavior)
  - Handles `read_file` tool calls for reading file content with optional line ranges
  - Handles `edit_file` tool calls for creating new files or editing existing ones

## Messages Broadcast

- `tools::ToolsAvailable` - Announces the availability of `read_file` and `edit_file` tools to AI agents
- `tools::ToolCallStatusUpdate` - Reports the results of file operations back to requesting agents
- `assistant::SystemPromptContribution` - Provides usage guidance and maintains a live view of all open files in the workspace

## Configuration

No configuration required. The actor is ready to use once included in your actor list.

## How It Works

When activated in a Hive system, this actor:

1. **Registers file tools** (`read_file` and `edit_file`) with AI agents
2. **Provides usage guidance** via system prompt contributions explaining best practices
3. **Handles read requests** with automatic line numbering and intelligent caching
4. **Processes edit requests** atomically, supporting both file creation and modification
5. **Maintains workspace awareness** by updating the system prompt with all open files
6. **Enforces security** by requiring absolute paths and validating all operations

Key features:
- **Smart file handling**: Small files (<64KB) are read fully, large files require line ranges
- **Atomic operations**: Multiple edits applied from bottom to top to maintain line integrity
- **Automatic directory creation**: Parent directories created as needed
- **Intelligent caching**: Optimizes repeated file access and tracks modifications

## Building

To build the File Interaction Actor WASM component:

```bash
cargo component build
```

This generates `target/wasm32-wasip1/debug/file_interaction.wasm` for use in the Hive system.

## Testing

Run the test suite:

```bash
cargo test
```