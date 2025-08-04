use hive_actor_utils::{
    common_messages::{
        actors::Exit,
        tools::{ExecuteTool, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, UIDisplayInfo},
    },
    tools,
};

#[allow(warnings)]
mod bindings;

#[derive(tools::macros::Tool)]
#[tool(
    name = "report_normal",
    description = "Report that the analyzed agent is healthy and making normal progress",
    schema = r#"{
        "type": "object", 
        "properties": {},
        "required": []
    }"#
)]
struct ReportNormalTool {}

impl tools::Tool for ReportNormalTool {
    fn new(_scope: String, _config: String) -> Self {
        Self {}
    }

    fn handle_call(&mut self, tool_call: ExecuteTool) {
        // Send success result
        let result = ToolCallResult {
            content: "Agent reported as healthy and making normal progress".to_string(),
            ui_display_info: UIDisplayInfo {
                collapsed: "Agent healthy".to_string(),
                expanded: Some("Agent is healthy and making normal progress".to_string()),
            },
        };

        let update = ToolCallStatusUpdate {
            id: tool_call.tool_call.id,
            status: ToolCallStatus::Done { result: Ok(result) },
        };
        let _ = Self::broadcast_common_message(update);

        let _ = Self::broadcast_common_message(Exit);
    }
}

