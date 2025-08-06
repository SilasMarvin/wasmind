use std::{sync::Arc, time::Duration};

use hive::coordinator::HiveCoordinator;
use hive_actor_loader::LoadedActor;
use hive_actor_utils::STARTING_SCOPE;
use hive_cli::{TuiResult, tui};
use tracing::info;

use crate::utils::create_spawn_agent_message;

// use crate::utils::{create_agent_status_update_message, create_spawn_agent_message};

pub async fn run() -> TuiResult<()> {
    info!("Starting command execution scenario");

    let tui_config = hive_cli::config::TuiConfig::default().parse()?;

    let context = Arc::new(hive::context::HiveContext::new::<LoadedActor>(vec![]));
    let mut coordinator: HiveCoordinator = HiveCoordinator::new(context.clone());

    let tui = tui::Tui::new(
        tui_config,
        coordinator.get_sender(),
        Some("Filler user prompt...".to_string()),
        context.clone(),
    );

    coordinator
        .start_hive(&vec![], "Root Agent".to_string())
        .await?;

    tui.run();

    // Spawn some agents
    for i in 0..300 {
        let (spawn_agent_message, agent1_scope) = create_spawn_agent_message(
            &format!("Sub Manager {i}"),
            Some(&STARTING_SCOPE.to_string()),
        );
        coordinator.broadcast_common_message(spawn_agent_message, false)?;
    }

    // let (spawn_agent_message, _) = create_spawn_agent_message("Worker 1", Some(&agent1_scope));
    // coordinator.broadcast_common_message(spawn_agent_message, false)?;

    tokio::time::sleep(Duration::from_secs(15)).await;

    Ok(())
}
