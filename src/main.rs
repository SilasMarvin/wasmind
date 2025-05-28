use std::sync::LazyLock;

use config::{Config, ConfigError, ParsedConfig};
use crossbeam::channel::unbounded;
use key_bindings::KeyBindingManager;
use rdev::{Event, EventType, listen};
use snafu::{Location, ResultExt, Snafu};
use tokio::runtime;
use tracing::error;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

mod assistant;
mod cli;
mod config;
mod context;
mod key_bindings;
mod mcp;
mod prompt_preview;
pub mod system_state;
pub mod template;
pub mod tools;
mod tui;
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

    #[snafu(display("Error sending MCP task"))]
    MCPTaskSend {
        #[snafu(implicit)]
        location: Location,
        #[snafu(source)]
        source: crossbeam::channel::SendError<mcp::Task>,
    },

    #[snafu(display("Error sending microphone task"))]
    MicrophoneTaskSend {
        #[snafu(implicit)]
        location: Location,
        #[snafu(source)]
        source: crossbeam::channel::SendError<context::microphone::Task>,
    },

    #[snafu(display("Error sending assistant task"))]
    AssistantTaskSend {
        #[snafu(implicit)]
        location: Location,
        #[snafu(source)]
        source: crossbeam::channel::SendError<assistant::Task>,
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

    let (tx, rx) = unbounded();
    let local_tx = tx.clone();
    let _worker_handle = std::thread::spawn(move || {
        worker::execute_worker(local_tx, rx, parsed_config);
    });

    let callback = move |event: Event| match event.event_type {
        EventType::KeyPress(key) => {
            let actions = key_binding_manager.handle_event(key);
            for action in actions {
                if let Err(e) = tx.send(worker::Event::Action(action)) {
                    error!("Error sending action to worker: {e:?}");
                }
            }
        }
        EventType::KeyRelease(_) => {
            key_binding_manager.clear();
        }
        _ => (),
    };

    // This will block and has to be in the main thread
    if let Err(error) = listen(callback) {
        error!("Error listening for global key events: {:?}", error)
    }

    Ok(())
}
