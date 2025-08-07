# Hive LLM Types

Type definitions and utilities for Large Language Model integration in Hive actors. This crate provides standardized types for chat messages, tool calls, and LLM API interactions.

[![docs.rs](https://docs.rs/hive_llm_types/badge.svg)](https://docs.rs/hive_llm_types)

## What This Crate Provides

**Chat Message Types**: Standardized representations for conversation flow:
- `ChatMessage` - Enum covering all message types (System, User, Assistant, Tool)
- `AssistantChatMessage` - Assistant responses with tool calls, reasoning, and thinking blocks
- `SystemChatMessage`, `UserChatMessage`, `ToolChatMessage` - Specific message types

**Tool Integration**: Types for function calling and tool execution:
- `Tool` - Tool definitions with JSON schema for parameters
- `ToolCall` - Function call requests from LLMs
- `ToolFunctionDefinition` - Function metadata and parameter schemas

**API Interaction**: Types for LLM service communication:
- `ChatRequest` - Complete request structure for LLM APIs
- `ChatResponse` - Response parsing with usage metrics
- `Choice` - Individual response options from LLM providers

**Advanced Features**: Support for modern LLM capabilities:
- **Reasoning content** - o1-style reasoning traces
- **Thinking blocks** - Claude-style thinking annotations with signatures
- **Provider-specific fields** - Extensible structure for different LLM providers

## Usage

```rust
use hive_llm_types::types::{ChatMessage, Tool, ToolCall};

// Create chat messages
let system_msg = ChatMessage::system("You are a helpful assistant");
let user_msg = ChatMessage::user("Hello!");

// Handle tool calls
let assistant_msg = ChatMessage::assistant_with_tools(vec![tool_call]);

// Define tools for LLM function calling
let tool = Tool {
    r#type: "function".to_string(),
    function: ToolFunctionDefinition { /* ... */ }
};
```

These types are used throughout the Hive actor ecosystem for consistent LLM integration.

## Links

- **🎭 [Assistant Actor](../../actors/assistant/)** - Primary user of these types for LLM interactions
- **🔧 [Actor Utils](../hive_actor_utils/)** - Common message types that use these LLM types
- **📖 [API Documentation](https://docs.rs/hive_llm_types)** - Complete API reference