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

    /// Registry of available actors mapped by logical name
    pub actor_executors: HashMap<String, Arc<dyn ActorExecutor + 'static>>,

    /// Track which actors are expected in each scope
    /// Arc<Mutex<>> for concurrent access from spawn_agent calls
    pub scope_tracking: Arc<Mutex<HashMap<Scope, HashSet<String>>>>,

    /// Track parent-child relationships between scopes
    /// Arc<Mutex<>> for concurrent access from spawn_agent calls
    pub scope_parents: Arc<Mutex<HashMap<Scope, Option<Scope>>>>,
}

impl HiveContext {
    pub fn new<T>(actors: Vec<T>) -> Self
    where
        T: ActorExecutor + 'static,
    {
        let (tx, _) = broadcast::channel(1024);

        let mut actor_executors = HashMap::new();
        for actor in actors {
            let logical_name = actor.logical_name().to_string();
            actor_executors.insert(logical_name, Arc::new(actor) as Arc<dyn ActorExecutor>);
        }

        Self {
            tx,
            actor_executors,
            scope_tracking: Arc::new(Mutex::new(HashMap::new())),
            scope_parents: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Add an individual actor of any type that implements ActorExecutor
    pub fn add_actor<T>(&mut self, actor: T)
    where
        T: ActorExecutor + 'static,
    {
        let logical_name = actor.logical_name().to_string();
        self.actor_executors.insert(logical_name, Arc::new(actor));
    }

    /// Spawn a new agent with the specified actors in a new scope
    pub async fn spawn_agent(
        &self,
        actor_ids: &[&str],
        agent_name: String,
        parent_scope: Option<Scope>,
    ) -> HiveResult<Scope> {
        let scope = crate::scope::new_scope();
        self.spawn_agent_in_scope(actor_ids, scope.clone(), agent_name, parent_scope)
            .await?;
        Ok(scope)
    }

    /// Spawn a new agent with the specified actors in a specific scope
    pub async fn spawn_agent_in_scope(
        &self,
        actor_names: &[&str],
        scope: Scope,
        agent_name: String,
        parent_scope: Option<Scope>,
    ) -> HiveResult<()> {
        let logical_actors_to_spawn: Vec<&str> = self
            .actor_executors
            .iter()
            .filter_map(|(logical_name, actor)| {
                if actor.auto_spawn() || actor_names.contains(&logical_name.as_str()) {
                    let mut actors_to_spawn = actor.required_spawn_with();
                    actors_to_spawn.push(logical_name.as_str());
                    Some(actors_to_spawn)
                } else {
                    None
                }
            })
            .flatten()
            .collect();

        let mut set_of_logical_actors_to_spawn = HashSet::new();

        for actor in logical_actors_to_spawn {
            if set_of_logical_actors_to_spawn.contains(&actor) {
                tracing::warn!(
                    "Attempted to spawn: `{actor}` twice in the same scope. Second request was ignored and `{actor}` was only spawned once."
                );
            } else {
                set_of_logical_actors_to_spawn.insert(actor);
            }
        }

        let rxs: Vec<_> = (0..set_of_logical_actors_to_spawn.len())
            .map(|_| self.tx.subscribe())
            .collect();

        let mut actors_spawned = HashSet::new();
        for (actor, rx) in set_of_logical_actors_to_spawn.iter().zip(rxs) {
            let context = Arc::new(self.clone());
            let actor = self
                .actor_executors
                .get(*actor)
                .ok_or(crate::Error::NonExistentActor {
                    actor: actor.to_string(),
                })?;
            actors_spawned.insert(actor.actor_id().to_string());
            actor
                .clone()
                .run(scope.clone(), self.tx.clone(), rx, context)
                .await;
        }

        {
            let mut tracking = self.scope_tracking.lock().unwrap();
            tracking.insert(scope.clone(), actors_spawned);
        }

        // Store parent relationship
        {
            let mut parents = self.scope_parents.lock().unwrap();
            parents.insert(scope.clone(), parent_scope.clone());
        }

        // Broadcast AgentSpawned message
        let agent_spawned = AgentSpawned {
            agent_id: scope.to_string(),
            name: agent_name,
            parent_agent: parent_scope.map(|s| s.to_string()),
            actors: set_of_logical_actors_to_spawn
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
        };

        self.broadcast_common_message(agent_spawned)?;

        Ok(())
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

    /// Get the parent scope for a given scope
    /// Returns None if the scope has no parent (root scope)
    pub fn get_parent_scope(&self, scope: Scope) -> Option<Scope> {
        let parents = self.scope_parents.lock().unwrap();
        parents.get(&scope).cloned().flatten()
    }
}
