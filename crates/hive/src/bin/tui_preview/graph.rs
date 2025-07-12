use std::time::Duration;

use hive::actors::tui::TuiActor;
use hive::actors::{Actor, AgentType};
use hive::config::{Config, ParsedConfig};
use hive::hive::MAIN_MANAGER_SCOPE;
use tokio::sync::broadcast;
use tracing::info;

use crate::utils::create_spawn_agent_message;

pub async fn run() {
    info!("Starting command execution scenario");

    // Create config
    let config: ParsedConfig = Config::new(true).unwrap().try_into().unwrap();
    let scope = MAIN_MANAGER_SCOPE;

    // Set up broadcast channel
    let (tx, _rx) = broadcast::channel(1000);

    // Create actors
    TuiActor::new(config.clone(), tx.clone(), scope.clone()).run();

    // Spawn some agents
    let (spawn_agent_message, agent1_scope) = create_spawn_agent_message(
        &scope,
        AgentType::SubManager,
        "Filler Role",
        "Manage some workers",
    );
    let _ = tx.send(spawn_agent_message);

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
