# Review Plan

*Plan review and feedback workflows for AI agents*

Review Plan adds a review layer to task planning, ensuring that proposed plans are reviewed by designated experts before execution.

## Overview

Review Plan helps maintain quality when AI agents create execution plans. The `request_plan_review` actor provides the `request_plan_review` tool that:

- **Expert review**: Spawns configured expert agents to review proposed plans
- **Consolidated feedback**: Collects and combines feedback from all reviewers
- **Pre-execution validation**: Ensures plans are vetted before agents begin work

The `request_plan_review` actor spawns review agents when called and waits for all responses before returning consolidated feedback.

## Architecture

### How It Works

1. **Plan Review Request**: An agent calls the `request_plan_review` tool with a task and proposed plan
2. **Expert Spawning**: The actor spawns the configured expert agents to review
3. **Feedback Collection**: Each expert uses the `review_plan` tool to provide feedback
4. **Consolidation**: The `request_plan_review` tool waits for all experts, then returns combined feedback
5. **Refinement**: The requesting agent can refine their plan based on the feedback

### Configuration

The `request_plan_review` actor is configured with:

- **reviewers**: Map of reviewer names to their actor configurations

## Actors Included

Review Plan consists of 2 actors:

### Core Actor
- **`request_plan_review`** - Provides the `request_plan_review` tool, spawns review agents and consolidates feedback

### Review Tool
- **`review_plan`** - Allows experts to provide detailed feedback on submitted plans - this is automatically spawned in the scope of the expert reviewers

## Usage

To use Review Plan, configure the `request_plan_review` actor:

```toml
[actors.request_plan_review]
source = { url = "https://github.com/SilasMarvin/wasmind", package = "actors/review_plan/crates/request_plan_review" }

[actors.request_plan_review.config.reviewers]
# Define expert reviewers - each is an agent that is spawned for each request_plan_review tool call with the provided list of actors
# It should at the minimum have some "assistant" and most likely have a way to explore the codebase
architecture_expert = ["architecture_expert_assistant", "read_file", "execute_bash", ...]
security_expert = ["security_expert_assistant", "read_file", "execute_bash", ...]
```

### Configuration Explained

- **reviewers**: Each entry spawns a separate expert agent for review
  - Key: Name identifier for the expert
  - Value: List of actors to spawn for that expert (typically includes an "assistant" and tools to explore the codebase)

## Example Workflows

### Plan Review Process
1. Agent calls `request_plan_review` with a task description and proposed plan
2. `request_plan_review` spawns the configured expert agents
3. Each expert reviews the plan and uses `review_plan` to provide feedback
4. `request_plan_review` waits for all experts to respond
5. Feedback is consolidated and returned to the requesting agent
6. Agent refines their plan based on the feedback before execution

---

*This README is part of the Wasmind actor system. For more information, see the main project documentation.*
