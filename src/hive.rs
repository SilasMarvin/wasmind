use crossbeam::channel;
use tokio::sync::broadcast;

use crate::{
    actors::{Action, Actor, Message, agent::Agent, tui::TuiActor},
    config::ParsedConfig,
};

#[cfg(feature = "gui")]
use crate::actors::context::Context;
#[cfg(feature = "audio")]
use crate::actors::microphone::Microphone;

/// Handle for communicating with the HIVE system
pub struct HiveHandle {
    /// Sender to send messages to the main manager
    pub message_tx: broadcast::Sender<Message>,
    /// Receiver to know when the system should exit
    pub exit_rx: channel::Receiver<()>,
}

/// Start the HIVE multi-agent system with TUI
#[tracing::instrument(name = "start_hive", skip(runtime, config))]
pub fn start_hive(runtime: &tokio::runtime::Runtime, config: ParsedConfig) -> HiveHandle {
    // Create crossbeam channel for exit notification
    let (exit_tx, exit_rx) = channel::bounded(1);

    // Create broadcast channel for TUI and context actors
    let (tx, _) = broadcast::channel::<Message>(1024);
    let message_tx = tx.clone();

    // Spawn the HIVE system task
    runtime.spawn(async move {
        let mut rx = tx.subscribe();

        // Create and run TUI and Context actors (these are shared across all agents)
        TuiActor::new(config.clone(), tx.clone()).run();
        #[cfg(feature = "gui")]
        Context::new(config.clone(), tx.clone()).run();
        #[cfg(feature = "audio")]
        Microphone::new(config.clone(), tx.clone()).run();

        // Create the Main Manager agent
        let main_manager = Agent::new_manager(
            crate::actors::agent::MAIN_MANAGER_ROLE.to_string(),
            "Assist the user with their software engineering tasks".to_string(),
            config.clone(),
        );

        // Start the Main Manager in its own task
        tokio::spawn(async move {
            main_manager.run().await;
        });

        // Keep the runtime alive and listen for exit signals
        loop {
            let msg = rx.recv().await.expect("Error receiving in hive");
            tracing::debug!(name = "hive_received_message", message = ?msg);
            if let Message::Action(Action::Exit) = msg {
                // This is a horrible hack to let the tui restore the terminal first
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                // Notify main thread that we're exiting
                let _ = exit_tx.send(());
                break;
            }
        }
    });

    HiveHandle {
        message_tx,
        exit_rx,
    }
}

/// Start the HIVE multi-agent system in headless mode
#[tracing::instrument(name = "start_headless_hive", skip(runtime, config), fields(prompt_length = initial_prompt.len()))]
pub fn start_headless_hive(
    runtime: &tokio::runtime::Runtime,
    config: ParsedConfig,
    initial_prompt: String,
) -> HiveHandle {
    // Create crossbeam channel for exit notification
    let (exit_tx, exit_rx) = channel::bounded(1);

    // Create broadcast channel for shared actors
    let (tx, _) = broadcast::channel::<Message>(1024);
    let message_tx = tx.clone();

    // Spawn the HIVE system task
    runtime.spawn(async move {
        let mut rx = tx.subscribe();

        // Create and run Context and Microphone actors (no TUI in headless mode)
        #[cfg(feature = "gui")]
        Context::new(config.clone(), tx.clone()).run();
        #[cfg(feature = "audio")]
        Microphone::new(config.clone(), tx.clone()).run();

        // Track when context actors are ready
        let mut ready_actors = std::collections::HashSet::new();
        let mut required_actors: Vec<&'static str> = Vec::new();
        #[cfg(feature = "gui")]
        required_actors.push(Context::ACTOR_ID);
        #[cfg(feature = "audio")]
        required_actors.push(Microphone::ACTOR_ID);
        let mut main_manager_started = false;

        // If no shared actors are required (headless build), start main manager immediately
        if required_actors.is_empty() {
            // Create the Main Manager agent with the initial prompt as its task
            let main_manager = Agent::new_manager(
                crate::actors::agent::MAIN_MANAGER_ROLE.to_string(),
                initial_prompt.clone(),
                config.clone(),
            );

            // Start the Main Manager in its own task
            let exit_tx = tx.clone();
            tokio::spawn(async move {
                main_manager.run().await;
                // When the main manager completes, send exit signal
                let _ = exit_tx.send(Message::Action(Action::Exit));
            });
            main_manager_started = true;
        }

        // Keep the runtime alive and listen for exit signals
        loop {
            match rx.recv().await {
                Ok(Message::ActorReady { actor_id }) => {
                    ready_actors.insert(actor_id);

                    // Check if all required actors are ready and we haven't started the main manager yet
                    if !main_manager_started
                        && required_actors.iter().all(|id| ready_actors.contains(id))
                    {
                        // Create the Main Manager agent with the initial prompt as its task
                        let main_manager = Agent::new_manager(
                            crate::actors::agent::MAIN_MANAGER_ROLE.to_string(),
                            initial_prompt.clone(),
                            config.clone(),
                        );

                        // Start the Main Manager in its own task
                        let exit_tx = tx.clone();
                        tokio::spawn(async move {
                            main_manager.run().await;
                            // When the main manager completes, send exit signal
                            let _ = exit_tx.send(Message::Action(Action::Exit));
                        });
                        main_manager_started = true;
                    }
                }
                Ok(Message::Action(Action::Exit)) => {
                    let _ = exit_tx.send(());
                    break;
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

    HiveHandle {
        message_tx,
        exit_rx,
    }
}
