use hive_actor_utils_common_messages::{CommonMessage, actors};
use tokio::sync::broadcast;

use crate::{
    HiveResult,
    actors::{ActorExecutor, MessageEnvelope},
    scope::Scope,
};

pub const STARTING_SCOPE: Scope =
    Scope::from_uuid(uuid::uuid!("00000000-0000-0000-0000-000000000000"));

/// Start the HIVE multi-agent system
pub async fn start_hive<T: ActorExecutor + Clone>(
    starting_actors: &[&str],
    actors: Vec<T>,
) -> HiveResult<()> {
    let (tx, _) = broadcast::channel::<MessageEnvelope>(1024);
    let mut rx = tx.subscribe();

    // Start the starting actors
    for actor in actors.clone().into_iter().filter(|actor| {
        starting_actors
            .iter()
            .find(|sa| actor.actor_id() == **sa)
            .is_some()
    }) {
        actor.run(STARTING_SCOPE.clone(), tx.clone()).await;
    }

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

    // Listen for messages
    loop {
        let msg = rx.recv().await;
        let msg = msg.expect("Error receiving in hive");
        let message_json = if let Ok(json_string) = String::from_utf8(msg.payload) {
            json_string
        } else {
            "na".to_string()
        };
        tracing::debug!(name = "hive_received_message", actor_id = msg.from_actor_id, message_type = msg.message_type, message = %message_json);

        if msg.message_type == actors::Exit::MESSAGE_TYPE {
            return Ok(());
        }
    }
}
