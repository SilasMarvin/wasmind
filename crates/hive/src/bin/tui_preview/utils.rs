use hive::{
    actors::{
        ActorMessage, AgentMessage, AgentStatus, AgentType, Message,
        tools::{
            Tool,
            command::{CommandParams, CommandTool},
        },
    },
    llm_client::{AssistantChatMessage, ChatMessage, ToolCall},
    scope::Scope,
};

pub fn create_spawn_agent_message(
    scope: &Scope,
    agent_type: AgentType,
    role: &str,
    task: &str,
) -> (ActorMessage, Scope) {
    let new_scope = Scope::new();

    (
        ActorMessage {
            scope: scope.clone(),
            message: Message::Agent(AgentMessage {
                agent_id: new_scope.clone(),
                message: hive::actors::AgentMessageType::AgentSpawned {
                    agent_type,
                    role: role.to_string(),
                    task_description: task.to_string(),
                    tool_call_id: "FILLER".to_string(),
                },
            }),
        },
        new_scope,
    )
}

pub fn create_agent_status_update_message(scope: &Scope, status: AgentStatus) -> ActorMessage {
    ActorMessage {
        scope: scope.clone(),
        message: Message::Agent(AgentMessage {
            agent_id: scope.clone(),
            message: hive::actors::AgentMessageType::InterAgentMessage(
                hive::actors::InterAgentMessage::StatusUpdate { status },
            ),
        }),
    }
}

pub fn create_command_tool_call(
    scope: &Scope,
    command: &str,
    args: &[&str],
) -> (ActorMessage, ChatMessage) {
    let call_id = "call_command_123";

    let command_params = CommandParams {
        command: command.to_string(),
        args: Some(args.iter().map(ToString::to_string).collect()),
        directory: None,
        timeout: None,
    };

    let tool_call = ToolCall {
        id: call_id.to_string(),
        tool_type: "function".to_string(),
        function: hive::llm_client::Function {
            name: CommandTool::TOOL_NAME.to_string(),
            arguments: serde_json::to_string(&command_params).unwrap(),
        },
        index: Some(0),
    };

    let chat_message = ChatMessage::Assistant(AssistantChatMessage::new_with_tools(vec![
        tool_call.clone(),
    ]));

    let tool_call_message = ActorMessage {
        scope: scope.clone(),
        message: Message::AssistantToolCall(tool_call),
    };

    (tool_call_message, chat_message)
}
