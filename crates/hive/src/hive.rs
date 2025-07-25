use hive_actor_loader::LoadedActor;
use tokio::sync::broadcast;

use crate::{
    SResult,
    actors::{MessageEnvelope, actor_manager::Manager},
    scope::Scope,
};

pub const MAIN_MANAGER_SCOPE: Scope =
    Scope::from_uuid(uuid::uuid!("00000000-0000-0000-0000-000000000000"));

/// Start the HIVE multi-agent system with TUI
pub async fn start_hive(loaded_actors: Vec<LoadedActor>) -> SResult<()> {
    let (tx, _) = broadcast::channel::<MessageEnvelope>(1024);

    for actor in loaded_actors {
        let manager = Manager::new(
            actor.crate_name.to_string(),
            MAIN_MANAGER_SCOPE.clone(),
            &actor.wasm,
        )
        .await;
    }

    let mut rx = tx.subscribe();

    return Ok(());

    // // Create the TUI immediatly
    // TuiActor::new(
    //     config.clone(),
    //     tx.clone(),
    //     MAIN_MANAGER_SCOPE,
    //     initial_prompt.clone(),
    // )
    // .run();
    //
    // // #[cfg(feature = "gui")]
    // // Context::new(config.clone(), tx.clone(), ROOT_AGENT_SCOPE).run();
    // // #[cfg(feature = "audio")]
    // // Microphone::new(config.clone(), tx.clone(), ROOT_AGENT_SCOPE).run();
    //
    // // Create the Main Manager agent
    // let main_manager = Agent::new(
    //     tx.clone(),
    //     MAIN_MANAGER_ROLE.to_string(),
    //     None,
    //     config.clone(),
    //     Scope::new(), // parent_scope means nothing for the MainManager
    //     AgentType::MainManager,
    // )
    // .with_scope(MAIN_MANAGER_SCOPE)
    // .with_actors([
    //     Planner::ACTOR_ID,
    //     SpawnAgent::ACTOR_ID,
    //     SendMessage::ACTOR_ID,
    //     WaitTool::ACTOR_ID,
    //     LiteLLMManager::ACTOR_ID,
    // ]);
    //
    // // Start the Main Manager
    // main_manager.run();
    //
    // // Submit the initial user prompt if it exists
    // if let Some(prompt) = initial_prompt.take() {
    //     sleep(Duration::from_millis(250)).await;
    //     let _ = tx.send(ActorMessage {
    //         scope: MAIN_MANAGER_SCOPE.clone(),
    //         message: Message::UserContext(crate::actors::UserContext::UserTUIInput(prompt)),
    //     });
    // }
    //
    // // Listen for messages
    // loop {
    //     let msg = rx.recv().await;
    //     let msg = msg.expect("Error receiving in hive");
    //     let message_json = serde_json::to_string(&msg).unwrap_or_else(|_| format!("{:?}", msg));
    //     tracing::debug!(name = "hive_received_message", message = %message_json, message_type = std::any::type_name::<Message>());
    //
    //     match msg.message {
    //         Message::Exit if msg.scope == MAIN_MANAGER_SCOPE => {
    //             // Let everything clean up
    //             sleep(Duration::from_millis(500)).await;
    //             return Ok(());
    //         }
    //         _ => (),
    //     }
    // }
}
