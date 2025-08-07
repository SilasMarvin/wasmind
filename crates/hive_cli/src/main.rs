use clap::Parser;
use hive::coordinator::HiveCoordinator;
use snafu::whatever;
use std::sync::Arc;

use hive_actor_utils_common_messages::assistant::AddMessage;
use hive_actor_utils_common_messages::litellm::BaseUrlUpdate;
use hive_llm_types::types::ChatMessage;

use hive_cli::{Error, TuiResult, config, init_logger_with_path, litellm_manager, tui};

mod cli;

#[tokio::main]
async fn main() -> TuiResult<()> {
    let cli = cli::Cli::parse();

    // Initialize logger with specified path or config default
    let log_file = if let Some(path) = &cli.log_file {
        path.clone()
    } else {
        hive_config::get_log_file_path()?
    };
    init_logger_with_path(log_file);

    // Handle info, clean, and status commands before loading configuration
    match &cli.command {
        Some(cli::Commands::Info) => {
            if let Err(e) = hive_cli::commands::info::show_info() {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
            return Ok(());
        }
        Some(cli::Commands::Clean) => {
            if let Err(e) = hive_cli::commands::clean::clean_cache() {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
            return Ok(());
        }
        Some(cli::Commands::Check) => {
            if let Err(e) = hive_cli::commands::check::show_status(cli.config.clone()).await {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
            return Ok(());
        }
        None => {
            // Continue with terminal interface startup
        }
    }

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
    println!("Starting LiteLLM...");
    tracing::info!("Starting LiteLLM manager...");
    let mut litellm_manager = litellm_manager::LiteLLMManager::new(litellm_config.clone());

    // Set up the full startup with signal handling to ensure cleanup during any phase
    let result = async {
        litellm_manager.start().await?;
        println!("✓ LiteLLM started at {}", litellm_config.get_base_url());
        tracing::info!(
            "LiteLLM manager started successfully at {}",
            litellm_config.get_base_url()
        );
        tracing::info!("Available models: {:?}", litellm_config.list_model_names());

        // Error if no starting actors are configured
        if config.starting_actors.is_empty() {
            if !config.actors.iter().any(|actor| actor.auto_spawn) {
                whatever!(
                    "No starting actors and no auto spawning actors configured - at least one starting actor or auto spawning actor is required"
                );
            }
        }

        // Load terminal interface configuration
        let tui_config = crate::config::TuiConfig::from_config(&config)?.parse()?;

        // Load the actors
        let loaded_actors = hive::load_actors(config.actors, config.actor_overrides).await?;

        // Create the context
        let context = Arc::new(hive::context::HiveContext::new(loaded_actors));
        let mut coordinator: HiveCoordinator = HiveCoordinator::new(context.clone());

        // Create the terminal interface making it subscribe to messages before starting hive
        let tui = tui::Tui::new(
            tui_config,
            coordinator.get_sender(),
            cli.prompt.clone(),
            context.clone(),
        );

        // Start the hive
        let starting_actors: Vec<&str> =
            config.starting_actors.iter().map(|s| s.as_str()).collect();
        coordinator
            .start_hive(&starting_actors, "Root Agent".to_string())
            .await?;

        // Start the terminal interface
        tui.run();

        // Broadcast the LiteLLM base URL to all actors
        let base_url_update = BaseUrlUpdate {
            base_url: litellm_config.get_base_url(),
            models_available: litellm_config
                .list_model_names()
                .into_iter()
                .cloned()
                .collect(),
        };
        coordinator.broadcast_common_message(base_url_update, true)?;

        // Broadcast initial user prompt if provided
        if let Some(prompt) = &cli.prompt {
            let add_message = AddMessage {
                agent: hive_actor_utils::STARTING_SCOPE.to_string(),
                message: ChatMessage::user(prompt),
            };
            coordinator.broadcast_common_message(add_message, false)?;
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
