mod common;

use hive::actors::assistant::{
    Assistant, format_agent_response_success, format_plan_approval_request,
    format_plan_approval_response,
};
use hive::actors::tools::plan_approval::{
    PlanApproval, format_plan_approval_success, format_plan_rejection,
};
use hive::actors::tools::planner::format_planner_success_response;
use hive::actors::tools::spawn_agent::SpawnAgent;
use hive::actors::{
    Actor, ActorMessage, AgentMessageType, AgentStatus, AgentType, InterAgentMessage, Message,
};
use hive::scope::Scope;
use serde_json::json;
use std::time::Duration;
use tokio::sync::broadcast;
use wiremock::MockServer;

/// Test verifies the complete planner approval workflow with expected message sequence:
///
/// SETUP PHASE:
/// - ActorReady messages from assistant, spawn_agent, plan_approval
/// - ToolsAvailable messages (2 tools)
/// - UserContext message with task
///
/// PHASE 1 - SPAWNING AND PLANNING:
/// 1. Manager receives task and calls spawn_agents tool (with wait=true)
/// 2. Manager status → Wait (AgentStatus::Wait via TaskStatusUpdate)
/// 3. AgentSpawned message for child worker
/// 4. Child calls planner tool (AssistantToolCall)
/// 5. PlanUpdated message with new plan
/// 6. Child status → AwaitingPlanApproval (AgentStatus::AwaitingManager via TaskStatusUpdate)
///
/// PHASE 2 - APPROVAL, COMPLETION, AND MANAGER RESUMPTION:
/// 7. Manager receives approval request and status → Processing
/// 8. Manager calls approve_plan tool (AssistantToolCall)
/// 9. Manager receives tool result from approve_plan
/// 10. Manager status → Wait (goes back to waiting after approval)
/// 11. Child receives PlanApproved message (InterAgentMessage)
/// 12. Child calls complete tool (AssistantToolCall)
/// 13. Child status → Done(Ok(...)) (AgentStatus::Done via TaskStatusUpdate)
/// 14. Manager receives system message with child completion status
/// 15. Manager status → Processing (resumes after child completion)
/// 16. Manager calls complete tool to finish overall task
#[tokio::test]
#[cfg_attr(not(feature = "test-utils"), ignore)]
async fn test_wait_child_planner_manager_approves() {
    // Start mock server
    let mock_server = MockServer::start().await;

    // Create shared broadcast channel and scope
    let (tx, mut rx) = broadcast::channel(1000);
    let manager_scope = Scope::new();

    // Create config with mock server URL
    let config = common::create_test_config_with_mock_endpoint(mock_server.uri());

    // Since we're using deterministic scopes, we can predict child scope
    // From the output, we know:
    // - Manager scope is #1: 00000000-0000-0001-0000-000000000000
    // - Child agent scope is #2: 00000000-0000-0002-0000-000000000000
    // - The child agent uses its own scope for API calls (it IS the assistant)
    let actual_child_scope =
        Scope::from_uuid("00000000-0000-0002-0000-000000000000".parse().unwrap());

    // Set up mock sequence for manager - spawn call and later approval
    let agents = vec![common::create_agent_spec(
        "Planning Worker",
        "Create a plan for the task",
        "Worker",
    )];

    // Set up mock sequence for manager agent
    common::create_mock_sequence(
        &mock_server,
        manager_scope,
        "Spawn a worker that will create a plan",
    )
    .responds_with_spawn_agents("chatcmpl-spawn", "spawn_call", agents, true)
    .then_expects_tool_result(
        "spawn_call",
        &format!("Spawned 1 agent: Planning Worker ({})", actual_child_scope),
    )
    .then_system_message(format_plan_approval_request(
        &actual_child_scope.to_string(),
        &TaskAwaitingManager::AwaitingPlanApproval {
            tool_call_id: "plan_call".to_string(),
        },
    ))
    .responds_with_approve_plan(
        "chatcmpl-approval",
        "approval_call_id",
        &actual_child_scope.to_string(),
    )
    .then_expects_tool_result(
        "approval_call_id",
        &format_plan_approval_success(&actual_child_scope),
    )
    .then_system_message(format_agent_response_success(
        &actual_child_scope,
        true,
        "Plan created and executed successfully",
    ))
    .responds_with_complete(
        "chatcmpl-manager-complete",
        "manager_complete_call",
        "Successfully spawned worker, approved plan, and completed task.",
        true,
    )
    .build()
    .await;

    // Set up mock sequence for child agent
    common::create_mock_sequence(
        &mock_server,
        actual_child_scope,
        "Create a plan for the task",
    )
    .responds_with_tool_call(
        "chatcmpl-planner",
        "plan_call",
        "planner",
        json!({
            "action": "create",
            "title": "Task Plan",
            "tasks": ["Analyze requirements", "Design solution", "Implement", "Test"]
        }),
    )
    .then_expects_tool_result(
        "plan_call",
        format_planner_success_response("Task Plan", AgentType::Worker),
    )
    .then_system_message(format_plan_approval_response(true, None))
    .responds_with_complete(
        "sub-agent-complete",
        "complete_call_id",
        "Plan created and executed successfully",
        true,
    )
    .build()
    .await;

    // Create manager assistant with spawn_agent and plan_approval tools
    let manager = Assistant::new(
        config.hive.main_manager_model.clone(),
        tx.clone(),
        manager_scope,
        vec![SpawnAgent::ACTOR_ID, PlanApproval::ACTOR_ID],
        None,
        vec![],
    );

    // Create tools for manager
    let spawn_agent = SpawnAgent::new(config.clone(), tx.clone(), manager_scope);
    let plan_approval = PlanApproval::new(config.clone(), tx.clone(), manager_scope);

    // Start manager actors
    manager.run();
    spawn_agent.run();
    plan_approval.run();

    // Wait for manager setup
    let mut manager_ready = false;
    let mut spawn_ready = false;
    let mut approval_ready = false;
    let mut tools_count = 0;

    // Setup phase: wait for all actors and tools to be ready
    while !manager_ready || !spawn_ready || !approval_ready || tools_count < 2 {
        if let Ok(msg) = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
            let msg = msg.unwrap();
            match &msg.message {
                Message::ActorReady { actor_id } => match actor_id.as_str() {
                    "assistant" => manager_ready = true,
                    "spawn_agent" => spawn_ready = true,
                    "plan_approval" => approval_ready = true,
                    _ => {}
                },
                Message::ToolsAvailable(_) => {
                    tools_count += 1;
                }
                _ => {}
            }
        }
    }

    // Wait for idle and consume it
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _idle_msg = rx.recv().await;

    // Send user input to manager
    tx.send(ActorMessage {
        scope: manager_scope,
        message: Message::UserContext(hive::actors::UserContext::UserTUIInput(
            "Spawn a worker that will create a plan".to_string(),
        )),
    })
    .unwrap();

    // Track workflow state
    let mut spawned_agent_id = None;

    // Phase 1 tracking
    let mut seen_manager_wait = false;
    let mut seen_child_plan = false;
    let mut seen_plan_updated = false;
    let mut seen_manager_awaiting_approval = false;

    // Phase 1: Wait for agent to be spawned and manager to enter wait state
    while let Ok(msg) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
        let msg = msg.unwrap();

        match &msg.message {
            Message::Agent(agent_msg) if agent_msg.agent_id != manager_scope => {
                match &agent_msg.message {
                    AgentMessageType::AgentSpawned { .. } => {
                        assert!(seen_manager_wait);
                        spawned_agent_id = Some(agent_msg.agent_id);
                        assert_eq!(agent_msg.agent_id, actual_child_scope);
                    }
                    AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                        status,
                    }) => {
                        if let AgentStatus::AwaitingManager(
                            TaskAwaitingManager::AwaitingPlanApproval { tool_call_id },
                        ) = status
                        {
                            assert!(seen_child_plan);
                            assert!(seen_plan_updated);
                            seen_manager_awaiting_approval = true;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            Message::Agent(agent_msg) if agent_msg.agent_id == manager_scope => {
                if let AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                    status,
                }) = &agent_msg.message
                {
                    match status {
                        AgentStatus::Wait { .. } => {
                            assert!(spawned_agent_id.is_none());
                            seen_manager_wait = true;
                        }
                        _ => {}
                    }
                }
            }
            Message::AssistantToolCall(tc) if tc.fn_name == "planner" => {
                seen_child_plan = true;
            }
            Message::PlanUpdated(_plan) => {
                assert!(seen_child_plan);
                seen_plan_updated = true;
            }
            _ => {}
        }
    }

    // Phase 1 assertions
    assert!(spawned_agent_id.is_some(), "Child agent should be spawned");
    assert!(seen_manager_wait, "Manager should enter wait state");
    assert!(seen_child_plan, "Child should create a plan");
    assert!(seen_plan_updated, "Child agent should update plan");
    assert!(
        seen_manager_awaiting_approval,
        "Manager should await plan approval"
    );

    // Phase 2: Manager approves plan

    // Phase 2 tracking
    let mut seen_manager_processing = false;
    let mut seen_manager_approval_call = false;
    let mut seen_manager_wait_after_approval = false;
    let mut seen_plan_approved = false;
    let mut seen_child_complete = false;
    let mut seen_manager_resume_processing = false;
    let mut seen_manager_final_complete = false;

    while let Ok(msg) = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
        let msg = msg.unwrap();

        match &msg.message {
            // Manager should transition to Processing when it gets approval request
            Message::Agent(agent_msg) if agent_msg.agent_id == manager_scope => {
                if let AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                    status,
                }) = &agent_msg.message
                {
                    match status {
                        AgentStatus::Processing => {
                            if !seen_manager_processing {
                                assert!(
                                    seen_manager_awaiting_approval,
                                    "Manager should be awaiting approval before processing"
                                );
                                seen_manager_processing = true;
                            }
                        }
                        AgentStatus::Wait { .. } => {
                            if seen_manager_approval_call && !seen_manager_wait_after_approval {
                                assert!(
                                    seen_manager_approval_call,
                                    "Manager should have called approve_plan before going back to wait"
                                );
                                seen_manager_wait_after_approval = true;
                            }
                        }
                        _ => {}
                    }
                }
            }

            // Manager makes approval tool call from LLM response
            Message::AssistantToolCall(tc) if tc.fn_name == "approve_plan" => {
                assert!(
                    seen_manager_processing,
                    "Manager should be processing before calling approve_plan"
                );
                seen_manager_approval_call = true;
            }

            // Child receives approval message and status updates
            Message::Agent(agent_msg) if agent_msg.agent_id == spawned_agent_id.unwrap() => {
                match &agent_msg.message {
                    AgentMessageType::InterAgentMessage(InterAgentMessage::PlanApproved) => {
                        assert!(
                            seen_manager_approval_call,
                            "Manager should have called approve_plan before child receives approval"
                        );
                        seen_plan_approved = true;
                    }
                    AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                        status,
                    }) => match status {
                        AgentStatus::Done(Ok(_)) => {
                            assert!(
                                seen_plan_approved,
                                "Child should have received plan approval before completing"
                            );
                            seen_child_complete = true;
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }

            // Child tool calls (including complete)
            Message::AssistantToolCall(tc) if tc.fn_name == "complete" => {
                // Check if this is manager's final complete call
                if msg.scope == manager_scope && seen_child_complete {
                    assert!(
                        seen_manager_resume_processing,
                        "Manager should have resumed processing before calling final complete"
                    );
                    seen_manager_final_complete = true;
                    break;
                }
            }
            _ => {}
        }

        // Manager resumption logic: After child completion, manager should resume
        if seen_child_complete && !seen_manager_resume_processing {
            // Look for manager resuming processing after receiving child completion message
            if let Message::Agent(agent_msg) = &msg.message {
                if agent_msg.agent_id == manager_scope {
                    if let AgentMessageType::InterAgentMessage(
                        InterAgentMessage::TaskStatusUpdate { status },
                    ) = &agent_msg.message
                    {
                        if matches!(status, AgentStatus::Processing) {
                            seen_manager_resume_processing = true;
                        }
                    }
                }
            }
        }
    }

    // Phase 2 assertions
    assert!(
        seen_manager_processing,
        "Manager should transition to Processing"
    );
    assert!(
        seen_manager_approval_call,
        "Manager should call approve_plan tool"
    );
    assert!(
        seen_manager_wait_after_approval,
        "Manager should go back to Wait state after approval"
    );
    assert!(seen_plan_approved, "Child should receive plan approval");
    assert!(seen_child_complete, "Child should complete task");

    assert!(
        seen_manager_resume_processing,
        "Manager should resume processing after child completion"
    );
    assert!(
        seen_manager_final_complete,
        "Manager should call complete tool to finish overall task"
    );
}

/// Test verifies the planner rejection workflow where manager rejects child's plan:
///
/// SETUP PHASE:
/// - ActorReady messages from assistant, spawn_agent, plan_approval
/// - ToolsAvailable messages (2 tools)
/// - UserContext message with task
///
/// PHASE 1 - SPAWNING AND PLANNING:
/// 1. Manager receives task and calls spawn_agents tool (with wait=true)
/// 2. Manager status → Wait (AgentStatus::Wait via TaskStatusUpdate)
/// 3. AgentSpawned message for child worker
/// 4. Child calls planner tool (AssistantToolCall)
/// 5. PlanUpdated message with new plan
/// 6. Child status → AwaitingPlanApproval (AgentStatus::AwaitingManager via TaskStatusUpdate)
///
/// PHASE 2 - REJECTION AND FINAL STATES:
/// 7. Manager receives approval request and status → Processing
/// 8. Manager calls reject_plan tool (AssistantToolCall)
/// 9. Manager receives tool result from reject_plan
/// 10. Manager status → Wait (goes back to waiting after rejection)
/// 11. Child receives PlanRejected message (InterAgentMessage)
/// 12. Child status → Processing (starts working but doesn't complete)
#[tokio::test]
#[cfg_attr(not(feature = "test-utils"), ignore)]
async fn test_wait_child_planner_manager_rejects() {
    // Start mock server
    let mock_server = MockServer::start().await;

    // Create shared broadcast channel and scope
    let (tx, mut rx) = broadcast::channel(1000);
    let manager_scope = Scope::new();

    // Create config with mock server URL
    let config = common::create_test_config_with_mock_endpoint(mock_server.uri());

    // Since we're using deterministic scopes, we can predict child scope
    let actual_child_scope =
        Scope::from_uuid("00000000-0000-0002-0000-000000000000".parse().unwrap());

    // Set up mock sequence for manager - spawn call and later rejection
    let agents = vec![common::create_agent_spec(
        "Planning Worker",
        "Create a plan for the task",
        "Worker",
    )];

    // Set up mock sequence for manager agent
    common::create_mock_sequence(
        &mock_server,
        manager_scope,
        "Spawn a worker that will create a plan",
    )
    .responds_with_spawn_agents("chatcmpl-spawn", "spawn_call", agents, true)
    .then_expects_tool_result(
        "spawn_call",
        &format!("Spawned 1 agent: Planning Worker ({})", actual_child_scope),
    )
    .then_system_message(format_plan_approval_request(
        &actual_child_scope.to_string(),
        &TaskAwaitingManager::AwaitingPlanApproval {
            tool_call_id: "plan_call".to_string(),
        },
    ))
    .responds_with_reject_plan(
        "chatcmpl-rejection",
        "rejection_call_id",
        &actual_child_scope.to_string(),
        "This plan needs more detail and better structure",
    )
    .then_expects_tool_result(
        "rejection_call_id",
        &format_plan_rejection(
            &actual_child_scope,
            "This plan needs more detail and better structure",
        ),
    )
    .build()
    .await;

    // Set up mock sequence for child agent
    common::create_mock_sequence(
        &mock_server,
        actual_child_scope,
        "Create a plan for the task",
    )
    .responds_with_tool_call(
        "chatcmpl-planner",
        "plan_call",
        "planner",
        json!({
            "action": "create",
            "title": "Task Plan",
            "tasks": ["Analyze requirements", "Design solution", "Implement", "Test"]
        }),
    )
    .then_expects_tool_result(
        "plan_call",
        format_planner_success_response("Task Plan", AgentType::Worker),
    )
    .then_system_message(format_plan_approval_response(
        false,
        Some("This plan needs more detail and better structure"),
    ))
    .build()
    .await;

    // Create manager assistant with spawn_agent and plan_approval tools
    let manager = Assistant::new(
        config.hive.main_manager_model.clone(),
        tx.clone(),
        manager_scope,
        vec![SpawnAgent::ACTOR_ID, PlanApproval::ACTOR_ID],
        None,
        vec![],
    );

    // Create tools for manager
    let spawn_agent = SpawnAgent::new(config.clone(), tx.clone(), manager_scope);
    let plan_approval = PlanApproval::new(config.clone(), tx.clone(), manager_scope);

    // Start manager actors
    manager.run();
    spawn_agent.run();
    plan_approval.run();

    // Wait for manager setup
    let mut manager_ready = false;
    let mut spawn_ready = false;
    let mut approval_ready = false;
    let mut tools_count = 0;

    // Setup phase: wait for all actors and tools to be ready
    while !manager_ready || !spawn_ready || !approval_ready || tools_count < 2 {
        if let Ok(msg) = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
            let msg = msg.unwrap();
            match &msg.message {
                Message::ActorReady { actor_id } => match actor_id.as_str() {
                    "assistant" => manager_ready = true,
                    "spawn_agent" => spawn_ready = true,
                    "plan_approval" => approval_ready = true,
                    _ => {}
                },
                Message::ToolsAvailable(_) => {
                    tools_count += 1;
                }
                _ => {}
            }
        }
    }

    // Wait for idle and consume it
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _idle_msg = rx.recv().await;

    // Send user input to manager
    tx.send(ActorMessage {
        scope: manager_scope,
        message: Message::UserContext(hive::actors::UserContext::UserTUIInput(
            "Spawn a worker that will create a plan".to_string(),
        )),
    })
    .unwrap();

    // Track workflow state
    let mut spawned_agent_id = None;

    // Phase 1 tracking
    let mut seen_manager_wait = false;
    let mut seen_child_plan = false;
    let mut seen_plan_updated = false;
    let mut seen_manager_awaiting_approval = false;

    // Phase 1: Wait for agent to be spawned and manager to enter wait state
    while let Ok(msg) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
        let msg = msg.unwrap();

        match &msg.message {
            Message::Agent(agent_msg) if agent_msg.agent_id != manager_scope => {
                match &agent_msg.message {
                    AgentMessageType::AgentSpawned { .. } => {
                        assert!(
                            seen_manager_wait,
                            "Manager should be waiting before child spawns"
                        );
                        spawned_agent_id = Some(agent_msg.agent_id);
                        assert_eq!(agent_msg.agent_id, actual_child_scope);
                    }
                    AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                        status,
                    }) => {
                        if let AgentStatus::AwaitingManager(
                            TaskAwaitingManager::AwaitingPlanApproval { tool_call_id },
                        ) = status
                        {
                            assert!(
                                seen_child_plan,
                                "Child should have created plan before awaiting approval"
                            );
                            assert!(
                                seen_plan_updated,
                                "Plan should be updated before awaiting approval"
                            );
                            seen_manager_awaiting_approval = true;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            Message::Agent(agent_msg) if agent_msg.agent_id == manager_scope => {
                if let AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                    status,
                }) = &agent_msg.message
                {
                    match status {
                        AgentStatus::Wait { .. } => {
                            assert!(
                                spawned_agent_id.is_none(),
                                "Child should not be spawned before manager waits"
                            );
                            seen_manager_wait = true;
                        }
                        _ => {}
                    }
                }
            }
            Message::AssistantToolCall(tc) if tc.fn_name == "planner" => {
                seen_child_plan = true;
            }
            Message::PlanUpdated(_plan) => {
                assert!(
                    seen_child_plan,
                    "Child should have called planner before plan update"
                );
                seen_plan_updated = true;
            }
            _ => {}
        }
    }

    // Phase 1 assertions
    assert!(spawned_agent_id.is_some(), "Child agent should be spawned");
    assert!(seen_manager_wait, "Manager should enter wait state");
    assert!(seen_child_plan, "Child should create a plan");
    assert!(seen_plan_updated, "Child agent should update plan");
    assert!(
        seen_manager_awaiting_approval,
        "Manager should await plan approval"
    );

    // Phase 2: Manager rejects plan

    // Phase 2 tracking
    let mut seen_manager_processing = false;
    let mut seen_manager_rejection_call = false;
    let mut seen_manager_wait_after_rejection = false;
    let mut seen_plan_rejected = false;
    let mut seen_child_processing = false;

    while let Ok(msg) = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
        let msg = msg.unwrap();

        match &msg.message {
            // Manager should transition to Processing when it gets approval request
            Message::Agent(agent_msg) if agent_msg.agent_id == manager_scope => {
                if let AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                    status,
                }) = &agent_msg.message
                {
                    match status {
                        AgentStatus::Processing => {
                            if !seen_manager_processing {
                                assert!(
                                    seen_manager_awaiting_approval,
                                    "Manager should be awaiting approval before processing"
                                );
                                seen_manager_processing = true;
                            }
                        }
                        AgentStatus::Wait { .. } => {
                            if seen_manager_rejection_call && !seen_manager_wait_after_rejection {
                                assert!(
                                    seen_manager_rejection_call,
                                    "Manager should have called reject_plan before going back to wait"
                                );
                                seen_manager_wait_after_rejection = true;
                                // This is the final state for the manager - check if we can end the test
                                if seen_child_processing {
                                    break;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            // Manager makes rejection tool call from LLM response
            Message::AssistantToolCall(tc) if tc.fn_name == "reject_plan" => {
                assert!(
                    seen_manager_processing,
                    "Manager should be processing before calling reject_plan"
                );
                seen_manager_rejection_call = true;
            }

            // Child receives rejection message and status updates
            Message::Agent(agent_msg) if agent_msg.agent_id == spawned_agent_id.unwrap() => {
                match &agent_msg.message {
                    AgentMessageType::InterAgentMessage(InterAgentMessage::PlanRejected {
                        ..
                    }) => {
                        assert!(
                            seen_manager_rejection_call,
                            "Manager should have called reject_plan before child receives rejection"
                        );
                        seen_plan_rejected = true;
                    }
                    AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                        status,
                    }) => match status {
                        AgentStatus::Processing => {
                            assert!(
                                seen_plan_rejected,
                                "Child should have received plan rejection before processing"
                            );
                            seen_child_processing = true;
                            // This is the final state for the child - check if we can end the test
                            if seen_manager_wait_after_rejection {
                                break;
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            _ => {}
        }
    }

    // Phase 2 assertions
    assert!(
        seen_manager_processing,
        "Manager should transition to Processing"
    );
    assert!(
        seen_manager_rejection_call,
        "Manager should call reject_plan tool"
    );
    assert!(
        seen_manager_wait_after_rejection,
        "Manager should go back to Wait state after rejection"
    );
    assert!(seen_plan_rejected, "Child should receive plan rejection");
    assert!(
        seen_child_processing,
        "Child should end in Processing state"
    );
}

/// Test verifies async plan approval when manager is busy processing:
///
/// SETUP PHASE:
/// - ActorReady messages from assistant, spawn_agent, plan_approval
/// - ToolsAvailable messages (2 tools)
/// - UserContext message with task
///
/// PHASE 1 - SPAWNING WITHOUT WAIT:
/// 1. Manager receives task and calls spawn_agents tool (with wait=false)
/// 2. Manager gets spawn response and makes another LLM request (with 3s delay)
/// 3. Manager status → Processing (busy with delayed request)
/// 4. AgentSpawned message for child worker
/// 5. Child calls planner tool (AssistantToolCall)
/// 6. PlanUpdated message with new plan
/// 7. Child status → AwaitingPlanApproval (while manager is still busy)
///
/// PHASE 2 - ASYNC APPROVAL:
/// 8. Manager finishes delayed request and transitions to Processing (for approval)
/// 9. Manager calls approve_plan tool (AssistantToolCall)
/// 10. Manager receives tool result and goes back to Processing (final state)
/// 11. Child receives PlanApproved message (InterAgentMessage)
/// 12. Child calls complete tool and status → Processing (final state)
#[tokio::test]
#[cfg_attr(not(feature = "test-utils"), ignore)]
async fn test_no_wait_child_planner_async_approval() {
    // Start mock server
    let mock_server = MockServer::start().await;

    // Create shared broadcast channel and scope
    let (tx, mut rx) = broadcast::channel(1000);
    let manager_scope = Scope::new();

    // Create config with mock server URL
    let config = common::create_test_config_with_mock_endpoint(mock_server.uri());

    // Since we're using deterministic scopes, we can predict child scope
    let actual_child_scope =
        Scope::from_uuid("00000000-0000-0002-0000-000000000000".parse().unwrap());

    // Set up mock sequence for manager - spawn call and delayed content response
    let agents = vec![common::create_agent_spec(
        "Background Worker",
        "Create a plan in the background",
        "Worker",
    )];

    // Set up mock sequence for manager agent
    common::create_mock_sequence(
        &mock_server,
        manager_scope,
        "Spawn a worker without waiting",
    )
    .responds_with_spawn_agents("chatcmpl-spawn", "spawn_call", agents, false)
    .then_expects_tool_result(
        "spawn_call",
        &format!(
            "Spawned 1 agent: Background Worker ({})",
            actual_child_scope
        ),
    )
    .responds_with_content_delay(
        "chatcmpl-filler",
        "Working on other important tasks...",
        Some(Duration::from_millis(500)),
    )
    .then_system_message(format_plan_approval_request(
        &actual_child_scope.to_string(),
        &TaskAwaitingManager::AwaitingPlanApproval {
            tool_call_id: "plan_call".to_string(),
        },
    ))
    .responds_with_approve_plan(
        "chatcmpl-approval",
        "approval_call_id",
        &actual_child_scope.to_string(),
    )
    .then_expects_tool_result(
        "approval_call_id",
        &format_plan_approval_success(&actual_child_scope),
    )
    .build()
    .await;

    // Set up mock sequence for child agent
    common::create_mock_sequence(
        &mock_server,
        actual_child_scope,
        "Create a plan in the background",
    )
    .responds_with_tool_call(
        "chatcmpl-planner",
        "plan_call",
        "planner",
        json!({
            "action": "create",
            "title": "Background Task Plan",
            "tasks": ["Analyze task", "Work independently", "Report back"]
        }),
    )
    .then_expects_tool_result(
        "plan_call",
        format_planner_success_response("Background Task Plan", AgentType::Worker),
    )
    .then_system_message(format_plan_approval_response(true, None))
    .responds_with_complete(
        "child-complete",
        "child_complete_call",
        "Background work completed successfully",
        true,
    )
    .build()
    .await;

    // Create manager assistant with spawn_agent and plan_approval tools
    let manager = Assistant::new(
        config.hive.main_manager_model.clone(),
        tx.clone(),
        manager_scope,
        vec![SpawnAgent::ACTOR_ID, PlanApproval::ACTOR_ID],
        None,
        vec![],
    );

    // Create tools for manager
    let spawn_agent = SpawnAgent::new(config.clone(), tx.clone(), manager_scope);
    let plan_approval = PlanApproval::new(config.clone(), tx.clone(), manager_scope);

    // Start manager actors
    manager.run();
    spawn_agent.run();
    plan_approval.run();

    // Wait for manager setup
    let mut manager_ready = false;
    let mut spawn_ready = false;
    let mut approval_ready = false;
    let mut tools_count = 0;

    // Setup phase: wait for all actors and tools to be ready
    while !manager_ready || !spawn_ready || !approval_ready || tools_count < 2 {
        if let Ok(msg) = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
            let msg = msg.unwrap();
            match &msg.message {
                Message::ActorReady { actor_id } => match actor_id.as_str() {
                    "assistant" => manager_ready = true,
                    "spawn_agent" => spawn_ready = true,
                    "plan_approval" => approval_ready = true,
                    _ => {}
                },
                Message::ToolsAvailable(_) => {
                    tools_count += 1;
                }
                _ => {}
            }
        }
    }

    // Wait for idle and consume it
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _idle_msg = rx.recv().await;

    // Send user input to manager
    tx.send(ActorMessage {
        scope: manager_scope,
        message: Message::UserContext(hive::actors::UserContext::UserTUIInput(
            "Spawn a worker without waiting".to_string(),
        )),
    })
    .unwrap();

    // Track workflow state
    let mut spawned_agent_id = None;

    // Phase 1 tracking - no wait spawning
    let mut seen_manager_processing_initial = false;
    let mut seen_child_spawn = false;
    let mut seen_child_plan = false;
    let mut seen_plan_updated = false;
    let mut seen_child_awaiting_approval = false;

    // Phase 2 tracking - async approval
    let mut seen_manager_processing_approval = false;
    let mut seen_manager_approval_call = false;
    let mut seen_plan_approved = false;
    let mut seen_child_processing_final = false;
    let mut seen_manager_processing_final = false;

    // Wait for the complete async workflow
    while let Ok(msg) = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
        let msg = msg.unwrap();

        match &msg.message {
            // Manager status updates
            Message::Agent(agent_msg) if agent_msg.agent_id == manager_scope => {
                if let AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                    status,
                }) = &agent_msg.message
                {
                    match status {
                        AgentStatus::Processing => {
                            if !seen_manager_processing_initial && !seen_child_spawn {
                                // Initial processing after spawn
                                seen_manager_processing_initial = true;
                            } else if seen_child_awaiting_approval
                                && !seen_manager_processing_approval
                            {
                                // Processing for approval after child needs approval
                                seen_manager_processing_approval = true;
                            } else if seen_manager_approval_call && !seen_manager_processing_final {
                                // Final processing state after approval
                                seen_manager_processing_final = true;
                                // Check if we can end the test
                                if seen_child_processing_final {
                                    break;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            // Child agent lifecycle
            Message::Agent(agent_msg) if agent_msg.agent_id != manager_scope => {
                match &agent_msg.message {
                    AgentMessageType::AgentSpawned { .. } => {
                        spawned_agent_id = Some(agent_msg.agent_id);
                        seen_child_spawn = true;
                        assert_eq!(agent_msg.agent_id, actual_child_scope);
                    }
                    AgentMessageType::InterAgentMessage(InterAgentMessage::TaskStatusUpdate {
                        status,
                    }) => {
                        match status {
                            AgentStatus::AwaitingManager(
                                TaskAwaitingManager::AwaitingPlanApproval { .. },
                            ) => {
                                assert!(
                                    seen_child_plan,
                                    "Child should have created plan before awaiting approval"
                                );
                                assert!(
                                    seen_plan_updated,
                                    "Plan should be updated before awaiting approval"
                                );
                                seen_child_awaiting_approval = true;
                            }
                            AgentStatus::Processing => {
                                if seen_plan_approved && !seen_child_processing_final {
                                    seen_child_processing_final = true;
                                    // Check if we can end the test
                                    if seen_manager_processing_final {
                                        break;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    AgentMessageType::InterAgentMessage(InterAgentMessage::PlanApproved) => {
                        assert!(
                            seen_manager_approval_call,
                            "Manager should have called approve_plan before child receives approval"
                        );
                        seen_plan_approved = true;
                    }
                    _ => {}
                }
            }

            // Tool calls
            Message::AssistantToolCall(tc) => match tc.fn_name.as_str() {
                "planner" => {
                    seen_child_plan = true;
                }
                "approve_plan" => {
                    assert!(
                        seen_manager_processing_approval,
                        "Manager should be processing approval before calling approve_plan"
                    );
                    seen_manager_approval_call = true;
                }
                _ => {}
            },

            // Plan updates
            Message::PlanUpdated(_plan) => {
                assert!(
                    seen_child_plan,
                    "Child should have called planner before plan update"
                );
                seen_plan_updated = true;
            }

            _ => {}
        }
    }

    // Phase 1 assertions - no wait spawning
    assert!(spawned_agent_id.is_some(), "Child agent should be spawned");
    assert!(
        seen_manager_processing_initial,
        "Manager should be processing initially"
    );
    assert!(seen_child_spawn, "Child should be spawned");
    assert!(seen_child_plan, "Child should create a plan");
    assert!(seen_plan_updated, "Child agent should update plan");
    assert!(
        seen_child_awaiting_approval,
        "Child should await plan approval"
    );

    // Phase 2 assertions - async approval
    assert!(
        seen_manager_processing_approval,
        "Manager should process approval request"
    );
    assert!(
        seen_manager_approval_call,
        "Manager should call approve_plan tool"
    );
    assert!(seen_plan_approved, "Child should receive plan approval");
    assert!(
        seen_child_processing_final,
        "Child should end in Processing state"
    );
    assert!(
        seen_manager_processing_final,
        "Manager should end in Processing state"
    );
}
