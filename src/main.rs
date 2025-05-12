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
mod config;
mod context;
mod key_bindings;
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
}

pub type SResult<T> = Result<T, Error>;

// Builds a tracing subscriber from the `FILLER_NAME` environment variable
// If the variables value is malformed or missing, sets the default log level to ERROR
fn init_logger() {
    let builder = FmtSubscriber::builder().with_env_filter(EnvFilter::from_env("FILLER_NAME"));

    builder
        .with_writer(std::io::stderr)
        .without_time()
        .with_ansi(false)
        .init()
}

fn main() -> SResult<()> {
    init_logger();

    let config = Config::new().context(ConfigSnafu)?;
    let parsed_config: ParsedConfig = config.try_into().context(ConfigSnafu)?;
    let mut key_binding_manager = KeyBindingManager::from(&parsed_config.keys);

    let (tx, rx) = unbounded();
    let local_tx = tx.clone();
    let _worker_handle = std::thread::spawn(move || {
        worker::execute_worker(local_tx, rx, parsed_config);
    });

    let terminal_tx = tx.clone();
    std::thread::spawn(move || {
        let mut buffer = String::new();
        loop {
            buffer.clear();
            if let Ok(_) = std::io::stdin().read_line(&mut buffer) {
                let input = buffer.trim().to_string();
                if !input.is_empty() {
                    if let Err(e) = terminal_tx.send(worker::Event::UserTUIInput(input)) {
                        error!("Error sending terminal input to worker: {e:?}");
                    }
                }
            }
        }
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
