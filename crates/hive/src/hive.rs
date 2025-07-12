use snafu::ResultExt;
use tokio::{select, sync::broadcast, task::JoinHandle};

use crate::{
    LiteLLMSnafu, SResult,
    actors::{
        Actor, ActorMessage, AgentMessage, AgentMessageType, AgentStatus, AgentType,
        InterAgentMessage, Message,
        agent::Agent,
        tools::{
            complete::CompleteTool, planner::Planner, send_message::SendMessage,
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

/// Start the HIVE multi-agent system with TUI
pub async fn start_hive(config: ParsedConfig) -> SResult<()> {
    let (tx, _) = broadcast::channel::<ActorMessage>(1024);

    let mut rx = tx.subscribe();

    // Create the TUI before waiting for LiteLLM to come up
    TuiActor::new(config.clone(), tx.clone(), MAIN_MANAGER_SCOPE).run();

    let mut join_handle: Option<JoinHandle<SResult<()>>> = Some(tokio::spawn(async move {
        // Start LiteLLM Docker container
        let litellm_config = LiteLLMConfig {
            port: config.hive.litellm.port,
            image: config.hive.litellm.image.clone(),
            container_name: config.hive.litellm.container_name.clone(),
            auto_remove: config.hive.litellm.auto_remove,
            env_overrides: config.hive.litellm.env_overrides.clone(),
        };
        LiteLLMManager::start(&litellm_config, &config)
            .await
            .context(LiteLLMSnafu)?;

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
        ]);

        // Start the Main Manager
        main_manager.run();

        Ok(())
    }));

    loop {
        let msg = if let Some(handle) = join_handle.take()
            && !handle.is_finished()
        {
            select! {
                res = handle => {
                    res.expect("Error joining manager spawn process in hive")?;
                    join_handle = None;
                    None
                },
                msg = rx.recv() => Some(msg)
            }
        } else {
            Some(rx.recv().await)
        };

        if let Some(msg) = msg {
            let msg = msg.expect("Error receiving in hive");
            let message_json = serde_json::to_string(&msg).unwrap_or_else(|_| format!("{:?}", msg));
            tracing::debug!(name = "hive_received_message", message = %message_json, message_type = std::any::type_name::<Message>());

            match msg.message {
                Message::Exit if msg.scope == MAIN_MANAGER_SCOPE => {
                    // This is a horrible hack to let the tui restore the terminal first
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    return Ok(());
                }
                _ => (),
            }
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
            return Ok(());
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
    )
    .with_scope(MAIN_MANAGER_SCOPE)
    .with_actors([
        Planner::ACTOR_ID,
        SpawnAgent::ACTOR_ID,
        SendMessage::ACTOR_ID,
        WaitTool::ACTOR_ID,
        CompleteTool::ACTOR_ID,
    ]);

    // Start the Main Manager
    main_manager.run();

    // Listen for exit signals and broadcast them
    loop {
        let msg = rx.recv().await.expect("Error receiving in hive");

        let message_json = serde_json::to_string(&msg).unwrap();
        tracing::debug!(name = "hive_message", message_type = std::any::type_name::<Message>(), message = %message_json);

        match msg.message {
            Message::Exit if msg.scope == MAIN_MANAGER_SCOPE => return Ok(()),
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
