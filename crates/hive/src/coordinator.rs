use hive_actor_utils::STARTING_SCOPE;
use hive_actor_utils::common_messages::actors;
use hive_actor_utils::messages::Message;
use snafu::ResultExt;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::Level;

use crate::SerializationSnafu;
use crate::{HiveResult, actors::MessageEnvelope, context::HiveContext, scope::Scope};

/// Coordinator that monitors actor lifecycle and system exit
pub struct HiveCoordinator {
    /// Receiver for monitoring messages
    rx: broadcast::Receiver<MessageEnvelope>,

    /// Reference to context for scope tracking info
    context: Arc<HiveContext>,

    /// Track which actors have sent ActorReady per scope
    ready_actors: HashMap<Scope, HashSet<String>>,

    /// Replayable messages which are broadcasted everytime a new agent is spawned
    replayable: Vec<MessageEnvelope>,
}

impl HiveCoordinator {
    pub fn new(context: Arc<HiveContext>) -> Self {
        let rx = context.tx.subscribe();
        Self {
            rx,
            context,
            ready_actors: HashMap::new(),
            replayable: vec![],
        }
    }

    pub async fn start_hive(
        &self,
        starting_actors: &[&str],
        root_agent_name: String,
    ) -> HiveResult<Scope> {
        self.context
            .spawn_agent_in_scope(
                starting_actors,
                STARTING_SCOPE.to_string(),
                root_agent_name,
                None,
            )
            .await?;
        Ok(STARTING_SCOPE.to_string())
    }

    /// Run the coordinator until system exit
    pub async fn run(mut self) -> HiveResult<()> {
        loop {
            match self.rx.recv().await {
                Ok(msg) => {
                    let span = tracing::span!(
                        Level::ERROR,
                        "hive_coordinator_run",
                        correlation_id = msg.id
                    );
                    let _enter = span.enter();

                    let message_json =
                        if let Ok(json_string) = String::from_utf8(msg.payload.clone()) {
                            json_string
                        } else {
                            "na".to_string()
                        };
                    tracing::debug!(
                        name = "hive_coordinator_received_message",
                        actor_id = msg.from_actor_id,
                        message_type = msg.message_type,
                        message = %message_json
                    );

                    match msg.message_type.as_str() {
                        actors::ActorReady::MESSAGE_TYPE => {
                            self.handle_actor_ready(msg)?;
                        }
                        actors::Exit::MESSAGE_TYPE => {
                            // Check if it's the STARTING_SCOPE exiting
                            if msg.from_scope == STARTING_SCOPE {
                                tracing::info!("Starting scope exited, shutting down system");
                                return Ok(());
                            }
                            // Otherwise it's just a scoped shutdown
                            tracing::info!("Scope {} is shutting down", msg.from_scope);
                        }
                        _ => {}
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::error!("Coordinator receiver lagged by {} messages", n);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::error!("Channel closed");
                    return Err(crate::Error::ChannelClosed);
                }
            }
        }
    }

    fn handle_actor_ready(&mut self, msg: MessageEnvelope) -> HiveResult<()> {
        // Get scope from message
        let scope = msg.from_scope.clone();

        // Track this actor as ready
        self.ready_actors
            .entry(scope.clone())
            .or_insert_with(HashSet::new)
            .insert(msg.from_actor_id.clone());

        // Check if all actors for this scope are ready
        if let Some(expected_actors) = self.context.scope_tracking.lock().unwrap().get(&scope) {
            let ready_count = self.ready_actors.get(&scope).map(|s| s.len()).unwrap_or(0);
            let expected_count = expected_actors.len();

            tracing::debug!(
                "Scope {} has {}/{} actors ready",
                scope,
                ready_count,
                expected_count
            );

            if ready_count == expected_count {
                // All actors for this scope are ready
                tracing::info!("All actors ready for scope {}", scope);

                // Broadcast AllActorsReady for this scope
                let all_ready_msg = MessageEnvelope {
                    id: crate::utils::generate_root_correlation_id(),
                    message_type: actors::AllActorsReady::MESSAGE_TYPE.to_string(),
                    from_actor_id: "hive_coordinator".to_string(),
                    from_scope: scope.to_string(),
                    payload: serde_json::to_string(&actors::AllActorsReady)
                        .unwrap()
                        .into_bytes(),
                };

                if let Err(e) = self.context.tx.send(all_ready_msg) {
                    tracing::error!("Failed to broadcast AllActorsReady: {}", e);
                }

                // Broadcast replyable messages
                for message in &self.replayable {
                    if let Err(e) = self.context.tx.send(message.clone()) {
                        tracing::error!("Failed to broadcast Replayable Message: {}", e);
                    }
                }
            }
        }

        Ok(())
    }

    pub fn broadcast_common_message<T>(&mut self, message: T, replayable: bool) -> HiveResult<()>
    where
        T: hive_actor_utils::common_messages::Message + Clone,
    {
        self.broadcast_common_message_in_scope(message, &STARTING_SCOPE.to_string(), replayable)
    }

    pub fn broadcast_common_message_in_scope<T>(
        &mut self,
        message: T,
        scope: &Scope,
        replayable: bool,
    ) -> HiveResult<()>
    where
        T: hive_actor_utils::common_messages::Message + Clone,
    {
        let message_envelope = MessageEnvelope {
            id: crate::utils::generate_root_correlation_id(),
            from_actor_id: "hive__coordinator".to_string(),
            from_scope: scope.to_owned(),
            message_type: T::MESSAGE_TYPE.to_string(),
            payload: serde_json::to_vec(&message).context(SerializationSnafu {
                message: "Failed to serialize message for broadcast",
            })?,
        };
        if replayable {
            self.replayable.push(message_envelope.clone());
        }
        self.context.broadcast(message_envelope)
    }

    /// Get the broadcast sender for sending messages to the system
    pub fn get_sender(&self) -> broadcast::Sender<MessageEnvelope> {
        self.context.tx.clone()
    }
}
