pub mod command;
pub mod planner;

use genai::chat::Tool;
use serde_json::Value;

pub trait InternalTool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn input_schema(&self) -> Value;
}

pub fn get_all_tools() -> Vec<Box<dyn InternalTool>> {
    vec![
        Box::new(command::Command::new()),
        Box::new(planner::Planner::new()),
    ]
}

pub fn tools_to_mcp(tools: Vec<Box<dyn InternalTool>>) -> Vec<Tool> {
    tools
        .into_iter()
        .map(|tool| Tool {
            name: tool.name().to_string(),
            description: Some(tool.description().to_string()),
            schema: Some(tool.input_schema()),
        })
        .collect()
}