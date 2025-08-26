# Code with Experts

*File editing with expert review and approval workflows*

Code with Experts adds an approval layer to file editing operations, ensuring that significant code changes are reviewed by designated experts before being applied.

## Overview

Code with Experts maintains code quality when AI agents make file edits. The `file_interaction_with_approval` actor provides `edit_file` and `read_file` tools with these features:

- **Automatic review triggers**: File edits above a configurable size threshold automatically trigger expert review
- **Expert approval workflow**: Designated experts can approve changes or request modifications  
- **Transparent feedback**: Expert decisions are included in the tool response

The `file_interaction_with_approval` actor spawns review agents when needed and waits for all responses before proceeding.

## Architecture

### How It Works

1. **File Edit Request**: An agent calls the `edit_file` tool provided by `file_interaction_with_approval`
2. **Threshold Check**: The actor checks if the diff size exceeds the configured minimum
3. **Expert Review**: For large changes, the actor spawns the configured expert agents to review the proposed edits
4. **Decision**: Each expert uses either the `approve` or `request_changes` tool
5. **Application**: The `edit_file` tool waits for all experts, then either applies approved changes or returns the requested modifications

### Configuration

The `file_interaction_with_approval` actor is configured with:

- **approvers**: Each entry spawns a separate expert agent for review
  - Key: Name identifier for the expert
  - Value: List of actors to spawn for that review (typically includes an "assistant" and tools to explore the codebase)
- **min_diff_size**: Minimum diff size (in characters) that triggers review (default: 20)

## Actors Included

Code with Experts consists of 3 actors:

### Core Actor
- **`file_interaction_with_approval`** - Provides `edit_file` and `read_file` tools, spawns review agents for edits above threshold

### Review Tools
- **`approve`** - Allows experts to approve proposed file changes
- **`request_changes`** - Enables experts to request modifications with specific feedback

## Usage

To use Code with Experts, configure the `file_interaction_with_approval` actor:

```toml
[actors.file_interaction_with_approval]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/code_with_experts/crates/file_interaction_with_approval" }

[actors.file_interaction_with_approval.config]
min_diff_size = 50  # Characters threshold for triggering review

[actors.file_interaction_with_approval.config.approvers]
# Define expert reviewers - each is an agent that is spawned for each edit_file tool call with the provided list of actors
# It should at the minimum have some "assistant" and most likely have a way to explore the codebase
code_expert = ["code_expert_assistant", "read_file", "execute_bash", ...]
senior_dev = ["senior_dev_assistant", "read_file", "execute_bash", ...]
```

### Configuration Explained

- **min_diff_size**: Changes with fewer characters than this bypass review and are applied directly
- **approvers**: Each entry spawns a separate expert agent for review
  - Key: Name identifier for the expert
  - Value: List of actors to spawn for that expert (typically includes some "assistant" and tools to read files / explore the codebase)

## Example Workflows

### Small Edit (No Review Required)
1. Agent uses `edit_file` to make a small change (< threshold)
2. Change is applied immediately without review
3. Agent receives success confirmation

### Large Edit (Expert Review)
1. Agent uses `edit_file` to make a significant change (> threshold)
2. `file_interaction_with_approval` spawns the configured expert agents
3. Each expert reviews the diff and either:
   - Uses `approve` to approve the changes
   - Uses `request_changes` to ask for modifications
4. `edit_file` waits for all experts to respond:
   - If all approve: changes are applied
   - If any request changes: feedback is consolidated and returned
5. Agent receives the `edit_file` result with expert feedback

## Actor Communication Flow

Code with Experts uses Wasmind's message passing for coordination:

### Tool Discovery
- `file_interaction_with_approval` broadcasts available file tools (`read_file`, `edit_file`)
- `approve` and `request_changes` actors broadcast their tools to expert agents

### Review Process
- Expert agents receive diffs as system messages
- Experts use `approve` or `request_changes` tools to respond
- `ApprovalResponse` messages coordinate decisions between actors

### Status Updates
- Tool call status updates show review progress (e.g., "Waiting for experts: 1/2")
- Final results include expert feedback directly in the response

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*
