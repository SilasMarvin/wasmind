use crate::actors::ActorExecutor;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use wasmind_actor_utils::STARTING_SCOPE;
use wasmind_actor_utils::common_messages::actors::AgentSpawned;
use wasmtime::{Config, Engine};

use crate::{SerializationSnafu, WasmindResult, actors::MessageEnvelope, scope::Scope};
use snafu::ResultExt;

/// Shared context for the Wasmind system that enables agent spawning
#[derive(Clone)]
pub struct WasmindContext {
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

    /// Global WASM engine for compilation and management of wasm modules
    pub engine: Engine,
}

impl WasmindContext {
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

        // Create the WASM engine with async support
        let mut config = Config::new();
        config.async_support(true);
        let engine = Engine::new(&config).unwrap();

        Self {
            tx,
            actor_executors,
            scope_tracking: Arc::new(Mutex::new(HashMap::new())),
            scope_parents: Arc::new(Mutex::new(HashMap::new())),
            engine,
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
    pub async fn spawn_agent<S: AsRef<str> + std::fmt::Debug>(
        &self,
        actor_ids: &[S],
        agent_name: String,
        parent_scope: Option<Scope>,
    ) -> WasmindResult<Scope> {
        let scope = crate::scope::new_scope();
        self.spawn_agent_in_scope(actor_ids, scope.clone(), agent_name, parent_scope)
            .await?;
        Ok(scope)
    }

    /// Spawn a new agent with the specified actors in a specific scope
    pub async fn spawn_agent_in_scope<S: AsRef<str> + std::fmt::Debug>(
        &self,
        actor_names: &[S],
        scope: Scope,
        agent_name: String,
        parent_scope: Option<Scope>,
    ) -> WasmindResult<()> {
        tracing::debug!(
            "Attempting to spawn agent in scope: {scope} with actors: {:?}",
            actor_names
        );

        let logical_actors_to_spawn: Vec<&str> = self
            .actor_executors
            .iter()
            .filter_map(|(logical_name, actor)| {
                if actor.auto_spawn()
                    || actor_names
                        .iter()
                        .any(|s| s.as_ref() == logical_name.as_str())
                {
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
                    "Attempted to spawn: `{actor}` twice in the same scope. Second request was ignored and `{actor}` will only be spawned once."
                );
            } else {
                set_of_logical_actors_to_spawn.insert(actor);
            }
        }

        for actor_name in actor_names {
            if !set_of_logical_actors_to_spawn.contains(&actor_name.as_ref()) {
                tracing::warn!(
                    "Could not spawn actor: {actor_name:?} in scope: {scope} - actor not found. Confirm it is correctly listed as a requirement."
                )
            }
        }

        // NOTE: It is important we store the relationship between the child and parent scope
        // before we spawn the children as the children may utilize this info in their new function
        {
            let mut parents = self.scope_parents.lock().unwrap();
            parents.insert(scope.clone(), parent_scope.clone());
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
                .run(
                    scope.clone(),
                    self.tx.clone(),
                    rx,
                    context,
                    self.engine.clone(),
                )
                .await;
        }

        {
            let mut tracking = self.scope_tracking.lock().unwrap();
            tracking.insert(scope.clone(), actors_spawned);
        }

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
    pub fn broadcast_common_message<T>(&self, message: T) -> WasmindResult<()>
    where
        T: wasmind_actor_utils::messages::Message,
    {
        let message_envelope = MessageEnvelope {
            id: crate::utils::generate_root_correlation_id(),
            from_actor_id: "wasmind__context".to_string(),
            from_scope: STARTING_SCOPE.to_string(),
            message_type: T::MESSAGE_TYPE.to_string(),
            payload: serde_json::to_vec(&message).context(SerializationSnafu {
                message: "Failed to serialize message for broadcast",
            })?,
        };
        self.broadcast(message_envelope)
    }

    /// Broadcasts a message to all actors
    pub fn broadcast(&self, message_envelope: MessageEnvelope) -> WasmindResult<()> {
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
