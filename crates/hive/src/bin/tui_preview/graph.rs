use std::time::Duration;

use hive::actors::litellm_manager::LiteLLMManager;
use hive::actors::tui::TuiActor;
use hive::actors::{Actor, AgentType, Message};
use hive::config::{Config, ParsedConfig};
use hive::hive::MAIN_MANAGER_SCOPE;
use tokio::sync::broadcast;
use tracing::info;

use crate::utils::{create_agent_status_update_message, create_spawn_agent_message};

pub async fn run() {
    info!("Starting command execution scenario");

    // Create config
    let config: ParsedConfig = Config::new(false).unwrap().try_into().unwrap();
    let scope = MAIN_MANAGER_SCOPE;

    // Set up broadcast channel
    let (tx, _rx) = broadcast::channel(1000);

    // Create actors
    TuiActor::new(config.clone(), tx.clone(), scope.clone(), None).run();
    // Send the LiteLLM Actor Ready message
    // Need to actually let the dashboard mount first
    tokio::time::sleep(Duration::from_secs(1)).await;
    let _ = tx.send(hive::actors::ActorMessage {
        scope: MAIN_MANAGER_SCOPE.clone(),
        message: Message::ActorReady {
            actor_id: LiteLLMManager::ACTOR_ID.to_string(),
        },
    });
    let _ = tx.send(create_agent_status_update_message(
        &scope,
        hive::actors::AgentStatus::Wait {
            reason: hive::actors::WaitReason::WaitForSystem {
                tool_name: None,
                tool_call_id: "Filler".to_string(),
            },
        },
    ));

    // Spawn some agents
    let (spawn_agent_message, agent1_scope) = create_spawn_agent_message(
        &scope,
        AgentType::SubManager,
        "Filler Role",
        "Manage some workers",
    );
    let _ = tx.send(spawn_agent_message);
    let _ = tx.send(create_agent_status_update_message(
        &agent1_scope,
        hive::actors::AgentStatus::Processing {
            id: uuid::Uuid::new_v4(),
        },
    ));

    let (spawn_agent_message, _) =
        create_spawn_agent_message(&scope, AgentType::Worker, "Filler Role", "Work");
    let _ = tx.send(spawn_agent_message);

    let (spawn_agent_message, _) =
        create_spawn_agent_message(&agent1_scope, AgentType::Worker, "Filler Role", "Work");
    let _ = tx.send(spawn_agent_message);

    let (spawn_agent_message, agent2_scope) = create_spawn_agent_message(
        &agent1_scope,
        AgentType::SubManager,
        "Filler Role",
        "Manage some workers",
    );
    let _ = tx.send(spawn_agent_message);

    let (spawn_agent_message, _) =
        create_spawn_agent_message(&agent2_scope, AgentType::Worker, "Filler Role", "Work");
    let _ = tx.send(spawn_agent_message);

    tokio::time::sleep(Duration::from_secs(10_000)).await;
}
