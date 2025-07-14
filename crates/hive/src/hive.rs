use std::time::Duration;
use tokio::{sync::broadcast, time::sleep};

use crate::{
    SResult,
    actors::{
        Actor, ActorMessage, AgentMessage, AgentMessageType, AgentStatus, AgentType,
        InterAgentMessage, Message,
        agent::Agent,
        litellm_manager::LiteLLMManager,
        tools::{
            complete::CompleteTool, planner::Planner, send_message::SendMessage,
            spawn_agent::SpawnAgent, wait::WaitTool,
        },
        tui::TuiActor,
    },
    config::ParsedConfig,
    scope::Scope,
};

// #[cfg(feature = "gui")]
// use crate::actors::context::Context;
// #[cfg(feature = "audio")]
// use crate::actors::microphone::Microphone;

pub const MAIN_MANAGER_SCOPE: Scope =
    Scope::from_uuid(uuid::uuid!("00000000-0000-0000-0000-000000000000"));
pub const MAIN_MANAGER_ROLE: &str = "Main Manager";

/// Start the HIVE multi-agent system with TUI
pub async fn start_hive(config: ParsedConfig, mut initial_prompt: Option<String>) -> SResult<()> {
    let (tx, _) = broadcast::channel::<ActorMessage>(1024);

    let mut rx = tx.subscribe();

    // Create the TUI immediatly
    TuiActor::new(
        config.clone(),
        tx.clone(),
        MAIN_MANAGER_SCOPE,
        initial_prompt.clone(),
    )
    .run();

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
    )
    .with_scope(MAIN_MANAGER_SCOPE)
    .with_actors([
        Planner::ACTOR_ID,
        SpawnAgent::ACTOR_ID,
        SendMessage::ACTOR_ID,
        WaitTool::ACTOR_ID,
        LiteLLMManager::ACTOR_ID,
    ]);

    // Start the Main Manager
    main_manager.run();

    // Submit the initial user prompt if it exists
    if let Some(prompt) = initial_prompt.take() {
        sleep(Duration::from_millis(250)).await;
        let _ = tx.send(ActorMessage {
            scope: MAIN_MANAGER_SCOPE.clone(),
            message: Message::UserContext(crate::actors::UserContext::UserTUIInput(prompt)),
        });
    }

    // Listen for messages
    loop {
        let msg = rx.recv().await;
        let msg = msg.expect("Error receiving in hive");
        let message_json = serde_json::to_string(&msg).unwrap_or_else(|_| format!("{:?}", msg));
        tracing::debug!(name = "hive_received_message", message = %message_json, message_type = std::any::type_name::<Message>());

        match msg.message {
            Message::Exit if msg.scope == MAIN_MANAGER_SCOPE => {
                // Let everything clean up
                sleep(Duration::from_millis(500)).await;
                return Ok(());
            }
            _ => (),
        }
    }
}

/// Start the HIVE multi-agent system in headless mode
pub async fn start_headless_hive(
    config: ParsedConfig,
    initial_prompt: String,
    tx: Option<broadcast::Sender<ActorMessage>>,
) -> SResult<()> {
    let tx = tx.unwrap_or_else(|| broadcast::channel::<ActorMessage>(1024).0);

    let mut rx = tx.subscribe();

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
    )
    .with_scope(MAIN_MANAGER_SCOPE)
    .with_actors([
        Planner::ACTOR_ID,
        SpawnAgent::ACTOR_ID,
        SendMessage::ACTOR_ID,
        WaitTool::ACTOR_ID,
        CompleteTool::ACTOR_ID,
        LiteLLMManager::ACTOR_ID,
    ]);

    // Start the Main Manager
    main_manager.run();

    // Listen for exit signals and broadcast them
    loop {
        let msg = rx.recv().await.expect("Error receiving in hive");

        let message_json = serde_json::to_string(&msg).unwrap();
        tracing::debug!(name = "hive_message", message_type = std::any::type_name::<Message>(), message = %message_json);

        match msg.message {
            Message::Exit if msg.scope == MAIN_MANAGER_SCOPE => {
                sleep(Duration::from_millis(200)).await;
                return Ok(());
            }
            Message::Agent(AgentMessage {
                agent_id,
                message:
                    AgentMessageType::InterAgentMessage(InterAgentMessage::StatusUpdate {
                        status: AgentStatus::Done(res),
                    }),
            }) if agent_id == MAIN_MANAGER_SCOPE => match res {
                Ok(agent_task_result) => {
                    if agent_task_result.success {
                        println!("Success: {}", agent_task_result.summary);
                    } else {
                        eprintln!("Failed: {}", agent_task_result.summary);
                    }
                }
                Err(error_message) => {
                    eprintln!("Errored: {error_message}");
                }
            },
            _ => (),
        }
    }
}
