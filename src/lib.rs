pub mod actors;
pub mod cli;
pub mod config;
pub mod hive;
pub mod key_bindings;
pub mod prompt_preview;
pub mod system_state;
pub mod template;

use actors::Message;
use hive::ROOT_AGENT_SCOPE;
use snafu::{Location, Snafu};
use std::sync::LazyLock;
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
    init_logger_with_path("log.txt");
}

pub fn init_logger_with_path<P: AsRef<std::path::Path>>(log_path: P) {
    use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

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
                .with_thread_ids(true)
                .with_timer(tracing_subscriber::fmt::time::time())
                .compact(),
        )
        .init();
}

pub fn run_main_program() -> SResult<()> {
    use config::{Config, ParsedConfig};
    use key_bindings::KeyBindingManager;
    #[cfg(feature = "gui")]
    use key_bindings::RdevToCrosstermConverter;
    #[cfg(feature = "gui")]
    use rdev::{Event, EventType, listen};
    use snafu::ResultExt;
    use tokio::runtime;
    use tracing::{error, info};

    let config = Config::new().context(ConfigSnafu)?;
    let parsed_config: ParsedConfig = config.try_into().context(ConfigSnafu)?;

    // Create the tokio runtime in main thread
    let runtime = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    // Start the HIVE multi-agent system
    let hive_handle = hive::start_hive(&runtime, parsed_config.clone());

    #[cfg(feature = "gui")]
    {
        let mut key_binding_manager = KeyBindingManager::from(&parsed_config.keys);
        let mut rdev_converter = RdevToCrosstermConverter::new();

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
                if let Some(key_event) = rdev_converter.handle_key_press(key) {
                    let actions = key_binding_manager.handle_event(key_event);
                    for action in actions {
                        if let Err(e) = message_tx.send(actors::ActorMessage {
                            scope: ROOT_AGENT_SCOPE,
                            message: Message::Action(action),
                        }) {
                            error!("Error sending action to actors: {:?}", e);
                        }
                    }
                }
            }
            EventType::KeyRelease(key) => {
                rdev_converter.handle_key_release(key);
            }
            _ => (),
        };

        info!("Starting global key listener");

        // This will block and has to be in the main thread
        if let Err(error) = listen(callback) {
            error!("Error listening for global key events: {:?}", error)
        }
    }

    #[cfg(not(feature = "gui"))]
    {
        // Wait for exit signal from HIVE system
        let _ = hive_handle.exit_rx.recv();
        info!("Received exit signal from HIVE system, exiting...");
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
    let hive_handle = hive::start_headless_hive(&runtime, parsed_config, prompt, None);

    // Wait for exit signal from HIVE system
    let _ = hive_handle.exit_rx.recv();
    info!("Received exit signal from HIVE system, exiting...");

    Ok(())
}
