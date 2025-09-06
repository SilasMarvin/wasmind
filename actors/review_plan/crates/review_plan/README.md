# Review Plan Actor

*Tool for providing feedback on submitted plans*

This actor provides the `review_plan` tool used by expert reviewers to provide detailed feedback on submitted task plans. It is automatically spawned as a dependency when plan review is requested and is not intended for standalone use.

## Tools Provided

### `review_plan`
- **Description**: Provide feedback on the submitted plan
- **Parameters**:
  - `feedback`: Detailed feedback on the plan - what works well, potential issues, suggestions for improvement (required)
- **Usage**: Used by expert agents to provide constructive feedback on task plans

## Configuration

No configuration required. Include this actor to provide the review_plan tool:

```toml
[actors.review_plan]
source = { git = "https://github.com/SilasMarvin/wasmind", sub_dir = "actors/review_plan/crates/review_plan" }
```

Note: This actor is typically automatically spawned as a dependency by the `request_plan_review` actor.

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*
