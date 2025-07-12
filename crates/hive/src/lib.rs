pub mod actors;
pub mod cli;
pub mod config;
pub mod hive;
pub mod litellm_manager;
pub mod llm_client;
pub mod prompt_preview;
pub mod scope;
pub mod system_state;
pub mod template;
pub mod utils;

use config::{Config, ParsedConfig};
use snafu::ResultExt;
use snafu::{Location, Snafu};
use std::sync::OnceLock;
use tokio::runtime;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

pub static IS_HEADLESS: OnceLock<bool> = OnceLock::new();

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Config Error"))]
    Config {
        #[snafu(source)]
        source: config::ConfigError,
    },

    #[snafu(display("LiteLLM Error"))]
    LiteLLM {
        #[snafu(source)]
        source: litellm_manager::LiteLLMError,
    },

    #[cfg(feature = "gui")]
    #[snafu(display("Error copying clipboard"))]
    Clipboard {
        #[snafu(implicit)]
        location: Location,
        #[snafu(source)]
        source: arboard::Error,
    },

    #[cfg(feature = "gui")]
    #[snafu(display("Error copying clipboard"))]
    Xcap {
        #[snafu(implicit)]
        location: Location,
        #[snafu(source)]
        source: xcap::XCapError,
    },

    #[snafu(whatever, display("{message}"))]
    Whatever {
        message: String,
        #[snafu(source(from(Box<dyn std::error::Error + Send + Sync>, Some)))]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[snafu(display("Tool execution not found for call_id: {call_id}"))]
    ToolExecutionNotFound {
        #[snafu(implicit)]
        location: Location,
        call_id: String,
    },
}

pub type SResult<T> = Result<T, Error>;

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

    tracing_subscriber::registry()
        .with(EnvFilter::from_env("HIVE_LOG"))
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

pub async fn run_main_program() -> SResult<()> {
    IS_HEADLESS.set(false).unwrap();

    let config = Config::new(false).context(ConfigSnafu)?;
    let parsed_config: ParsedConfig = config.try_into().context(ConfigSnafu)?;

    // Start the HIVE multi-agent system
    hive::start_hive(parsed_config).await
}

pub async fn run_headless_program(
    prompt: String,
    auto_approve_commands_override: bool,
) -> SResult<()> {
    IS_HEADLESS.set(true).unwrap();

    let config = Config::new(true).context(ConfigSnafu)?;
    let mut parsed_config: ParsedConfig = config.try_into().context(ConfigSnafu)?;

    // Override config setting if CLI flag is provided
    if auto_approve_commands_override {
        parsed_config.auto_approve_commands = true;
    }

    // Start the HIVE system without TUI
    hive::start_headless_hive(parsed_config, prompt, None).await
}
