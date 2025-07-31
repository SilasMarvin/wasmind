use crate::actors::ActorExecutor;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

use hive_actor_utils_common_messages::actors::AgentSpawned;

use crate::{HiveResult, SerializationSnafu, actors::MessageEnvelope, scope::Scope};
use snafu::ResultExt;

/// Shared context for the Hive system that enables agent spawning
#[derive(Clone)]
pub struct HiveContext {
    /// Broadcast channel for all messages
    pub tx: broadcast::Sender<MessageEnvelope>,

    /// Registry of available actors (can be cloned to spawn)
    pub actor_executors: Vec<Arc<dyn ActorExecutor + 'static>>,

    /// Track which actors are expected in each scope
    /// Arc<Mutex<>> for concurrent access from spawn_agent calls
    pub scope_tracking: Arc<Mutex<HashMap<Scope, HashSet<String>>>>,
}

impl HiveContext {
    pub fn new<T>(actors: Vec<T>) -> Self
    where
        T: ActorExecutor + 'static,
    {
        let (tx, _) = broadcast::channel(1024);

        let actor_executors = actors
            .into_iter()
            .map(|actor| Arc::new(actor) as Arc<dyn ActorExecutor>)
            .collect();

        Self {
            tx,
            actor_executors,
            scope_tracking: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Add an individual actor of any type that implements ActorExecutor
    pub fn add_actor<T>(&mut self, actor: T)
    where
        T: ActorExecutor + 'static,
    {
        self.actor_executors.push(Arc::new(actor));
    }

    /// Spawn a new agent with the specified actors in a new scope
    pub async fn spawn_agent(
        &self,
        actor_ids: &[&str],
        agent_name: String,
        parent_scope: Option<Scope>,
    ) -> HiveResult<Scope> {
        let scope = Scope::new();
        self.spawn_agent_in_scope(actor_ids, scope, agent_name, parent_scope)
            .await
    }

    /// Spawn a new agent with the specified actors in a specific scope
    pub async fn spawn_agent_in_scope(
        &self,
        actor_ids: &[&str],
        scope: Scope,
        agent_name: String,
        parent_scope: Option<Scope>,
    ) -> HiveResult<Scope> {
        // Track what actors we're spawning in this scope
        {
            let mut tracking = self.scope_tracking.lock().unwrap();
            tracking.insert(
                scope.clone(),
                actor_ids.iter().map(|s| s.to_string()).collect(),
            );
        }

        // Clone and run the actors with the new scope
        for actor in &self.actor_executors {
            if actor.auto_spawn() || actor_ids.contains(&actor.actor_id()) {
                let context = Arc::new(self.clone());
                actor
                    .clone()
                    .run(scope.clone(), self.tx.clone(), context)
                    .await;
            }
        }

        // Broadcast AgentSpawned message
        let agent_spawned = AgentSpawned {
            agent_id: scope.to_string(),
            name: agent_name,
            parent_agent: parent_scope.map(|s| s.to_string()),
            actors: actor_ids.iter().map(|s| s.to_string()).collect(),
        };

        self.broadcast_common_message(agent_spawned)?;

        Ok(scope)
    }

    /// Broadcast a common message to all actors
    pub fn broadcast_common_message<T>(&self, message: T) -> HiveResult<()>
    where
        T: hive_actor_utils_common_messages::Message,
    {
        let message_envelope = MessageEnvelope {
            from_actor_id: "hive__context".to_string(),
            from_scope: crate::hive::STARTING_SCOPE.to_string(),
            message_type: T::MESSAGE_TYPE.to_string(),
            payload: serde_json::to_vec(&message).context(SerializationSnafu {
                message: "Failed to serialize message for broadcast",
            })?,
        };
        self.broadcast(message_envelope)
    }

    /// Broadcasts a message to all actors
    pub fn broadcast(&self, message_envelope: MessageEnvelope) -> HiveResult<()> {
        self.tx
            .send(message_envelope)
            .map_err(|_| crate::Error::Broadcast)?;
        Ok(())
    }
}
