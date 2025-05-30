use crossbeam::channel;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use tracing::{error, info};

use crate::{
    actors::{
        Action, Actor, Message,
        assistant::Assistant,
        context::Context,
        microphone::Microphone,
        tools::{
            command::Command, edit_file::EditFile, file_reader::FileReaderActor, mcp::MCP, planner::Planner,
        },
        tui::TuiActor,
    },
    config::ParsedConfig,
};

/// Handle for communicating with the worker thread
pub struct WorkerHandle {
    /// Sender to send messages to the actor system
    pub message_tx: broadcast::Sender<Message>,
    /// Receiver to know when the system should exit
    pub exit_rx: channel::Receiver<()>,
}

pub fn start_actors(runtime: &tokio::runtime::Runtime, config: ParsedConfig) -> WorkerHandle {
    info!("Starting actor system");

    // Create crossbeam channel for exit notification
    let (exit_tx, exit_rx) = channel::bounded(1);

    // Create broadcast channel for actor communication
    let (tx, _) = broadcast::channel::<Message>(1024);
    let message_tx = tx.clone();

    // Spawn the actor system task
    runtime.spawn(async move {
        let mut rx = tx.subscribe();

        // Create shared file reader that will be used by both FileReaderActor and EditFile
        let file_reader = Arc::new(Mutex::new(
            crate::actors::tools::file_reader::FileReader::new(),
        ));

        info!("Creating and starting actors...");

        // Create and run Core Actors
        TuiActor::new(config.clone(), tx.clone()).run();
        Assistant::new(config.clone(), tx.clone()).run();

        // Create and run Tool Actors
        Command::new(config.clone(), tx.clone()).run();
        FileReaderActor::with_file_reader(config.clone(), tx.clone(), file_reader.clone()).run();
        EditFile::with_file_reader(config.clone(), tx.clone(), file_reader).run();
        Planner::new(config.clone(), tx.clone()).run();

        // Create and run Context Actors
        Context::new(config.clone(), tx.clone()).run();
        Microphone::new(config.clone(), tx.clone()).run();

        // Create and run MCP actor
        MCP::new(config.clone(), tx.clone()).run();

        info!("All actors started");

        // Keep the runtime alive
        // Listen for exit signals
        loop {
            if let Ok(Message::Action(Action::Exit)) = rx.recv().await {
                info!("Received exit signal, shutting down");
                // Notify main thread that we're exiting
                let _ = exit_tx.send(());
                break;
            }
        }
    });

    WorkerHandle {
        message_tx,
        exit_rx,
    }
}

pub fn start_headless_actors(runtime: &tokio::runtime::Runtime, config: ParsedConfig, initial_prompt: String) -> WorkerHandle {
    info!("Starting headless actor system");

    // Create crossbeam channel for exit notification
    let (exit_tx, exit_rx) = channel::bounded(1);

    // Create broadcast channel for actor communication
    let (tx, _) = broadcast::channel::<Message>(1024);
    let message_tx = tx.clone();

    // Spawn the actor system task
    runtime.spawn(async move {
        let mut rx = tx.subscribe();

        // Create shared file reader that will be used by both FileReaderActor and EditFile
        let file_reader = Arc::new(Mutex::new(
            crate::actors::tools::file_reader::FileReader::new(),
        ));

        info!("Creating and starting headless actors...");

        // Create and run Core Actors (excluding TuiActor)
        Assistant::new(config.clone(), tx.clone()).run();

        // Create and run Tool Actors
        Command::new(config.clone(), tx.clone()).run();
        FileReaderActor::with_file_reader(config.clone(), tx.clone(), file_reader.clone()).run();
        EditFile::with_file_reader(config.clone(), tx.clone(), file_reader).run();
        Planner::new(config.clone(), tx.clone()).run();

        // Create and run Context Actors
        Context::new(config.clone(), tx.clone()).run();
        Microphone::new(config.clone(), tx.clone()).run();

        // Create and run MCP actor
        MCP::new(config.clone(), tx.clone()).run();

        info!("All headless actors started");

        // Track which actors are ready
        let mut ready_actors = std::collections::HashSet::new();
        let required_actors = vec![
            Assistant::ACTOR_ID,
            Command::ACTOR_ID,
            FileReaderActor::ACTOR_ID,
            EditFile::ACTOR_ID,
            Planner::ACTOR_ID,
            Context::ACTOR_ID,
            Microphone::ACTOR_ID,
            MCP::ACTOR_ID,
        ];
        let mut initial_prompt_sent = false;

        // Keep the runtime alive and listen for exit signals or completion
        loop {
            match rx.recv().await {
                Ok(Message::ActorReady { actor_id }) => {
                    info!("Actor {} is ready", actor_id);
                    ready_actors.insert(actor_id);
                    
                    // Check if all required actors are ready and we haven't sent the initial prompt yet
                    if !initial_prompt_sent && required_actors.iter().all(|id| ready_actors.contains(id)) {
                        info!("All actors ready, sending initial prompt");
                        let _ = tx.send(Message::UserTUIInput(initial_prompt.clone()));
                        initial_prompt_sent = true;
                    }
                }
                Ok(Message::Action(Action::Exit)) => {
                    info!("Received exit signal, shutting down");
                    let _ = exit_tx.send(());
                    break;
                }
                Ok(Message::AssistantResponse(content)) => {
                    // Check if this is a final response (no tool calls)
                    match content {
                        genai::chat::MessageContent::Text(_) => {
                            info!("Assistant responded with text only, exiting headless mode");
                            let _ = exit_tx.send(());
                            break;
                        }
                        genai::chat::MessageContent::Parts(_) => {
                            // Parts without tool calls means final response
                            info!("Assistant responded with parts only, exiting headless mode");
                            let _ = exit_tx.send(());
                            break;
                        }
                        genai::chat::MessageContent::ToolCalls(_) => {
                            // Tool calls present, continue processing
                        }
                        _ => {
                            // For other content types, continue processing
                        }
                    }
                }
                Ok(_) => {
                    // Continue processing other messages
                }
                Err(_) => {
                    // Channel error, exit
                    break;
                }
            }
        }
    });

    WorkerHandle {
        message_tx,
        exit_rx,
    }
}
