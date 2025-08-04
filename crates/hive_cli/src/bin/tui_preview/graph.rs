use std::{sync::Arc, time::Duration};

use hive::{coordinator::HiveCoordinator, hive::STARTING_SCOPE};
use hive_actor_loader::LoadedActor;
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

    // let config = hive_config::load_default_config().unwrap();
    // let tui_config = crate::config::TuiConfig::from_config(&config)?.parse()?;
    //
    // // Set up broadcast channel
    // let (tx, _rx) = broadcast::channel(1000);
    //
    // // Create actors
    // TuiActor::new(config.clone(), tx.clone(), scope.clone(), None).run();
    // // Send the LiteLLM Actor Ready message
    // // Need to actually let the dashboard mount first
    // tokio::time::sleep(Duration::from_secs(1)).await;
    // let _ = tx.send(hive::actors::ActorMessage {
    //     scope: MAIN_MANAGER_SCOPE.clone(),
    //     message: Message::ActorReady {
    //         actor_id: LiteLLMManager::ACTOR_ID.to_string(),
    //     },
    // });
    // let _ = tx.send(create_agent_status_update_message(
    //     &scope,
    //     hive::actors::AgentStatus::Wait {
    //         reason: hive::actors::WaitReason::WaitForSystem {
    //             tool_name: None,
    //             tool_call_id: "Filler".to_string(),
    //         },
    //     },
    // ));

    // Spawn some agents
    let (spawn_agent_message, agent1_scope) =
        create_spawn_agent_message("Sub Manager 1", Some(&STARTING_SCOPE.to_string()));
    coordinator.broadcast_common_message(spawn_agent_message, false)?;

    let (spawn_agent_message, _) = create_spawn_agent_message("Worker 1", Some(&agent1_scope));
    coordinator.broadcast_common_message(spawn_agent_message, false)?;

    // let (spawn_agent_message, _) =
    //     create_spawn_agent_message(&agent1_scope, AgentType::Worker, "Filler Role", "Work");
    // let _ = tx.send(spawn_agent_message);
    //
    // let (spawn_agent_message, agent2_scope) = create_spawn_agent_message(
    //     &agent1_scope,
    //     AgentType::SubManager,
    //     "Filler Role",
    //     "Manage some workers",
    // );
    // let _ = tx.send(spawn_agent_message);
    //
    // let (spawn_agent_message, _) =
    //     create_spawn_agent_message(&agent2_scope, AgentType::Worker, "Filler Role", "Work");
    // let _ = tx.send(spawn_agent_message);

    tokio::time::sleep(Duration::from_secs(10_000)).await;

    Ok(())
}
