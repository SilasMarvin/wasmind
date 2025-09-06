use snafu::{ResultExt, Snafu};
use tracing_rolling_file::*;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

pub mod commands;
pub mod config;
pub mod litellm_manager;
pub mod tui;
pub mod utils;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(transparent)]
    Wasmind {
        #[snafu(source)]
        source: wasmind::Error,
    },

    #[snafu(transparent)]
    Config {
        #[snafu(source(from(wasmind::wasmind_config::Error, Box::new)))]
        source: Box<wasmind::wasmind_config::Error>,
    },

    #[snafu(transparent)]
    ActorLoader {
        #[snafu(source(from(wasmind::wasmind_actor_loader::Error, Box::new)))]
        source: Box<wasmind::wasmind_actor_loader::Error>,
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

    #[snafu(display("IO error: {}", source))]
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

pub fn init_logger_with_path<P: AsRef<std::path::Path>>(log_path: P) -> TuiResult<()> {
    init_logger_with_rotation(log_path, 100, 5)
}

pub fn init_logger_with_rotation<P: AsRef<std::path::Path>>(
    log_path: P,
    max_file_size_mb: usize,
    max_file_count: usize,
) -> TuiResult<()> {
    // Create parent directory if it doesn't exist
    if let Some(parent) = log_path.as_ref().parent() {
        std::fs::create_dir_all(parent).context(IoSnafu)?;
    }

    let log_path_str = log_path.as_ref().to_string_lossy().to_string();

    // Create rolling file appender with size limits
    let rolling_condition =
        RollingConditionBase::new().max_size((max_file_size_mb * 1024 * 1024) as u64); // Convert MB to bytes

    let file_appender =
        RollingFileAppenderBase::new(&log_path_str, rolling_condition, max_file_count).map_err(
            |e| Error::Whatever {
                message: format!("Failed to create rolling file appender: {e:?}"),
                source: Some(Box::new(e)),
            },
        )?;

    let (non_blocking, _guard) = file_appender.get_non_blocking_appender();

    // Store the guard to prevent it from being dropped
    // This is a known issue with tracing-rolling-file - we need to keep the guard alive
    // Maybe we should return the guard instead?
    std::mem::forget(_guard);

    // Create filter with info as default, excluding cranelift debug logs
    let env_filter = EnvFilter::builder()
        .with_default_directive(tracing::level_filters::LevelFilter::INFO.into())
        .with_env_var("WASMIND_LOG")
        .from_env_lossy()
        .add_directive("cranelift_codegen=info".parse().unwrap())
        .add_directive("wasmtime_cranelift=info".parse().unwrap())
        .add_directive("wasmtime=info".parse().unwrap());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false)
                .with_target(true)
                .with_level(true)
                .with_line_number(true)
                .with_timer(tracing_subscriber::fmt::time::time())
                .compact(),
        )
        .init();

    Ok(())
}
