use crate::llm_client::ToolCall;
use rmcp::{
    RoleClient,
    model::CallToolRequestParam,
    service::{RunningService, ServiceExt},
    transport,
};
use snafu::{ResultExt, Snafu};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

use crate::actors::{
    Action, Actor, ActorMessage, Message, ToolCallStatus, ToolCallUpdate,
};
use crate::config::ParsedConfig;
use crate::scope::Scope;

#[derive(Debug, Snafu)]
enum MCPError {
    #[snafu(display("Error starting MCP server for {server} with: {command} {args:?}"))]
    StartMCP {
        server: String,
        command: String,
        args: Vec<String>,
        #[snafu(source)]
        source: std::io::Error,
    },

    #[snafu(display("MCP service error for server {server}"))]
    Service {
        server: String,
        #[snafu(source)]
        source: rmcp::ServiceError,
    },
}

type MResult<T> = Result<T, MCPError>;

/// Running MCP tool call
#[derive(Debug)]
struct RunningMCPCall {
    tool_call_id: String,
    server_name: String,
    tool_name: String,
    handle: JoinHandle<()>,
}

/// MCP actor
pub struct MCP {
    tx: broadcast::Sender<ActorMessage>,
    config: ParsedConfig,
    servers: HashMap<String, Arc<RunningService<RoleClient, ()>>>,
    func_to_server: HashMap<String, String>,
    running_calls: HashMap<String, RunningMCPCall>,
    scope: Scope,
}

impl MCP {
    pub fn new(config: ParsedConfig, tx: broadcast::Sender<ActorMessage>, scope: Scope) -> Self {
        Self {
            config,
            tx,
            servers: HashMap::new(),
            func_to_server: HashMap::new(),
            running_calls: HashMap::new(),
            scope,
        }
    }

    async fn start_servers(&mut self) -> MResult<()> {
        let mut tools: Vec<crate::llm_client::Tool> = Vec::new();

        for (server_name, server_config) in &self.config.mcp_servers {
            info!(
                server = %server_name,
                command = %server_config.command,
                args = ?server_config.args,
                "Starting MCP server"
            );

            let mut cmd = Command::new(&server_config.command);
            cmd.args(&server_config.args);
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());

            let service = ()
                .serve(
                    transport::TokioChildProcess::new(&mut cmd).with_context(|_| {
                        StartMCPSnafu {
                            server: server_name.clone(),
                            command: server_config.command.clone(),
                            args: server_config.args.clone(),
                        }
                    })?,
                )
                .await
                .with_context(|_| StartMCPSnafu {
                    server: server_name.clone(),
                    command: server_config.command.clone(),
                    args: server_config.args.clone(),
                })?;

            // List tools from this server
            let mcp_tools = service
                .list_tools(Default::default())
                .await
                .with_context(|_| ServiceSnafu {
                    server: server_name.clone(),
                })?
                .tools;

            // Map each tool to its server
            for tool in &mcp_tools {
                self.func_to_server
                    .insert(tool.name.to_string(), server_name.clone());
            }

            info!(
                server = %server_name,
                tool_count = %mcp_tools.len(),
                "MCP server started successfully"
            );

            // Convert MCP tools to LLM Tools
            tools.extend(mcp_tools.into_iter().map(|tool| crate::llm_client::Tool {
                tool_type: "function".to_string(),
                function: crate::llm_client::ToolFunction {
                    name: tool.name.to_string(),
                    description: tool.description.map(|x| x.to_string()).unwrap_or_default(),
                    parameters: serde_json::to_value(tool.input_schema).unwrap(),
                },
            }));

            self.servers.insert(server_name.clone(), Arc::new(service));
        }

        // Broadcast available tools
        if !tools.is_empty() {
            self.broadcast(Message::ToolsAvailable(tools));
        }

        Ok(())
    }

    async fn handle_tool_call(&mut self, tool_call: ToolCall) {
        // Check if this tool call is for an MCP function
        let server_name = match self.func_to_server.get(&tool_call.function.name) {
            Some(name) => name.clone(),
            None => {
                // Not an MCP tool, ignore
                return;
            }
        };

        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.id.clone(),
            status: ToolCallStatus::Received,
        }));

        self.execute_mcp_tool(tool_call, &server_name).await;
    }

    async fn execute_mcp_tool(&mut self, tool_call: ToolCall, server_name: &str) {
        let tx = self.tx.clone();
        let scope = self.scope.clone();

        // Get the server - we need to clone the Arc for the async task
        let server = match self.servers.get(server_name) {
            Some(server) => Arc::clone(server),
            None => {
                self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.id.clone(),
                    status: ToolCallStatus::Finished(Err(format!(
                        "Server not found: {}",
                        server_name
                    ))),
                }));
                return;
            }
        };

        let tool_call_id = tool_call.id.clone();
        let tool_name = tool_call.function.name.clone();
        let fn_name = tool_call.function.name.clone();
        let fn_arguments = tool_call.function.arguments.clone();
        let server_name_task = server_name.to_string();

        // Spawn the MCP tool execution in a separate task
        let handle = tokio::spawn(async move {
            debug!(
                server = %server_name_task,
                tool = %tool_name,
                "Executing MCP tool"
            );

            let result = server
                .call_tool(CallToolRequestParam {
                    name: fn_name.into(),
                    arguments: Some(serde_json::from_str(&fn_arguments).unwrap()),
                })
                .await;

            let status = match result {
                Ok(tool_response) => {
                    info!(
                        server = %server_name_task,
                        tool = %tool_name,
                        "MCP tool execution completed"
                    );

                    if tool_response.is_error.is_some_and(|x| x) {
                        ToolCallStatus::Finished(Err("MCP tool reported an error".to_string()))
                    } else {
                        let content = tool_response
                            .content
                            .into_iter()
                            .map(|content| match content.raw {
                                rmcp::model::RawContent::Text(raw_text_content) => {
                                    raw_text_content.text
                                }
                                rmcp::model::RawContent::Image(_) => "[Image content]".to_string(),
                                rmcp::model::RawContent::Resource(_) => {
                                    "[Resource content]".to_string()
                                }
                                rmcp::model::RawContent::Audio(_) => "[Audio content]".to_string(),
                            })
                            .collect::<Vec<String>>()
                            .join("\n\n");

                        ToolCallStatus::Finished(Ok(content))
                    }
                }
                Err(e) => {
                    error!(
                        server = %server_name_task,
                        tool = %tool_name,
                        error = %e,
                        "MCP tool execution failed"
                    );
                    ToolCallStatus::Finished(Err(format!("Failed to execute MCP tool: {}", e)))
                }
            };

            let _ = tx.send(ActorMessage {
                scope,
                message: Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call_id,
                    status,
                }),
            });
        });

        // Store the handle so we can cancel it later if needed
        let running_call = RunningMCPCall {
            tool_call_id: tool_call.id.clone(),
            server_name: server_name.to_string(),
            tool_name: tool_call.function.name.clone(),
            handle,
        };

        self.running_calls.insert(tool_call.id, running_call);
    }

    fn cleanup_completed_calls(&mut self) {
        // Remove completed tasks from the HashMap
        self.running_calls
            .retain(|_, call| !call.handle.is_finished());
    }

    #[allow(dead_code)]
    fn cancel_call(&mut self, tool_call_id: &str) {
        if let Some(call) = self.running_calls.remove(tool_call_id) {
            call.handle.abort();
            info!(
                server = %call.server_name,
                tool = %call.tool_name,
                call_id = %call.tool_call_id,
                "Cancelled MCP tool execution"
            );
        }
    }

    fn cancel_all_calls(&mut self) {
        for (_, call) in self.running_calls.drain() {
            call.handle.abort();
            info!(
                server = %call.server_name,
                tool = %call.tool_name,
                call_id = %call.tool_call_id,
                "Cancelled MCP tool execution"
            );
        }
    }

    async fn shutdown_servers(&mut self) {
        info!("Shutting down MCP servers");

        // Cancel all running calls first
        self.cancel_all_calls();

        // We don't wait for servers to shut down gracefully - just drop them
        // The underlying processes will be terminated when the service is dropped
        self.servers.clear();
        self.func_to_server.clear();
    }
}

#[async_trait::async_trait]
impl Actor for MCP {
    const ACTOR_ID: &'static str = "mcp";

    fn get_rx(&self) -> broadcast::Receiver<ActorMessage> {
        self.tx.subscribe()
    }

    fn get_tx(&self) -> broadcast::Sender<ActorMessage> {
        self.tx.clone()
    }

    fn get_scope(&self) -> &Scope {
        &self.scope
    }

    async fn on_start(&mut self) {
        if let Err(e) = self.start_servers().await {
            error!("Failed to start MCP servers: {}", e);
        }
    }

    async fn on_stop(&mut self) {
        self.shutdown_servers().await;
    }

    async fn handle_message(&mut self, message: ActorMessage) {
        // Cleanup completed calls periodically
        self.cleanup_completed_calls();

        match message.message {
            Message::AssistantToolCall(tool_call) => self.handle_tool_call(tool_call).await,
            Message::Action(Action::Cancel) => {
                // Cancel all running MCP calls
                self.cancel_all_calls();
            }
            _ => (),
        }
    }
}
