use crossbeam::channel::{Receiver, Sender};
use genai::{Client, chat::ChatRequest};
use snafu::ResultExt;
use tracing::error;

use crate::{GenaiSnafu, SResult, TOKIO_RUNTIME, config::ParsedConfig, worker};

/// Tasks the assistant can receive from the worker
#[derive(Debug, Clone)]
pub enum Task {
    Assist(ChatRequest),
    Cancel,
}

pub fn execute_assistant(tx: Sender<worker::Event>, rx: Receiver<Task>, config: ParsedConfig) {
    if let Err(e) = do_execute_assistant(tx, rx, config) {
        error!("Error while executing assistant: {e:?}");
    }
}

fn do_execute_assistant(
    tx: Sender<worker::Event>,
    rx: Receiver<Task>,
    config: ParsedConfig,
) -> SResult<()> {
    while let Ok(task) = rx.recv() {
        match task {
            Task::Assist(chat_request) => {
                TOKIO_RUNTIME.spawn(assist(tx.clone(), chat_request, config.clone()));
            }
            Task::Cancel => {
                // Handle the cancel task
            }
        }
    }
    Ok(())
}

async fn assist(tx: Sender<worker::Event>, chat_request: ChatRequest, config: ParsedConfig) {
    if let Err(e) = do_assist(tx, chat_request, config).await {
        error!("Error while executing assistant: {e:?}");
    }
}

async fn do_assist(
    tx: Sender<worker::Event>,
    chat_request: ChatRequest,
    config: ParsedConfig,
) -> SResult<()> {
    let client = Client::builder()
        .with_service_target_resolver(config.model.service_target_resolver)
        .build();

    let resp = client
        .exec_chat(&config.model.name, chat_request, None)
        .await
        .context(GenaiSnafu)?;

    if let Some(message_content) = resp.content {
        tx.send(worker::Event::ChatResponse(message_content))
            .whatever_context("Error sending chat response")?;
    } else {
        error!("No message content from assistant: {:?}", resp);
    }

    Ok(())
}
