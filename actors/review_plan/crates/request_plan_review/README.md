# Request Plan Review Actor

*Tool for requesting expert review of task plans before execution*

This actor provides the `request_plan_review` tool that enables AI agents to submit their task plans for expert review before beginning execution. It spawns configured expert agents to provide feedback and consolidates their responses.

## Actor ID
`wasmind:request_plan_review`

## Tools Provided

### `request_plan_review`
- **Description**: Submit a task and plan for review by expert reviewers
- **Parameters**:
  - `task`: Description of what you are trying to accomplish (required)
  - `plan`: Your proposed plan for accomplishing the task (required)
- **Usage**: Get expert feedback on plans before execution to catch potential issues

## Configuration

Requires configuration to specify expert reviewers:

```toml
[actors.request_plan_review]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/review_plan/crates/request_plan_review" }

[actors.request_plan_review.config.reviewers]
expert_name = ["assistant", "read_file", "execute_bash", ...]
```

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*