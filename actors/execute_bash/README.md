# Execute Bash Tool Actor

*Example tool actor providing command-line execution capabilities for the Wasmind library*

This tool actor provides AI agents with the ability to execute bash commands in a controlled, stateless environment. Each command runs in a fresh bash shell with support for all standard shell features including pipes, redirections, and command chaining.

## Actor ID
`execute_bash`

## Tools Provided

This actor exposes the following tool to AI agents:

### `execute_bash`
- **Description**: Execute a bash command in a stateless environment with full shell features
- **Parameters**:
  - `command`: The bash command to execute (required)
  - `args`: Optional array of additional arguments to append to the command
  - `directory`: Optional working directory for command execution
  - `timeout`: Optional timeout in seconds (default: 30s, max: 600s)
- **Usage**: Agents use this tool to run shell scripts, system utilities, file operations, and any command-line tasks

## When You Might Want This Actor

Include this actor in your Wasmind configuration when you need your AI agents to:

- **Execute system commands**: Run shell scripts, system utilities, and command-line tools
- **File operations**: Create, modify, and manage files using standard Unix tools
- **Development workflows**: Build projects, run tests, manage dependencies
- **System administration**: Monitor processes, check disk usage, manage services  
- **Data processing**: Use command-line tools like `grep`, `awk`, `sed` for text processing
- **Network operations**: Test connectivity, download files, make HTTP requests
- **General automation**: Any task that can be accomplished via command line

This actor is essential for AI agents that need to interact with the underlying system and perform practical tasks beyond just conversation.

## Messages Listened For

- `tools::ExecuteTool` - Receives tool execution requests when the AI agent wants to run a command
  - **Scope**: Only listens to messages from its own scope (standard tool actor behavior)
  - Contains the bash command to execute along with optional parameters like working directory and timeout

## Messages Broadcast

- `tools::ToolsAvailable` - Announces the `execute_bash` tool to AI agents when initialized
- `tools::ToolCallStatusUpdate` - Broadcasts command execution results back to the requesting agent
  - Includes both success and failure outcomes with detailed output information
  - Provides structured UI display information for better user experience

## Configuration

No configuration required. Include this actor to provide the execute_bash tool:

```toml
[actors.execute_bash]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/execute_bash" }
```

## How It Works

When activated in a Wasmind system, this actor:

1. **Registers the `execute_bash` tool** with AI agents, making it available for use
2. **Receives tool calls** when agents decide to run shell commands
3. **Executes commands safely** using `bash -c` in isolated environments
4. **Handles all execution outcomes** including success, failure, timeouts, and signals
5. **Provides rich feedback** with both stdout/stderr output and structured UI displays
6. **Manages output size** by intelligently truncating large outputs while preserving important information

The actor ensures each command runs in a fresh bash environment without session state, providing predictable and secure command execution for AI agents.

## Building

To build the Execute Bash Actor WASM component:

```bash
cargo component build
```

This generates `target/wasm32-wasip1/debug/execute_bash.wasm` for use in the Wasmind system.

## Testing

Run the test suite:

```bash
cargo test
```

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*
