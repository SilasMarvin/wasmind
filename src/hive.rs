use crossbeam::channel;
use tokio::sync::broadcast;

use crate::{
    actors::{
        Action, Actor, ActorMessage, AgentMessage, AgentMessageType, AgentStatus, AgentType,
        InterAgentMessage, Message, agent::Agent, tui::TuiActor,
    },
    config::ParsedConfig,
    scope::Scope,
};

#[cfg(feature = "gui")]
use crate::actors::context::Context;
#[cfg(feature = "audio")]
use crate::actors::microphone::Microphone;

pub const ROOT_AGENT_SCOPE: Scope =
    Scope::from_uuid(uuid::uuid!("29443a2e-78e1-4983-975a-d68b0e6c4cf0"));

/// Handle for communicating with the HIVE system
pub struct HiveHandle {
    /// Sender to send messages to the main manager
    pub message_tx: broadcast::Sender<ActorMessage>,
    /// Receiver to know when the system should exit
    pub exit_rx: channel::Receiver<()>,
}

/// Start the HIVE multi-agent system with TUI
#[tracing::instrument(name = "start_hive", skip(runtime, config))]
pub fn start_hive(runtime: &tokio::runtime::Runtime, config: ParsedConfig) -> HiveHandle {
    // Create crossbeam channel for exit notification
    let (exit_tx, exit_rx) = channel::bounded(1);

    // Create broadcast channel for TUI and context actors
    let (tx, _) = broadcast::channel::<ActorMessage>(1024);
    let message_tx = tx.clone();

    // Spawn the HIVE system task
    runtime.spawn(async move {
        let mut rx = tx.subscribe();

        // Create and run TUI and Context actors (these are shared across all agents)
        TuiActor::new(config.clone(), tx.clone(), ROOT_AGENT_SCOPE).run();
        #[cfg(feature = "gui")]
        Context::new(config.clone(), tx.clone(), ROOT_AGENT_SCOPE).run();
        #[cfg(feature = "audio")]
        Microphone::new(config.clone(), tx.clone(), ROOT_AGENT_SCOPE).run();

        // Create the Main Manager agent
        let main_manager = Agent::new_with_scope(
            tx.clone(),
            crate::actors::agent::MAIN_MANAGER_ROLE.to_string(),
            None,
            config.clone(),
            ROOT_AGENT_SCOPE,
            Scope::new(), // parent_scope means nothing for the MainManager
            AgentType::MainManager
        );

        // Start the Main Manager
        main_manager.run();

        // Keep the runtime alive and listen for exit signals
        loop {
            let msg = rx.recv().await.expect("Error receiving in hive");
            let message_json = serde_json::to_string(&msg).unwrap_or_else(|_| format!("{:?}", msg));
            tracing::debug!(name = "hive_received_message", message = %message_json, message_type = std::any::type_name::<Message>());

            match msg.message {
                Message::Action(Action::Exit) if msg.scope == ROOT_AGENT_SCOPE => {
                    // This is a horrible hack to let the tui restore the terminal first
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    // Notify main thread that we're exiting
                    let _ = exit_tx.send(());
                    break;
                }
                _ => ()
            }
        }
    });

    HiveHandle {
        message_tx,
        exit_rx,
    }
}

/// Start the HIVE multi-agent system in headless mode
#[tracing::instrument(name = "start_headless_hive", skip(runtime, config, tx), fields(prompt_length = initial_prompt.len()))]
pub fn start_headless_hive(
    runtime: &tokio::runtime::Runtime,
    config: ParsedConfig,
    initial_prompt: String,
    tx: Option<broadcast::Sender<ActorMessage>>,
) -> HiveHandle {
    // Create crossbeam channel for exit notification
    let (exit_tx, exit_rx) = channel::bounded(1);

    let tx = tx.unwrap_or_else(|| broadcast::channel::<ActorMessage>(1024).0);

    let message_tx = tx.clone();
    runtime.spawn(async move {
        let mut rx = tx.subscribe();

        // Create and run Context and Microphone actors (no TUI in headless mode)
        #[cfg(feature = "gui")]
        Context::new(config.clone(), tx.clone(), ROOT_AGENT_SCOPE).run();
        #[cfg(feature = "audio")]
        Microphone::new(config.clone(), tx.clone(), ROOT_AGENT_SCOPE).run();

        let main_manager = Agent::new_with_scope(
            tx.clone(),
            crate::actors::agent::MAIN_MANAGER_ROLE.to_string(),
            Some(initial_prompt),
            config.clone(),
            ROOT_AGENT_SCOPE,
            Scope::new(), // parent_scope means nothing for the MainManager 
            AgentType::MainManager
        );

        // Start the Main Manager
        main_manager.run();

        // Listen for exit signals and broadcast them
        loop {
            let msg = rx.recv().await.expect("Error receiving in hive");
            let message_json = serde_json::to_string(&msg).unwrap_or_else(|_| format!("{:?}", msg));
            tracing::debug!(name = "hive_message", message = %message_json, message_type = std::any::type_name::<Message>());

            match msg.message {
                Message::Action(Action::Exit) if msg.scope == ROOT_AGENT_SCOPE => {
                    let _ = exit_tx.send(());
                    break;
                }
                Message::Agent(AgentMessage {
                    agent_id,
                    message: AgentMessageType::InterAgentMessage(InterAgentMessage::StatusUpdate {
                        status: AgentStatus::Done(res),
                    })
                }) if agent_id == ROOT_AGENT_SCOPE => {
                    match res {
                        Ok(agent_task_result) => {
                            if agent_task_result.success {
                                println!("Success: {}", agent_task_result.summary);
                            } else {
                                eprintln!("Failed: {}", agent_task_result.summary);
                            }
                        },
                        Err(error_message) => {
                            eprintln!("Errored: {error_message}");
                        },
                    }
                }
                _ => ()
            }
        }
    });

    HiveHandle {
        message_tx,
        exit_rx,
    }
}
