use hive_actor_utils_common_messages::{Message, actors};
use tokio::sync::broadcast;

use crate::{
    HiveResult,
    actors::{ActorExecutor, MessageEnvelope},
    scope::Scope,
};

pub const STARTING_SCOPE: Scope =
    Scope::from_uuid(uuid::uuid!("00000000-0000-0000-0000-000000000000"));

/// Start the HIVE multi-agent system and return the broadcast sender
pub async fn start_hive<T: ActorExecutor + Clone>(
    starting_actors: &[&str],
    actors: Vec<T>,
) -> HiveResult<broadcast::Sender<MessageEnvelope>> {
    let (tx, _) = broadcast::channel::<MessageEnvelope>(1024);

    // Start the starting actors
    for actor in actors.clone().into_iter().filter(|actor| {
        starting_actors
            .iter()
            .find(|sa| actor.actor_id() == **sa)
            .is_some()
    }) {
        actor.run(STARTING_SCOPE.clone(), tx.clone()).await;
    }

    Ok(tx)
}

/// Wait for the HIVE system to exit
pub async fn wait_for_exit(tx: broadcast::Sender<MessageEnvelope>) -> HiveResult<()> {
    let mut rx = tx.subscribe();

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
