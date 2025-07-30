pub mod actors;
pub mod context;
pub mod coordinator;
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

    #[snafu(display("Failed to broadcast message"))]
    Broadcast,

    #[snafu(display("Channel closed"))]
    ChannelClosed,

    #[snafu(display("Invalid scope format: {scope}"))]
    InvalidScope {
        scope: String,
        #[snafu(source)]
        source: uuid::Error,
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
