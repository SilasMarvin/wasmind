# Review Plan Actor

*Tool for providing feedback on submitted plans*

This actor provides the `review_plan` tool used by expert reviewers to provide detailed feedback on submitted task plans. It is automatically spawned as a dependency when plan review is requested and is not intended for standalone use.

## Actor ID
`rpr__review_plan`

## Tools Provided

### `review_plan`
- **Description**: Provide feedback on the submitted plan
- **Parameters**:
  - `feedback`: Detailed feedback on the plan - what works well, potential issues, suggestions for improvement (required)
- **Usage**: Used by expert agents to provide constructive feedback on task plans

## Configuration

This actor requires no configuration. It is automatically spawned as a dependency by the `request_plan_review` actor.

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*