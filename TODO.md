# Agent-Manager Message System Refactoring

## Overview
This is a major refactoring of the HIVE multi-agent message passing system. The goal is to simplify and unify how agents and managers communicate by:
1. Removing the `AwaitingManager` agent status
2. Enhancing the `Wait` status to include a reason for waiting
3. Consolidating message tools into `send_message` (for managers) and `send_manager_message` (for agents)
4. Supporting both synchronous (wait for response) and asynchronous (fire-and-forget) messaging

## Key Design Decisions
- **NO backwards compatibility** - This is a breaking change
- Plan approval workflow remains unchanged (handled automatically by the system)
- Messages use specific XML-like formats: `<manager_message>` and `<sub_agent_message agent_id="{id}">`
- Both tools support a `wait` parameter that determines if the sender blocks for a response

## Implementation Steps

### Step 1: Update AgentStatus Enum
**File**: `src/actors/mod.rs`

1. Remove `AwaitingManager` variant from `AgentStatus` enum
2. Update `Wait` variant to include a reason:
```rust
pub enum AgentStatus {
    Idle,
    Processing,
    AwaitingTools { pending_tool_calls: Vec<String> },
    Wait { tool_call_id: String, reason: WaitReason },
    Done(AgentTaskResult),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WaitReason {
    WaitingForAgentResponse { agent_id: Scope },
    WaitingForManagerResponse,
    WaitingForPlanApproval,
}
```

3. Remove `TaskAwaitingManager` enum entirely

### Step 2: Update Assistant State Handling
**File**: `src/actors/assistant.rs`

1. Remove all handling of `AwaitingManager` status
2. Update all `Wait` status creation to include appropriate `WaitReason`
3. Remove the special case handling for `TaskAwaitingManager` in tool response processing
4. Update state transition logic:
   - When receiving a message while in `Wait` state, check the `reason` to determine if this message resolves the wait
   - For `WaitingForAgentResponse`, check if message is from the expected agent
   - For `WaitingForManagerResponse`, check if message is from manager
   - For `WaitingForPlanApproval`, keep existing plan approval logic

### Step 3: Update InterAgentMessage
**File**: `src/actors/mod.rs`

Keep the existing `InterAgentMessage` enum but ensure:
- `ManagerMessage` is used for manager->agent communication
- Add a new variant for agent->manager communication:
```rust
pub enum InterAgentMessage {
    TaskStatusUpdate { status: AgentStatus },
    PlanApproved,
    PlanRejected { reason: String },
    ManagerMessage { message: String },
    SubAgentMessage { message: String }, // New: for agent->manager
}
```

### Step 4: Remove Old Tools
1. Delete `src/actors/tools/request_information.rs`
2. Delete `src/actors/tools/send_information.rs`
3. Remove references from `src/actors/tools/mod.rs`
4. Remove tool IDs from agent tool lists in `src/actors/agent.rs`

### Step 5: Create send_message Tool (for Managers)
**File**: `src/actors/tools/send_message.rs`

```rust
pub const SEND_MESSAGE_TOOL_NAME: &str = "send_message";
pub const SEND_MESSAGE_TOOL_DESCRIPTION: &str = "Send a message to a subordinate agent";
pub const SEND_MESSAGE_TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "agent_id": {
            "type": "string",
            "description": "The ID of the agent to send the message to"
        },
        "message": {
            "type": "string",
            "description": "The message to send"
        },
        "wait": {
            "type": "boolean",
            "description": "Whether to wait for a response from the agent"
        }
    },
    "required": ["agent_id", "message", "wait"]
}"#;
```

Tool behavior:
- Send `InterAgentMessage::ManagerMessage` to the specified agent
- If `wait=true`, set manager status to `Wait { tool_call_id, reason: WaitReason::WaitingForAgentResponse { agent_id } }`
- If `wait=false`, complete the tool call immediately

### Step 6: Create send_manager_message Tool (for Agents)
**File**: `src/actors/tools/send_manager_message.rs`

```rust
pub const SEND_MANAGER_MESSAGE_TOOL_NAME: &str = "send_manager_message";
pub const SEND_MANAGER_MESSAGE_TOOL_DESCRIPTION: &str = "Send a message to your manager";
pub const SEND_MANAGER_MESSAGE_TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "message": {
            "type": "string",
            "description": "The message to send to your manager"
        },
        "wait": {
            "type": "boolean",
            "description": "Whether to wait for a response from the manager"
        }
    },
    "required": ["message", "wait"]
}"#;
```

Tool behavior:
- Send `InterAgentMessage::SubAgentMessage` to the parent manager
- If `wait=true`, set agent status to `Wait { tool_call_id, reason: WaitReason::WaitingForManagerResponse }`
- If `wait=false`, complete the tool call immediately

### Step 7: Update Message Handling in Assistant
**File**: `src/actors/assistant.rs`

1. Handle `InterAgentMessage::SubAgentMessage`:
   - Format as `<sub_agent_message agent_id="{agent_id}">message</sub_agent_message>`
   - If manager is in `Wait` state with `WaitingForAgentResponse` for this agent, complete the wait
   - Otherwise, queue the message

2. Update `InterAgentMessage::ManagerMessage` handling:
   - Keep existing format `<manager_message>message</manager_message>`
   - If agent is in `Wait` state with `WaitingForManagerResponse`, complete the wait
   - Otherwise, queue the message

### Step 8: Update Tool Registration
**File**: `src/actors/agent.rs`

1. For managers (MainManager, SubManager):
   - Remove `RequestInformation::ACTOR_ID` and `SendInformation::ACTOR_ID`
   - Add `SendMessage::ACTOR_ID`

2. For workers:
   - Remove `RequestInformation::ACTOR_ID`
   - Add `SendManagerMessage::ACTOR_ID`

### Step 9: Update System State
**File**: `src/system_state.rs`

Update any display logic that shows agent status to handle the new `WaitReason` enum.

### Step 10: Update Assistant Tests
**File**: `src/actors/assistant.rs` (test module)

1. Update all tests that use `AwaitingManager` to use `Wait` with appropriate reason
2. Remove tests specific to `AwaitingManager` behavior
3. Add new tests for:
   - Manager sending message with `wait=true` and receiving response
   - Agent sending message with `wait=true` and receiving response
   - Non-waiting message scenarios
   - Multiple agents messaging simultaneously

### Step 11: Update Tool Integration Tests
**File**: `tests/tools_integration.rs`

1. Remove `test_request_information_tool`
2. Remove `test_send_information_tool`
3. Add `test_send_message_tool` (manager perspective)
4. Add `test_send_manager_message_tool` (agent perspective)
5. Add tests for wait/no-wait scenarios

### Step 12: Update Spawn Agent Tests
**Files**: `tests/spawn_agent_plan_approval.rs`, `tests/spawn_agent_info_request.rs`

1. Update mocks to use new message tools
2. Update expected message formats
3. Update state assertions to check for `Wait` with correct `WaitReason`
4. The plan approval tests should mostly remain unchanged since plan approval is handled automatically

## Migration Notes

### Common Patterns to Update

1. **Replace AwaitingManager checks**:
```rust
// OLD
if let AgentStatus::AwaitingManager(task) = status { ... }

// NEW
if let AgentStatus::Wait { reason: WaitReason::WaitingForManagerResponse, .. } = status { ... }
```

2. **Replace request_information tool calls**:
```rust
// OLD
tool_call("request_information", json!({ "request": "Need info" }))

// NEW
tool_call("send_manager_message", json!({ "message": "Need info", "wait": true }))
```

3. **Replace send_information tool calls**:
```rust
// OLD
tool_call("send_information", json!({ "agent_id": "...", "message": "Info" }))

// NEW
tool_call("send_message", json!({ "agent_id": "...", "message": "Info", "wait": false }))
```

## Testing Strategy

1. **Unit tests**: Ensure state transitions work correctly with new `Wait` reasons
2. **Integration tests**: Test full message round-trips between agents and managers
3. **Plan approval tests**: Verify these still work unchanged
4. **Concurrent messaging**: Test multiple agents messaging simultaneously
5. **Edge cases**: Test timeout scenarios, invalid agent IDs, etc.

## Success Criteria

- All existing tests pass after refactoring
- Code is simpler with fewer special cases
- Message passing is more flexible (sync/async options)
- Plan approval workflow remains unchanged
- Clear separation between manager->agent and agent->manager communication