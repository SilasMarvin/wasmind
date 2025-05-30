use genai::chat::Tool;
use tokio::sync::broadcast;
use tracing::info;

use crate::{
    actors::tools::{command, edit_file, file_reader, planner},
    actors::{Actor, Message},
    config::ParsedConfig,
};

/// Tool Discovery actor that broadcasts available tools
pub struct ToolDiscovery {
    tx: broadcast::Sender<Message>,
    config: ParsedConfig,
}

impl ToolDiscovery {
    fn get_internal_tools(&self) -> Vec<Tool> {
        vec![
            Tool {
                name: command::TOOL_NAME.to_string(),
                description: Some(command::TOOL_DESCRIPTION.to_string()),
                schema: Some(serde_json::from_str(command::TOOL_INPUT_SCHEMA).unwrap()),
            },
            Tool {
                name: edit_file::TOOL_NAME.to_string(),
                description: Some(edit_file::TOOL_DESCRIPTION.to_string()),
                schema: Some(serde_json::from_str(edit_file::TOOL_INPUT_SCHEMA).unwrap()),
            },
            Tool {
                name: file_reader::TOOL_NAME.to_string(),
                description: Some(file_reader::TOOL_DESCRIPTION.to_string()),
                schema: Some(serde_json::from_str(file_reader::TOOL_INPUT_SCHEMA).unwrap()),
            },
            Tool {
                name: planner::TOOL_NAME.to_string(),
                description: Some(planner::TOOL_DESCRIPTION.to_string()),
                schema: Some(serde_json::from_str(planner::TOOL_INPUT_SCHEMA).unwrap()),
            },
        ]
    }
}

#[async_trait::async_trait]
impl Actor for ToolDiscovery {
    fn new(config: ParsedConfig, tx: broadcast::Sender<Message>) -> Self {
        Self { tx, config }
    }

    fn get_rx(&self) -> broadcast::Receiver<Message> {
        self.tx.subscribe()
    }

    async fn on_start(&mut self) {
        info!("Tool Discovery actor starting - broadcasting available tools");

        // Get internal tools
        let mut tools = self.get_internal_tools();

        // TODO: Get MCP tools when MCP actor is implemented
        // For now, just broadcast internal tools

        info!("Broadcasting {} tools", tools.len());
        let _ = self.tx.send(Message::ToolsAvailable(tools));
    }

    async fn handle_message(&mut self, _message: Message) {
        info!("RECIEVE IN TOOL DISCOVERY: {:?}", _message);
        // Tool discovery doesn't need to handle any messages currently
    }
}

