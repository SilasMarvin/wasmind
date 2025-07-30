use hive_actor_loader::LoadedActor;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::{HiveResult, context::HiveContext, coordinator::HiveCoordinator, scope::Scope};

pub const STARTING_SCOPE: Scope =
    Scope::from_uuid(uuid::uuid!("00000000-0000-0000-0000-000000000000"));

/// Start the HIVE multi-agent system
pub async fn start_hive(
    starting_actors: &[&str],
    loaded_actors: Vec<LoadedActor>,
) -> HiveResult<HiveCoordinator> {
    // Create broadcast channel
    let (tx, _) = broadcast::channel(1024);

    // Create shared context
    let context = Arc::new(HiveContext::new(tx, loaded_actors));

    // Create and run coordinator
    let coordinator = HiveCoordinator::new(context.clone());

    // Start initial actors in the starting scope
    context
        .spawn_agent_in_scope(starting_actors, STARTING_SCOPE)
        .await?;

    Ok(coordinator)
}
