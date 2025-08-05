pub mod actors;
pub mod context;
pub mod coordinator;
pub mod scope;
pub mod utils;

use hive_actor_loader::{ActorLoader, LoadedActor};

use snafu::Snafu;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(transparent)]
    Config {
        #[snafu(source)]
        source: hive_config::Error,
    },

    #[snafu(transparent)]
    ActorLoader {
        #[snafu(source)]
        source: hive_actor_loader::Error,
    },

    #[snafu(display("Serialization error: {message}"))]
    Serialization {
        message: String,
        #[snafu(source)]
        source: serde_json::Error,
    },

    #[snafu(display("Failed to broadcast message"))]
    Broadcast,

    #[snafu(display("Channel closed"))]
    ChannelClosed,

    #[snafu(display("Attempt to spawn non-existent actor: {actor}"))]
    NonExistentActor { actor: String },
}

pub type HiveResult<T> = Result<T, Error>;

pub async fn load_actors(
    actors: Vec<hive_config::Actor>,
    actor_overrides: Vec<hive_config::ActorOverride>,
) -> HiveResult<Vec<LoadedActor>> {
    let actor_loader = ActorLoader::new(None)?;
    Ok(actor_loader.load_actors(actors, actor_overrides).await?)
}
