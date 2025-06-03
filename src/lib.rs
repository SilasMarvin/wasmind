pub mod actors;
pub mod cli;
pub mod config;
pub mod hive;
pub mod key_bindings;
pub mod prompt_preview;
pub mod system_state;
pub mod template;

use std::sync::LazyLock;
use snafu::{Location, Snafu};
use tokio::runtime;

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
        source: config::ConfigError,
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

// Library functions that main.rs can use
pub fn init_logger() {
    use tracing_subscriber::{EnvFilter, FmtSubscriber};
    
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

pub fn run_main_program() -> SResult<()> {
    use config::{Config, ParsedConfig};
    use key_bindings::KeyBindingManager;
    use rdev::{Event, EventType, listen};
    use snafu::ResultExt;
    use tokio::runtime;
    use tracing::{error, info};
    
    let config = Config::new().context(ConfigSnafu)?;
    let parsed_config: ParsedConfig = config.try_into().context(ConfigSnafu)?;
    let mut key_binding_manager = KeyBindingManager::from(&parsed_config.keys);

    // Create the tokio runtime in main thread
    let runtime = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    // Start the HIVE multi-agent system
    let hive_handle = hive::start_hive(&runtime, parsed_config);

    // Clone the message sender for the callback
    let message_tx = hive_handle.message_tx.clone();

    // Spawn a thread to monitor for exit
    std::thread::spawn(move || {
        // Wait for exit signal from HIVE system
        let _ = hive_handle.exit_rx.recv();
        info!("Received exit signal from HIVE system, exiting...");
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

pub fn run_headless_program(prompt: String, auto_approve_commands_override: bool) -> SResult<()> {
    use config::{Config, ParsedConfig};
    use snafu::ResultExt;
    use tokio::runtime;
    use tracing::info;
    
    let config = Config::new().context(ConfigSnafu)?;
    let mut parsed_config: ParsedConfig = config.try_into().context(ConfigSnafu)?;

    // Override config setting if CLI flag is provided
    if auto_approve_commands_override {
        parsed_config.auto_approve_commands = true;
    }

    // Create the tokio runtime in main thread
    let runtime = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    // Start the HIVE system without TUI
    let hive_handle = hive::start_headless_hive(&runtime, parsed_config, prompt);

    // Wait for exit signal from HIVE system
    let _ = hive_handle.exit_rx.recv();
    info!("Received exit signal from HIVE system, exiting...");

    Ok(())
}