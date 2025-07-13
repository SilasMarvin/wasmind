use std::time::Duration;

use hive::actors::tools::command::CommandTool;
use hive::actors::tui::TuiActor;
use hive::actors::{Actor, ActorMessage, AssistantChatState, Message};
use hive::config::{Config, ParsedConfig};
use hive::hive::MAIN_MANAGER_SCOPE;
use hive::llm_client::ChatMessage;
use tokio::sync::broadcast;
use tracing::info;

use crate::utils::create_command_tool_call;

pub async fn run() {
    info!("Starting command execution scenario");

    // Create config
    let config: ParsedConfig = Config::new(false).unwrap().try_into().unwrap();
    let scope = MAIN_MANAGER_SCOPE;

    // Set up broadcast channel
    let (tx, _rx) = broadcast::channel(1000);

    // Create actors
    TuiActor::new(config.clone(), tx.clone(), scope.clone()).run();
    CommandTool::new(config.clone(), tx.clone(), scope.clone()).run();

    let mut chat_history = vec![ChatMessage::user("Use the command tool do something!")];

    let (command_tool_call_actor_message, assistant_response_chat_history_message) =
        create_command_tool_call(
            &scope,
            "sleep 10 && echo 'Command completed after 10 seconds!'",
            &[],
        );

    chat_history.push(assistant_response_chat_history_message);
    let _ = tx.send(ActorMessage {
        scope: scope.clone(),
        message: Message::AssistantChatUpdated(AssistantChatState {
            system: "System message..".to_string(),
            tools: vec![],
            messages: chat_history.clone(),
        }),
    });
    let _ = tx.send(command_tool_call_actor_message);

    tokio::time::sleep(Duration::from_secs(10_000)).await;
}
