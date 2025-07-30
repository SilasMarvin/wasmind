use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

use hive_actor_loader::LoadedActor;

use crate::{HiveResult, actors::MessageEnvelope, scope::Scope};

/// Shared context for the Hive system that enables agent spawning
pub struct HiveContext {
    /// Broadcast channel for all messages
    pub tx: broadcast::Sender<MessageEnvelope>,
    
    /// Registry of available actors (can be cloned to spawn)
    pub actor_registry: HashMap<String, LoadedActor>,
    
    /// Track which actors are expected in each scope
    /// Arc<Mutex<>> for concurrent access from spawn_agent calls
    pub scope_tracking: Arc<Mutex<HashMap<Scope, HashSet<String>>>>,
}

impl HiveContext {
    pub fn new(
        tx: broadcast::Sender<MessageEnvelope>,
        loaded_actors: Vec<LoadedActor>,
    ) -> Self {
        let actor_registry = loaded_actors
            .into_iter()
            .map(|actor| (actor.id.clone(), actor))
            .collect();
        
        Self {
            tx,
            actor_registry,
            scope_tracking: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    
    /// Spawn a new agent with the specified actors in a new scope
    pub async fn spawn_agent(&self, actor_ids: &[&str]) -> HiveResult<Scope> {
        let scope = Scope::new();
        self.spawn_agent_in_scope(actor_ids, scope).await
    }
    
    /// Spawn a new agent with the specified actors in a specific scope
    pub async fn spawn_agent_in_scope(&self, actor_ids: &[&str], scope: Scope) -> HiveResult<Scope> {
        
        // Track what actors we're spawning in this scope
        {
            let mut tracking = self.scope_tracking.lock().unwrap();
            tracking.insert(
                scope.clone(),
                actor_ids.iter().map(|s| s.to_string()).collect(),
            );
        }
        
        // Clone and run the actors with the new scope
        for actor_id in actor_ids {
            if let Some(loaded_actor) = self.actor_registry.get(*actor_id) {
                use crate::actors::ActorExecutor;
                let context = Arc::new(self.clone());
                loaded_actor.clone().run(scope.clone(), self.tx.clone(), context).await;
            } else {
                tracing::warn!("Actor '{}' not found in registry", actor_id);
            }
        }
        
        Ok(scope)
    }
}

impl Clone for HiveContext {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            actor_registry: self.actor_registry.clone(),
            scope_tracking: self.scope_tracking.clone(),
        }
    }
}