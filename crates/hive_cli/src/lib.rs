use snafu::Snafu;

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
        source: hive_config::Error,
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

    #[snafu(whatever, display("{message}"))]
    Whatever {
        message: String,
        #[snafu(source(from(Box<dyn std::error::Error + Send + Sync>, Some)))]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

pub type TuiResult<T> = Result<T, Error>;
