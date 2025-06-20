// Integration tests for spawn agent functionality
// Tests are organized by scenario type as documented in spawn_agent_integration/README.md

#[path = "spawn_agent_integration/basic_scenarios.rs"]
mod basic_scenarios;

// Future test modules:
// #[path = "spawn_agent_integration/plan_approval.rs"]
// mod plan_approval;
// #[path = "spawn_agent_integration/info_request.rs"]
// mod info_request;
// #[path = "spawn_agent_integration/multiple_agents.rs"]
// mod multiple_agents;
// #[path = "spawn_agent_integration/nested_spawning.rs"]
// mod nested_spawning;