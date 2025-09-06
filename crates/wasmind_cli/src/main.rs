use clap::Parser;
use snafu::whatever;
use std::sync::Arc;
use wasmind::coordinator::WasmindCoordinator;

use wasmind_actor_utils::common_messages::assistant::AddMessage;
use wasmind_actor_utils::common_messages::litellm::BaseUrlUpdate;
use wasmind_actor_utils::llm_client_types::ChatMessage;

use wasmind_cli::{Error, TuiResult, config, init_logger_with_path, litellm_manager, tui};

mod cli;

#[tokio::main]
#[snafu::report]
async fn main() -> TuiResult<()> {
    let cli = cli::Cli::parse();

    // Initialize logger with specified path or config default
    let log_file = if let Some(path) = &cli.log_file {
        path.clone()
    } else {
        wasmind::wasmind_config::get_log_file_path()?
    };
    init_logger_with_path(log_file)?;

    // Handle info, clean, and status commands before loading configuration
    match &cli.command {
        Some(cli::Commands::Info) => {
            if let Err(e) = wasmind_cli::commands::info::show_info() {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        Some(cli::Commands::Clean) => {
            if let Err(e) = wasmind_cli::commands::clean::clean_cache() {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        Some(cli::Commands::Check) => {
            if let Err(e) = wasmind_cli::commands::check::show_status(cli.config.clone()).await {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        None => {
            // Continue with terminal interface startup
        }
    }

    println!("Starting Wasmind");
    println!("It may take a few moments to load actors the first time\n");

    // Load configuration
    let config = if let Some(config_path) = cli.config {
        wasmind::wasmind_config::load_from_path(config_path)?
    } else {
        wasmind::wasmind_config::load_default_config()?
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

    // Error if no starting actors are configured
    if config.starting_actors.is_empty() && !config.actors.iter().any(|actor| actor.auto_spawn) {
        whatever!(
            "No starting actors and no auto spawning actors configured - at least one starting actor or auto spawning actor is required"
        );
    }

    // Load terminal interface configuration
    let tui_config = crate::config::TuiConfig::from_config(&config)?.parse()?;

    // Load the actors
    let cache_dir = wasmind::wasmind_config::get_actors_cache_dir()?;
    let actor_loader = wasmind::wasmind_actor_loader::ActorLoader::new(cache_dir)?;
    let loaded_actors = actor_loader
        .load_actors(config.actors, config.actor_overrides)
        .await?;

    // Create LiteLLM manager first so it's available for the entire scope
    println!("\nStarting LiteLLM...");
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

        // Create the context
        let context = Arc::new(wasmind::context::WasmindContext::new(loaded_actors));
        let mut coordinator: WasmindCoordinator = WasmindCoordinator::new(context.clone());

        // Create the terminal interface making it subscribe to messages before starting wasmind
        let tui = tui::Tui::new(
            tui_config,
            coordinator.get_sender(),
            cli.prompt.clone(),
            context.clone(),
        );

        // Start the wasmind system
        let starting_actors: Vec<&str> =
            config.starting_actors.iter().map(|s| s.as_str()).collect();
        coordinator
            .start_wasmind(&starting_actors, "Root Agent".to_string())
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
                agent: wasmind_actor_utils::STARTING_SCOPE.to_string(),
                message: ChatMessage::user(prompt),
            };
            coordinator.broadcast_common_message(add_message, false)?;
        }

        // Wait for the wasmind system to exit
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
