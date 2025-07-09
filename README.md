## HIVE System Migration Plan

```
tree src/
src/
├── actors
│   ├── assistant.rs
│   ├── context.rs
│   ├── microphone.rs
│   ├── mod.rs
│   ├── tools
│   │   ├── command.rs
│   │   ├── edit_file.rs
│   │   ├── file_reader.rs
│   │   ├── mcp.rs
│   │   ├── mod.rs
│   │   └── planner.rs
│   └── tui
│       ├── app.rs
│       ├── events.rs
│       ├── mod.rs
│       ├── ui.rs
│       └── widgets.rs
├── cli.rs
├── config.rs
├── input
│   └── mod.rs
├── key_bindings.rs
├── main.rs
├── prompt_preview.rs
├── system_state.rs
├── template.rs
└── worker.rs
```


**Overall Goal:** Migrate the existing single-agent TUI LLM chat and tool-calling system to a multi-agent system named "HIVE." This system will feature a hierarchy of "Manager Agents" that can delegate tasks to specialized "Worker Agents" or other "Manager Agents."

**Core Concepts:**

1.  **Agent:** The fundamental actor unit in HIVE. An agent can be either a `ManagerAgent` or a `WorkerAgent`. Each agent has a unique ID. Things like creating the unique id, etc.. are handled by the system. The LLM interats with the "system" via function calls.
2.  **Manager Agent (Manager):**
    *   Responsible for receiving tasks (from a user or another Manager), planning, breaking down complex tasks, creating new tasks, and delegating them to Worker Agents or other Manager Agents.
    *   The "Main Manager" is the top-level manager the user interacts with directly.
    *   Managers **do not** directly execute tools like `file_reader` or `command`. Their "tools" are primarily for planning, task creation, and agent spawning/management.
3.  **Worker Agent (Worker):**
    *   Responsible for executing a specific task assigned by a Manager.
    *   Has a specific `role` (e.g., "Software Engineer," "Researcher") that informs its capabilities and system prompt.
    *   Can propose a `plan` for its task to the Manager for approval if the task is complex.
    *   Executes allowed tools (e.g., `file_reader`, `command`, `edit_file`) based on a whitelist, similar to the current system.
4.  **Task:** A unit of work with a description, assigned agent, status (e.g., `AwaitingManager: Enum of (AwaitingPlanApproval, AwaitingMoreInformation)` `InProgress`, `Completed`, `Failed`), and potentially resulting artifacts.
5.  **Plan:** A sequence of steps proposed by an agent (Manager or Worker) to accomplish a task. Plans from Workers require Manager approval.
6.  **System Prompt:** Each agent receives a tailored system prompt.
    *   **Main Manager:** "You are the HIVE Main Manager. Your primary role is to understand user requests, formulate plans, break them down into actionable tasks, and delegate these tasks to specialized Worker Agents or Sub-Manager Agents. You can create plans and manage tasks. You do not execute tools like file readers or command execution directly."
    *   **Sub-Manager:** "You are a HIVE Manager Agent. Your role is: `{sub_manager_role}`. You receive tasks from your supervising manager. Your responsibility is to break these tasks down, create sub-tasks, and assign them to appropriate Worker Agents or other Manager Agents you spawn. You can create plans for your assigned objectives. You do not execute tools like file readers or command execution directly."
    *   **Worker Agent:** "You are a HIVE Worker Agent. Your designated role is: `{worker_role}`. Your current task is: `{task_description}`. You can use available tools to accomplish this task. If the task is complex, you can propose a plan to your Manager for approval. Report your progress, any issues, or task completion to your Manager."

---

**I. Codebase Restructure & New Modules:**

1.  **Create `src/actors/agent.rs`:**
    *   Define a public enum `AgentType` or similar: `enum AgentBehavior { Manager(ManagerLogic), Worker(WorkerLogic) }`.
    *   This module will house the core logic for both Manager and Worker agents.
    *   First thoroughly explore the `src/worker.rs` and `src/actors/mod.rs`

2.  **Define Agent Structures:**
    *   **`ManagerLogic` struct:**
        *   `id: AgentId` (unique identifier)
        *   `role: String` (e.g., "Main Manager", "Project Lead Manager")
    *   **`WorkerLogic` struct:**
        *   `id: AgentId`
        *   `role: String` (e.g., "Software Engineer")
    *   Both agents should create their own set of actors as seen in the `src/worker.rs`. They will also receive a channel from their manager and should broadcast status change updates back.

3.  **Actor Spawning and Management:**
    *   Managers will spawn new Worker Agents or other Manager Agents. Each spawned agent will run in its own Tokio task.

**II. Manager Agent Functionality:**

1.  **Task Creation and Assignment (Unified Function Call):**
    *   Managers will use a function:
        `spawn_agent_and_assign_task(agent_role: String, task_description: String, agent_type: enum { Worker, Manager }, wait: bool) -> Result<AgentSpawnedResponse, Error>`
    *   **Process:**
        1.  Manager's LLM decides to delegate and formulates this function call.
        2.  The HIVE system receives this call.
        3.  A new unique `AgentId` and `TaskId` are generated.
        4.  A new agent (Worker or Manager type) is instantiated with the specified `agent_role`.
        5.  The `task_description` is encapsulated in a `Task` object, associated with the new agent, and given an initial status (e.g., `InProgress` for worker, `PendingDelegation` for new manager).
        6.  The new agent's system prompt is constructed using its `agent_role` and `task_description` (for Workers) or just `agent_role` (for Managers, who will then be told their overall objective by the spawning Manager).
        7.  **If `wait` is `false`:** The Manager receives an `AgentSpawnedResponse { agent_id, task_id, agent_role }` message immediately, and its LLM interaction can continue.
        8. **If `wait` is `true`:** The Manager's LLM interaction is paused. When a spawned agent (or the user) sends a message, the Manager's LLM interaction is resumed with that context.

2.  **Planning Tool:**
    *   Managers can create high-level plans. This tool is similar to the existing planner but might be simpler as detailed execution is delegated.
    *   When a sub manager or worker creates a plan, its manager must approve that plan. This does not pertain to the very first manager we spawn.
    *   See the `planner.rs` for more info on how the planner tool works.

3.  **Task Management Tools (for internal HIVE system, callable by Manager LLM):**
    *   `approve_plan(task_id: TaskId, plan_id: PlanId) -> PlanApprovalResponse`
    *   `reject_plan(task_id: TaskId, plan_id: PlanId, reason: String) -> PlanRejectionResponse`
    *   This is used by manager's to approve or reject plans.

4.  **State Monitoring and Notifications:**
    *   Managers automatically monitor the state of tasks assigned to their child agents.
    *   When a child agent sends a message (e.g., `TaskCompleted`, `PlanSubmittedForApproval`), we update the internal state of the task tool and prompt the manager. See how the `planner.rs` works in combination with the `system_state.rs`.
    *   The Manager's LLM is prompted/informed when a task it's overseeing is completed or requires its attention (e.g., plan approval). This should be formatted clearly, presenting the current state of relevant tasks.

**III. Worker Agent Functionality:**

1.  **Task Execution:**
    *   Receives a task from its Manager.
    *   Uses its LLM, role, task description, and available tools to work towards task completion.
2.  **Plan Proposal:**
    *   If a task is complex, the Worker can formulate a plan (list of sub-steps, potentially involving tool calls) see `planner.rs`. When a plan is created we automatically update the status to `PendingPlanApproval` and prompt the manager to approve or reject this plan.
    *   Waits for Manager's response (`PlanApproved` or `PlanRejected`) before proceeding with that plan.
3.  **Tool Execution:**
    *   Can execute tools from the existing toolset (`command.rs`, `edit_file.rs`, `file_reader.rs`, etc.).
    *   Tool execution must follow the established whitelist logic, potentially configured per-role or per-agent instance.
4.  **Communication with Manager:**
    * Task status updates are sent from the sub agent to the controlling agent. See the next section.


**IV. Communication Protocol (Agent Messages):**

There is an existing enum messaging system in the `src/actors/mod.rs` file for communication inside of an agent. 

We need to add another communication enum type for communicating outside of an individual agent: manager -> worker and worker -> manger.

Something like:

```rust

enum TaskAwaitingManager {
    AwaitingPlanApproval(Plan),
    AwaitingMoreInformation(String)
}

enum TaskStatus {
    Done(Result<String, String>),
    InProgress,
    AwaitingManager(TaskAwaitingManager)
}
```

**V. Tool Access Control:**

1.  **Managers:**
    *   Allowed "tools":
        *   `spawn_agent_and_assign_task`
        *   `create_plan`
        *   `approve_plan`
        *   `reject_plan`
    *   **Not allowed:** Direct execution tools like `file_reader`, `command`, `edit_file`.
2.  **Workers:**
    *   Allowed tools will be based on the existing whitelist mechanism, potentially refined by `role`.
    *   Access to `file_reader`, `command`, `edit_file`, `mcp`, `planner` (for its own sub-planning, which then gets submitted for approval).
