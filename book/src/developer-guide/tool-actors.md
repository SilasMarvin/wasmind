# Tool Actors

Tool actors are actors that expose capabilities (tools) to LLMs. They enable AI assistants to perform actions like executing commands, reading files, or interacting with external systems.

## What is a Tool Actor?

A tool actor is any actor that:
1. Broadcasts `ToolsAvailable` messages to announce its capabilities
2. Handles `ExecuteTool` messages to perform actions
3. Sends `ToolCallStatusUpdate` messages to report results

Tool actors are regular actors - they follow the same patterns you've already learned, just with a specific purpose (calling them tool actors is just a categorization we made up for convenience).

## The Tool Pattern

Here's the basic flow of tool interactions:

```
1. Tool actor starts → Broadcasts ToolsAvailable
2. Assistant collects tools → Includes in LLM context
3. LLM generates tool call → Assistant sends ExecuteTool
4. Tool actor executes → Sends ToolCallStatusUpdate
5. Assistant receives result → Continues conversation
```

## Building a Tool Actor (Rust)

The easiest way to build a tool actor in Rust is using the `Tool` derive macro:

```rust
use wasmind_actor_utils::{
    tools,
    common_messages::tools::ExecuteTool,
};
use serde::{Deserialize, Serialize};

#[derive(tools::macros::Tool)]
#[tool(
    name = "read_file",
    description = "Read contents of a file",
    schema = r#"{
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "The file path to read"
            }
        },
        "required": ["path"]
    }"#
)]
pub struct ReadFileTool {
    scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadFileParams {
    pub path: String,
}

impl tools::Tool for ReadFileTool {
    fn new(scope: String, _config: String) -> Self {
        Self { scope }
    }

    fn handle_call(&mut self, tool_call: ExecuteTool) {
        let params: ReadFileParams = match serde_json::from_str(&tool_call.tool_call.function.arguments) {
            Ok(params) => params,
            Err(e) => {
                // Send error ToolCallStatusUpdate and return
                return;
            }
        };
        
        // Read the file
        let contents = match std::fs::read_to_string(&params.path) {
            Ok(contents) => contents,
            Err(e) => {
                // Send error ToolCallStatusUpdate and return
                return;
            }
        };
        
        // Send success ToolCallStatusUpdate with file contents
    }
}
```

## What the Tool Macro Does Automatically

When you use `#[derive(tools::macros::Tool)]`, the macro generates all the actor boilerplate and handles the message flow for you. Here's what happens automatically:

### 1. On Actor Creation (`new()`)
The macro automatically broadcasts a `ToolsAvailable` message to announce your tool:

```rust
// This happens automatically in the generated new() function:
broadcast(ToolsAvailable {
    tools: vec![Tool {
        tool_type: "function",
        function: ToolFunctionDefinition {
            name: "your_tool_name",        // From #[tool(name = "...")]
            description: "your description", // From #[tool(description = "...")]
            parameters: {...}               // From #[tool(schema = "...")]
        }
    }]
})
```

**Note**: The tool definition format follows LiteLLM's OpenAI API compatibility standard for function calling. For more details on tool schemas and function calling, see the <a href="https://docs.litellm.ai/docs/completion/function_call" target="_blank">LiteLLM function calling documentation</a>.

### 2. Message Handling
The macro automatically:
- Listens for `ExecuteTool` messages from the same scope
- Checks if the tool name matches yours
- Calls your `handle_call()` method with the full message

### 3. What You Implement
You only need to provide:
```rust
impl tools::Tool for YourTool {
    fn new(scope: String, config: String) -> Self {
        // Your initialization logic
    }
    
    fn handle_call(&mut self, tool_call: ExecuteTool) {
        // Your tool's actual logic:
        // 1. Parse parameters from tool_call.tool_call.function.arguments
        // 2. Execute your tool's functionality 
        // 3. Send ToolCallStatusUpdate message with result
    }
}
```

Everything else - the guest trait, bindgen exports, message routing - is handled by the macro.

**Note**: Your `handle_call()` method is responsible for sending `ToolCallStatusUpdate` messages to report results back to the requesting assistant.

## Manual Tool Implementation

You can implement a tool actor manually without the derive macro by implementing the guest trait, exporting bindgen functions, and handling the message broadcasting yourself.

If you want to see what the macro generates, examine the macro source code in [`wasmind_actor_utils_macros`](https://github.com/SilasMarvin/wasmind/tree/main/crates/wasmind_actor_utils_macros).

## Tool Parameters

Tools use JSON Schema to define their parameters. The schema is provided as a raw JSON string in the `#[tool()]` attribute:

```rust
#[derive(tools::macros::Tool)]
#[tool(
    name = "read_file",
    description = "Read contents of a file",
    schema = r#"{
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "The file path to read"
            },
            "limit": {
                "type": "integer",
                "description": "Maximum number of lines to read",
                "minimum": 1
            }
        },
        "required": ["path"]
    }"#
)]
pub struct ReadFileTool {
    scope: String,
}
```

The schema is included in the tool definition sent to the LLM via LiteLLM's OpenAI-compatible function calling format. This helps the LLM understand what parameters your tool expects and their constraints. For more information on parameter schemas and function definitions, see the <a href="https://docs.litellm.ai/docs/completion/function_call" target="_blank">LiteLLM function calling documentation</a>.

## Tool Status Reporting

Your tool actor communicates back to the assistant using `ToolCallStatusUpdate` messages with different status types and UI display information.

### ToolCallStatus Types

```rust
pub enum ToolCallStatus {
    // Tool acknowledged the request - use for long-running operations
    Received {
        display_info: UIDisplayInfo,
    },
    
    // Tool waiting for system/user approval (rarely used)
    AwaitingSystem {
        details: AwaitingSystemDetails,
    },
    
    // Tool completed - success or error
    Done {
        result: Result<ToolCallResult, ToolCallResult>,
    },
}
```

### UIDisplayInfo Structure

The `UIDisplayInfo` provides a clean interface in wasmind_cli's TUI:

```rust
pub struct UIDisplayInfo {
    pub collapsed: String,      // Short summary shown by default
    pub expanded: Option<String>, // Detailed view when expanded
}
```

**Why UIDisplayInfo matters:**
- **Collapsed**: Provides scannable overview (e.g., "ls: Success (15 files)")
- **Expanded**: Shows full details when user clicks to expand (complete output, error traces)
- **User Experience**: Programs building on Wasmind (like `wasmind_cli`) use this info to display tool execution updates to users in a clean, organized way
- Essential for good UX - users can scan tool results quickly and dive into details when needed

### Example: Success Response

```rust
// For a successful file read
let ui_display = UIDisplayInfo {
    collapsed: format!("Read {}: {} bytes", filename, content.len()),
    expanded: Some(format!(
        "File: {}\nSize: {} bytes\n\nContent:\n{}", 
        filename, 
        content.len(), 
        content
    )),
};

let result = ToolCallResult {
    content: serde_json::to_string(&json!({ "contents": content })).unwrap(),
    ui_display_info: ui_display,
};

// Send success status
let status_update = ToolCallStatusUpdate {
    status: ToolCallStatus::Done { 
        result: Ok(result) 
    },
    id: tool_call.tool_call.id,
    originating_request_id: tool_call.originating_request_id,
};
Self::broadcast_common_message(status_update).unwrap();
```

### Example: Error Response

```rust
// For parameter parsing error
let ui_display = UIDisplayInfo {
    collapsed: "Parameters: Invalid format".to_string(),
    expanded: Some(format!(
        "Error: Failed to parse parameters\n\nDetails: {}\n\nExpected format: {}", 
        error_message,
        expected_schema
    )),
};

let error_result = ToolCallResult {
    content: format!("Parameter error: {}", error_message),
    ui_display_info: ui_display,
};

let status_update = ToolCallStatusUpdate {
    status: ToolCallStatus::Done { 
        result: Err(error_result) 
    },
    id: tool_call.tool_call.id,
    originating_request_id: tool_call.originating_request_id,
};
Self::broadcast_common_message(status_update).unwrap();
```

### Message Types Summary

Tool actors work with these key message types:

```rust
// Announce available tools (sent automatically by macro)
pub struct ToolsAvailable {
    pub tools: Vec<Tool>,
}

// Request tool execution (received from assistants)
pub struct ExecuteTool {
    pub tool_call: ToolCall,
    pub originating_request_id: String,
}

// Report execution status (sent by your handle_call method)
pub struct ToolCallStatusUpdate {
    pub status: ToolCallStatus,
    pub id: String,
    pub originating_request_id: String,
}
```

## Example: File Reading Tool

Here's a complete example of a file reading tool:

```rust
use wasmind_actor_utils::{
    tools,
    common_messages::tools::{ExecuteTool, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, UIDisplayInfo},
    messages::Message,
};
use serde::{Deserialize, Serialize};

#[derive(tools::macros::Tool)]
#[tool(
    name = "read_file",
    description = "Read contents of a text file",
    schema = r#"{
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "The file path to read"
            }
        },
        "required": ["path"]
    }"#
)]
pub struct ReadFileTool {
    scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadFileParams {
    pub path: String,
}

impl tools::Tool for ReadFileTool {
    fn new(scope: String, _config: String) -> Self {
        Self { scope }
    }

    fn handle_call(&mut self, tool_call: ExecuteTool) {
        // Parse parameters
        let params: ReadFileParams = match serde_json::from_str(&tool_call.tool_call.function.arguments) {
            Ok(params) => params,
            Err(e) => {
                // Send error status with helpful UI info
                let ui_display = UIDisplayInfo {
                    collapsed: "Parameters: Invalid format".to_string(),
                    expanded: Some(format!("Failed to parse parameters: {}", e)),
                };
                
                let error_result = ToolCallResult {
                    content: format!("Parameter parsing error: {}", e),
                    ui_display_info: ui_display,
                };
                
                let status = ToolCallStatusUpdate {
                    status: ToolCallStatus::Done { result: Err(error_result) },
                    id: tool_call.tool_call.id,
                    originating_request_id: tool_call.originating_request_id,
                };
                
                Self::broadcast_common_message(status).unwrap();
                return;
            }
        };
        
        // Read the file using standard library (works in WASM)
        let contents = match std::fs::read_to_string(&params.path) {
            Ok(contents) => contents,
            Err(e) => {
                // Send error status with file error details
                let ui_display = UIDisplayInfo {
                    collapsed: format!("Failed to read {}", params.path),
                    expanded: Some(format!("File: {}\nError: {}", params.path, e)),
                };
                
                let error_result = ToolCallResult {
                    content: format!("Failed to read file: {}", e),
                    ui_display_info: ui_display,
                };
                
                let status = ToolCallStatusUpdate {
                    status: ToolCallStatus::Done { result: Err(error_result) },
                    id: tool_call.tool_call.id,
                    originating_request_id: tool_call.originating_request_id,
                };
                
                Self::broadcast_common_message(status).unwrap();
                return;
            }
        };
        
        // Send success status with file contents
        let ui_display = UIDisplayInfo {
            collapsed: format!("Read {}: {} bytes", params.path, contents.len()),
            expanded: Some(format!(
                "File: {}\nSize: {} bytes\n\nFirst 500 chars:\n{}", 
                params.path,
                contents.len(),
                &contents[..contents.len().min(500)]
            )),
        };
        
        let result = ToolCallResult {
            content: serde_json::to_string(&serde_json::json!({
                "path": params.path,
                "contents": contents
            })).unwrap(),
            ui_display_info: ui_display,
        };
        
        let status = ToolCallStatusUpdate {
            status: ToolCallStatus::Done { result: Ok(result) },
            id: tool_call.tool_call.id,
            originating_request_id: tool_call.originating_request_id,
        };
        
        Self::broadcast_common_message(status).unwrap();
    }
}
```

## Configuration

Add tool actors to your wasmind configuration:

```toml
[[actors]]
name = "execute_bash"
path = "./actors/execute_bash"
enabled = true

[[actors]]
name = "file_reader"
path = "./actors/file_reader"
enabled = true
```

## Language Support

Currently, tool actors are easiest to build in Rust with the provided macros and utilities. We will add support and examples for more languages soon.

## Best Practices

1. **Clear tool names and descriptions** - Help the LLM understand when to use your tool
2. **Validate parameters** - Always validate input before executing
3. **Handle errors gracefully** - Return clear error messages
4. **Document parameter schemas** - Use descriptions in your schema definitions
5. **Keep tools focused** - Each tool should do one thing well

## Next Steps

- See [Examples](./examples.md) for complete tool actor implementations
- Learn about [Message Patterns](./message-patterns.md) for coordination
- Explore existing tool actors in the `/actors` directory
