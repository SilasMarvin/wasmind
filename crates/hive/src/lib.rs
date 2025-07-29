pub mod actors;
pub mod hive;
pub mod scope;

use hive_actor_loader::{ActorLoader, LoadedActor};

use snafu::Snafu;
use std::path::PathBuf;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

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

    #[snafu(display("Broadcast error: {message}"))]
    Broadcast {
        message: String,
    },
}

pub type HiveResult<T> = Result<T, Error>;

// Library functions that main.rs can use
pub fn init_test_logger() {
    init_logger_with_path("log.txt");
}

pub fn init_logger_with_path<P: AsRef<std::path::Path>>(log_path: P) {
    // Create parent directory if it doesn't exist
    if let Some(parent) = log_path.as_ref().parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(log_path)
        .expect("Unable to open log file");

    // Create filter that excludes cranelift debug logs in debug builds
    let env_filter = EnvFilter::from_env("HIVE_LOG")
        .add_directive("cranelift_codegen=info".parse().unwrap())
        .add_directive("wasmtime_cranelift=info".parse().unwrap())
        .add_directive("wasmtime=info".parse().unwrap());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            fmt::layer()
                .with_writer(file)
                .with_ansi(false)
                .with_target(true)
                .with_level(true)
                .with_line_number(true)
                .with_timer(tracing_subscriber::fmt::time::time())
                .compact(),
        )
        .init();
}

pub async fn load_actors(actors: Vec<hive_config::Actor>) -> HiveResult<Vec<LoadedActor>> {
    let temp_cache = PathBuf::from("/tmp/hive_cache");
    let actor_loader = ActorLoader::new(Some(temp_cache))?;
    Ok(actor_loader.load_actors(actors).await?)
}

/// Broadcast a common message to all actors in the root scope
///
/// This utility function simplifies broadcasting messages that implement the Message trait
/// from hive_actor_utils_common_messages.
///
/// # Arguments
/// * `from_actor_id` - The ID of the actor sending the message
/// * `message` - Any message type that implements the Message trait
/// * `tx` - The message sender from the hive system
///
/// # Example
/// ```rust
/// use hive_actor_utils_common_messages::litellm::BaseUrlUpdate;
///
/// let base_url_update = BaseUrlUpdate {
///     base_url: "http://localhost:4000".to_string(),
///     models_available: vec!["gpt-4".to_string()],
/// };
///
/// broadcast_common_message("litellm_manager", base_url_update, &tx)
///     .expect("Failed to broadcast base URL update");
/// ```
pub fn broadcast_common_message<T>(
    from_actor_id: impl Into<String>,
    message: T,
    tx: &tokio::sync::broadcast::Sender<actors::MessageEnvelope>,
) -> HiveResult<()>
where
    T: hive_actor_utils_common_messages::Message,
{
    use snafu::ResultExt;

    let message_envelope = actors::MessageEnvelope {
        from_actor_id: from_actor_id.into(),
        from_scope: hive::STARTING_SCOPE.to_string(),
        message_type: T::MESSAGE_TYPE.to_string(),
        payload: serde_json::to_vec(&message).context(SerializationSnafu {
            message: "Failed to serialize message for broadcast",
        })?,
    };

    tx.send(message_envelope)
        .map_err(|_| Error::Broadcast {
            message: "Failed to send message - no receivers".to_string(),
        })?;

    Ok(())
}
