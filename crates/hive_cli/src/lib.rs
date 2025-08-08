use snafu::Snafu;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

pub mod commands;
pub mod config;
pub mod litellm_manager;
pub mod tui;
pub mod utils;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(transparent)]
    Hive {
        #[snafu(source)]
        source: hive::Error,
    },

    #[snafu(transparent)]
    Config {
        #[snafu(source)]
        source: hive::hive_config::Error,
    },

    #[snafu(transparent)]
    ActorLoader {
        #[snafu(source)]
        source: hive::hive_actor_loader::Error,
    },

    #[snafu(transparent)]
    LiteLLMConfig {
        #[snafu(source)]
        source: config::ConfigError,
    },

    #[snafu(transparent)]
    LiteLLM {
        #[snafu(source)]
        source: litellm_manager::LiteLLMError,
    },

    #[snafu(transparent)]
    Io {
        #[snafu(source)]
        source: std::io::Error,
    },

    #[snafu(whatever, display("{message}"))]
    Whatever {
        message: String,
        #[snafu(source(from(Box<dyn std::error::Error + Send + Sync>, Some)))]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

pub type TuiResult<T> = Result<T, Error>;

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
