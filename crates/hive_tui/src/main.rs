use clap::Parser;
use snafu::{Snafu, whatever};

use hive_actor_utils_common_messages::assistant::AddMessage;
use hive_actor_utils_common_messages::litellm::BaseUrlUpdate;
use hive_llm_types::types::ChatMessage;

mod actors;
mod cli;
mod config;
mod litellm_manager;

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

#[tokio::main]
async fn main() -> TuiResult<()> {
    hive::init_test_logger();

    let cli = cli::Cli::parse();

    // Load configuration
    let config = if let Some(config_path) = cli.config {
        hive_config::load_from_path(config_path)?
    } else {
        hive_config::load_default_config()?
    };

    // Parse LiteLLM configuration
    let litellm_config = config::LiteLLMConfig::from_config(&config)?;
    tracing::info!(
        "LiteLLM configuration loaded: base_url = {}",
        litellm_config.get_base_url()
    );
    tracing::info!("Available models: {:?}", litellm_config.list_model_names());

    // Require models
    if litellm_config.models.is_empty() {
        whatever!("No LiteLLM models configured - LiteLLM section is required");
    }

    // Create LiteLLM manager first so it's available for the entire scope
    tracing::info!("Starting LiteLLM manager...");
    let mut litellm_manager = litellm_manager::LiteLLMManager::new(litellm_config.clone());

    // Set up the full startup with signal handling to ensure cleanup during any phase
    let result = async {
        litellm_manager.start().await?;
        tracing::info!(
            "LiteLLM manager started successfully at {}",
            litellm_config.get_base_url()
        );
        tracing::info!("Available models: {:?}", litellm_config.list_model_names());

        // Error if no starting actors are configured
        if config.starting_actors.is_empty() {
            whatever!("No starting actors configured - at least one starting actor is required");
        }

        // Load the actors
        let loaded_actors = hive::load_actors(config.actors).await?;

        // Start the hive
        let starting_actors: Vec<&str> =
            config.starting_actors.iter().map(|s| s.as_str()).collect();
        let coordinator = hive::hive::start_hive(&starting_actors, loaded_actors).await?;

        // Broadcast the LiteLLM base URL to all actors
        let base_url_update = BaseUrlUpdate {
            base_url: litellm_config.get_base_url(),
            models_available: litellm_config
                .list_model_names()
                .into_iter()
                .cloned()
                .collect(),
        };
        coordinator.broadcast_common_message(base_url_update)?;

        // Broadcast initial user prompt if provided
        if let Some(prompt) = &cli.prompt {
            let add_message = AddMessage {
                agent: hive::hive::STARTING_SCOPE.to_string(),
                message: ChatMessage::user(prompt),
            };
            coordinator.broadcast_common_message(add_message)?;
        }

        // Wait for the hive to exit
        Ok(coordinator.run().await?)
    };

    // Explicitly handling the ctrl+c ensures drop is being called correctly on the litellm_manager
    // Without this I had some issues where it was not being called and the containers were left alive

    // Run with signal handling for the entire operation
    let shutdown_result: Result<(), Error> = tokio::select! {
        result = result => result,
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Received Ctrl+C, shutting down gracefully...");
            Ok(())
        }
    };

    shutdown_result
}

// use hive::{init_test_logger, run_headless_program, run_main_program, SResult};
//
// #[tokio::main]
// async fn main() -> SResult<()> {
//     use clap::Parser;
//
//     init_test_logger();
//
//     // Parse command line arguments
//     let cli = hive::cli::Cli::parse();
//
//     match cli.command {
//         None => {
//             // No subcommand provided, use top-level prompt if any
//             run_main_program(cli.prompt).await?;
//         }
//         Some(hive::cli::Commands::Headless {
//             prompt,
//             auto_approve_commands,
//         }) => {
//             run_headless_program(prompt, auto_approve_commands).await?;
//         }
//         Some(hive::cli::Commands::PromptPreview {
//             all,
//             empty,
//             files,
//             plan,
//             agents,
//             complete,
//             full,
//             agent_types,
//             config,
//         }) => {
//             if let Err(e) = hive::prompt_preview::execute_demo(
//                 all,
//                 empty,
//                 files,
//                 plan,
//                 agents,
//                 complete,
//                 full,
//                 agent_types,
//                 config,
//             ) {
//                 eprintln!("Prompt preview error: {}", e);
//                 std::process::exit(1);
//             }
//         }
//     }
//
//     Ok(())
// }
