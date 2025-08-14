# Conversation Compaction Actor

The Conversation Compaction Actor automatically monitors token usage and compacts conversation history when it exceeds configured thresholds, preventing context overflow while preserving essential task state through LLM-powered summarization.

## Actor ID
`wasmind:conversation_compaction`

## Configuration

```toml
[conversation_compaction]
token_threshold = 50000  # Trigger compaction when total tokens exceed this limit
model_name = "gpt-4o-mini"  # Model to use for generating conversation summaries
```

### Configuration Options

- **`token_threshold`**: Total token count that triggers compaction. Set based on your model's context window and desired safety margin.
- **`model_name`**: The LLM model used for summarization. Choose a capable model that can understand and distill complex conversations.

**IMPORTANT**: This actor has no default configuration. You must specify a config.

## How It Works

The Conversation Compaction Actor operates as a background monitor that:

1. **Tracks Token Usage**: Monitors every `Response` message from the assistant to accumulate total token count
2. **Stores Chat State**: Maintains the latest conversation state from `ChatStateUpdated` messages  
3. **Triggers Compaction**: When tokens exceed the threshold, initiates the compaction process
4. **Generates Summary**: Uses an LLM to analyze the conversation and create a structured summary
5. **Resets Conversation**: Replaces the entire chat history with a single user message containing the summary

## Messages Listened For

### From Any Scope

- **`litellm::BaseUrlUpdate`**: Receives the LiteLLM service URL for making compaction requests
  - Stores the base URL needed for API calls to generate summaries

### From Own Scope Only

- **`assistant::ChatStateUpdated`**: Receives updates to the current conversation state
  - Maintains a complete picture of the conversation for compaction
  - Includes system prompt, messages, and tool interactions

- **`assistant::Response`**: Monitors assistant responses to track token usage
  - Extracts token count from the `usage` field
  - Accumulates total tokens across the conversation
  - Triggers compaction when threshold is exceeded

## Messages Broadcast

- **`assistant::InterruptAndForceStatus`**: Pauses the assistant during compaction
  - Sets status to `CompactingConversation` to signal the compaction is starting
  - Other actors can listen for this status change to perform cleanup (like clearing file caches)

- **`assistant::CompactedConversation`**: Replaces the conversation history with a summary
  - Contains a single user message with the conversation summary
  - Triggers assistant to replace its entire chat history
  - Assistant automatically continues processing with the new summary context

## Compaction Process

When token threshold is exceeded, the actor:

### 1. Sets Compaction Status
```rust
InterruptAndForceStatus {
    agent: scope,
    status: Wait {
        reason: CompactingConversation
    }
}
```
This signals to other actors (like file_interaction) that compaction is starting so they can clear caches.

### 2. Generates Summary
The actor uses an LLM prompt that:
- Analyzes the complete conversation transcript
- Extracts only essential information for task continuation
- Formats output as a structured state summary
- Focuses on actionable information over conversation history

### 3. Replaces Conversation History
```rust
CompactedConversation {
    agent: scope,
    messages: vec![User("Below is the current state from the last task...\n\n<current_state_summary>...")]
}
```

## Error Handling

If compaction fails (network error, LLM unavailable, etc.), the actor:
1. Logs the error with details
2. Releases the assistant from waiting state
3. Allows conversation to continue (may hit token limits)
4. Will retry compaction on next threshold crossing

## Building

To build the Conversation Compaction Actor WASM component:

```bash
cargo component build
```

This generates `target/wasm32-wasip1/debug/conversation_compaction.wasm` for use in the Wasmind system.

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*
