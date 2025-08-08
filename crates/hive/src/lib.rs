pub mod actors;
pub mod context;
pub mod coordinator;
pub mod scope;
pub mod utils;

// Re-export the config and loader crates for convenience
pub use hive_actor_loader;
pub use hive_config;

use snafu::Snafu;

#[derive(Debug, Snafu)]
pub enum Error {
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
