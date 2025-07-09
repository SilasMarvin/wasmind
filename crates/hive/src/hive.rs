use crossbeam::channel;
use tokio::sync::broadcast;

use crate::{
    actors::{
        Action, Actor, ActorMessage, AgentMessage, AgentMessageType, AgentStatus, AgentType,
        InterAgentMessage, Message,
        agent::Agent,
        tools::{
            complete::Complete, planner::Planner, send_message::SendMessage,
            spawn_agent::SpawnAgent, wait::WaitTool,
        },
        tui::TuiActor,
    },
    config::ParsedConfig,
    litellm_manager::{LiteLLMConfig, LiteLLMManager},
    scope::Scope,
};

// #[cfg(feature = "gui")]
// use crate::actors::context::Context;
// #[cfg(feature = "audio")]
// use crate::actors::microphone::Microphone;

pub const MAIN_MANAGER_SCOPE: Scope =
    Scope::from_uuid(uuid::uuid!("00000000-0000-0000-0000-000000000000"));
pub const MAIN_MANAGER_ROLE: &str = "Main Manager";

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

        // Create the TUI before waiting for LiteLLM to come up
        TuiActor::new(config.clone(), tx.clone(), MAIN_MANAGER_SCOPE).run();

        // Start LiteLLM Docker container
        let litellm_config = LiteLLMConfig {
            port: config.hive.litellm.port,
            image: config.hive.litellm.image.clone(),
            container_name: config.hive.litellm.container_name.clone(),
            auto_remove: config.hive.litellm.auto_remove,
            env_overrides: config.hive.litellm.env_overrides.clone(),
        };
        let _litellm_manager = match LiteLLMManager::start(&litellm_config, &config).await {
            Ok(manager) => manager,
            Err(e) => {
                tracing::error!("Failed to start LiteLLM container: {}", e);
                let _ = exit_tx.send(());
                return;
            }
        };

        // #[cfg(feature = "gui")]
        // Context::new(config.clone(), tx.clone(), ROOT_AGENT_SCOPE).run();
        // #[cfg(feature = "audio")]
        // Microphone::new(config.clone(), tx.clone(), ROOT_AGENT_SCOPE).run();

        // Create the Main Manager agent
        let main_manager = Agent::new(
            tx.clone(),
            MAIN_MANAGER_ROLE.to_string(),
            None,
            config.clone(),
            Scope::new(), // parent_scope means nothing for the MainManager
            AgentType::MainManager,
        ).with_scope(MAIN_MANAGER_SCOPE)
        .with_actors([Planner::ACTOR_ID, SpawnAgent::ACTOR_ID, SendMessage::ACTOR_ID, WaitTool::ACTOR_ID]);

        // Start the Main Manager
        main_manager.run();

        // Keep the runtime alive and listen for exit signals
        loop {
            let msg = rx.recv().await.expect("Error receiving in hive");
            let message_json = serde_json::to_string(&msg).unwrap_or_else(|_| format!("{:?}", msg));
            tracing::debug!(name = "hive_received_message", message = %message_json, message_type = std::any::type_name::<Message>());

            match msg.message {
                Message::Action(Action::Exit) if msg.scope == MAIN_MANAGER_SCOPE => {
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

        // Start LiteLLM Docker container
        let litellm_config = LiteLLMConfig {
            port: config.hive.litellm.port,
            image: config.hive.litellm.image.clone(),
            container_name: config.hive.litellm.container_name.clone(),
            auto_remove: config.hive.litellm.auto_remove,
            env_overrides: config.hive.litellm.env_overrides.clone(),
        };
        let _litellm_manager = match LiteLLMManager::start(&litellm_config, &config).await {
            Ok(manager) => manager,
            Err(e) => {
                tracing::error!("Failed to start LiteLLM container: {}", e);
                let _ = exit_tx.send(());
                return;
            }
        };

        // // Create and run Context and Microphone actors (no TUI in headless mode)
        // #[cfg(feature = "gui")]
        // Context::new(config.clone(), tx.clone(), ROOT_AGENT_SCOPE).run();
        // #[cfg(feature = "audio")]
        // Microphone::new(config.clone(), tx.clone(), ROOT_AGENT_SCOPE).run();

        let main_manager = Agent::new(
            tx.clone(),
            MAIN_MANAGER_ROLE.to_string(),
            Some(initial_prompt),
            config.clone(),
            Scope::new(), // parent_scope means nothing for the MainManager 
            AgentType::MainManager,
        ).with_scope(MAIN_MANAGER_SCOPE)
        .with_actors([Planner::ACTOR_ID, SpawnAgent::ACTOR_ID, SendMessage::ACTOR_ID, WaitTool::ACTOR_ID, Complete::ACTOR_ID]);

        // Start the Main Manager
        main_manager.run();

        // Listen for exit signals and broadcast them
        loop {
            let msg = rx.recv().await.expect("Error receiving in hive");

            let message_json = serde_json::to_string(&msg).unwrap();
            tracing::debug!(name = "hive_message", message_type = std::any::type_name::<Message>(), message = %message_json);

            match msg.message {
                Message::Action(Action::Exit) if msg.scope == MAIN_MANAGER_SCOPE => {
                    let _ = exit_tx.send(());
                    break;
                }
                Message::Agent(AgentMessage {
                    agent_id,
                    message: AgentMessageType::InterAgentMessage(InterAgentMessage::StatusUpdate {
                        status: AgentStatus::Done(res),
                    })
                }) if agent_id == MAIN_MANAGER_SCOPE => {
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
