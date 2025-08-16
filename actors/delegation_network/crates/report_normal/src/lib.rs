use wasmind_actor_utils::{
    common_messages::{
        assistant::{AgentTaskResponse, RequestStatusUpdate, Status},
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
struct ReportNormalTool {
    scope: String,
}

impl tools::Tool for ReportNormalTool {
    fn new(scope: String, _config: String) -> Self {
        Self { scope: scope }
    }

    fn handle_call(&mut self, tool_call: ExecuteTool) {
        let _ = Self::broadcast_common_message(RequestStatusUpdate {
            agent: self.scope.clone(),
            status: Status::Done {
                result: Ok(AgentTaskResponse {
                    summary: "Approved edit_file request".to_string(),
                    success: true,
                }),
            },
            originating_request_id: Some(tool_call.originating_request_id.clone()),
        });

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
            originating_request_id: tool_call.originating_request_id,
            status: ToolCallStatus::Done { result: Ok(result) },
        };
        let _ = Self::broadcast_common_message(update);
    }
}
