use std::sync::LazyLock;

use config::{Config, ConfigError, ParsedConfig};
use key_bindings::KeyBindingManager;
use rdev::{Event, EventType, listen};
use snafu::{Location, ResultExt, Snafu};
use tokio::runtime;
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

pub mod actors;

mod cli;
mod config;
mod key_bindings;
mod prompt_preview;
pub mod system_state;
pub mod template;
mod worker;

pub static TOKIO_RUNTIME: LazyLock<runtime::Runtime> = LazyLock::new(|| {
    runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("Error building tokio runtime")
});

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Config Error"))]
    Config {
        #[snafu(source)]
        source: ConfigError,
    },

    #[snafu(display("Error copying clipboard"))]
    Clipboard {
        #[snafu(implicit)]
        location: Location,
        #[snafu(source)]
        source: arboard::Error,
    },

    #[snafu(display("Error copying clipboard"))]
    Xcap {
        #[snafu(implicit)]
        location: Location,
        #[snafu(source)]
        source: xcap::XCapError,
    },

    #[snafu(display("Error with GenAI"))]
    Genai {
        #[snafu(implicit)]
        location: Location,
        #[snafu(source)]
        source: genai::Error,
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

// Builds a tracing subscriber from the `FILLER_NAME` environment variable
// If the variables value is malformed or missing, sets the default log level to ERROR
fn init_logger() {
    let builder = FmtSubscriber::builder().with_env_filter(EnvFilter::from_env("FILLER_NAME"));

    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open("log.txt")
        .expect("Unable to open log file");

    builder
        .with_writer(file)
        .without_time()
        .with_ansi(false)
        .init()
}

fn main() -> SResult<()> {
    use clap::Parser;

    init_logger();

    // Parse command line arguments
    let cli = cli::Cli::parse();

    match cli.command.unwrap_or_default() {
        cli::Commands::Run => {
            run_main_program()?;
        }
        cli::Commands::Headless { prompt } => {
            run_headless_program(prompt)?;
        }
        cli::Commands::PromptPreview {
            all,
            empty,
            files,
            plan,
            complete,
            config,
        } => {
            if let Err(e) = prompt_preview::execute_demo(all, empty, files, plan, complete, config)
            {
                eprintln!("Prompt preview error: {}", e);
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

fn run_main_program() -> SResult<()> {
    let config = Config::new().context(ConfigSnafu)?;
    let parsed_config: ParsedConfig = config.try_into().context(ConfigSnafu)?;
    let mut key_binding_manager = KeyBindingManager::from(&parsed_config.keys);

    // Create the tokio runtime in main thread
    let runtime = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    // Start the actor system
    let worker_handle = worker::start_actors(&runtime, parsed_config);

    // Clone the message sender for the callback
    let message_tx = worker_handle.message_tx.clone();

    // Spawn a thread to monitor for exit
    std::thread::spawn(move || {
        // Wait for exit signal from actors
        let _ = worker_handle.exit_rx.recv();
        info!("Received exit signal from actors, exiting...");
        std::process::exit(0);
    });

    let callback = move |event: Event| match event.event_type {
        EventType::KeyPress(key) => {
            let actions = key_binding_manager.handle_event(key);
            for action in actions {
                if let Err(e) = message_tx.send(actors::Message::Action(action)) {
                    error!("Error sending action to actors: {:?}", e);
                }
            }
        }
        EventType::KeyRelease(_) => {
            key_binding_manager.clear();
        }
        _ => (),
    };

    info!("Starting global key listener");

    // This will block and has to be in the main thread
    if let Err(error) = listen(callback) {
        error!("Error listening for global key events: {:?}", error)
    }

    Ok(())
}

fn run_headless_program(prompt: String) -> SResult<()> {
    let config = Config::new().context(ConfigSnafu)?;
    let parsed_config: ParsedConfig = config.try_into().context(ConfigSnafu)?;

    // Create the tokio runtime in main thread
    let runtime = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    // Start the actor system without TUI
    let worker_handle = worker::start_headless_actors(&runtime, parsed_config, prompt);

    // Wait for exit signal from actors
    let _ = worker_handle.exit_rx.recv();
    info!("Received exit signal from actors, exiting...");

    Ok(())
}
