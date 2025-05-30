use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use tracing::{error, info};

use crate::{
    actors::assistant::Assistant,
    actors::context::Context,
    actors::microphone::Microphone,
    actors::tool_discovery::ToolDiscovery,
    actors::tools::{
        command::Command, edit_file::EditFile, file_reader::FileReaderActor, planner::Planner,
    },
    actors::tui::TuiActor,
    actors::{Actor, Message},
    config::ParsedConfig,
};

pub fn execute_worker(config: ParsedConfig) {
    info!("Starting worker with actor system");

    // Create the tokio runtime
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    runtime.block_on(async {
        // Create broadcast channel for actor communication
        let (tx, mut rx) = broadcast::channel::<Message>(1024);

        // Create shared file reader that will be used by both FileReaderActor and EditFile
        let file_reader = Arc::new(Mutex::new(
            crate::actors::tools::file_reader::FileReader::new(),
        ));

        info!("Creating and starting actors...");

        // Create and run Core Actors
        TuiActor::new(config.clone(), tx.clone()).run();
        Assistant::new(config.clone(), tx.clone()).run();
        ToolDiscovery::new(config.clone(), tx.clone()).run();

        // Create and run Tool Actors
        Command::new(config.clone(), tx.clone()).run();
        FileReaderActor::with_file_reader(config.clone(), tx.clone(), file_reader.clone()).run();
        EditFile::with_file_reader(config.clone(), tx.clone(), file_reader).run();
        Planner::new(config.clone(), tx.clone()).run();

        // Create and run Context Actors
        Context::new(config.clone(), tx.clone()).run();
        Microphone::new(config.clone(), tx.clone()).run();

        // TODO: Create and run MCP actor when implemented

        info!("All actors started");

        // Keep the runtime alive
        // Listen for exit signals
        loop {
            if let Ok(Message::TUIExit) = rx.recv().await {
                info!("Received exit signal, shutting down");
                break;
            }
        }
    });
}
