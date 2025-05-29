use std::{collections::HashMap, sync::OnceLock};

use crossbeam::channel::{Receiver, Sender};
use genai::{
    chat::{Tool, ToolCall},
};
use rmcp::{
    RoleClient,
    model::CallToolRequestParam,
    service::{RunningService, ServiceExt},
    transport::{self},
};
use snafu::{Location, ResultExt, Snafu};
use tokio::process::Command;
use tracing::{error, info};

use crate::{
    SResult, TOKIO_RUNTIME,
    config::ParsedConfig,
    worker::{self, Event},
};

pub static MCP_FUNCS_TO_SERVER: OnceLock<HashMap<String, String>> = OnceLock::new();
pub static MCP_SERVERS: OnceLock<HashMap<String, RunningService<RoleClient, ()>>> = OnceLock::new();

/// Errors while executing MCP
#[derive(Debug, Snafu)]
enum MCPError {
    #[snafu(display("Error starting MCP server for {server} With: {command} {args:?}"))]
    StartMCP {
        #[snafu(implicit)]
        location: Location,
        server: String,
        command: String,
        args: Vec<String>,
        #[snafu(source)]
        source: std::io::Error,
    },

    Service {
        #[snafu(implicit)]
        location: Location,
        server: String,
        #[snafu(source)]
        source: rmcp::ServiceError,
    },

    SendEvent {
        #[snafu(implicit)]
        location: Location,
        #[snafu(source)]
        source: crossbeam::channel::SendError<Event>,
    },

    #[snafu(display("MCP Function not found: {func_name}"))]
    _FuncNotFound {
        #[snafu(implicit)]
        location: Location,
        func_name: String,
    },

    #[snafu(display("MCP Server not found: {server}"))]
    _ServerNotFound {
        #[snafu(implicit)]
        location: Location,
        server: String,
    },
}

type MResult<T> = Result<T, MCPError>;

/// Tasks the assistant can receive from the worker
#[derive(Debug, Clone)]
pub enum Task {
    UseTools(Vec<ToolCall>),
    Cancel,
}

pub fn execute_mcp(tx: Sender<worker::Event>, rx: Receiver<Task>, _config: ParsedConfig) {
    if let Err(e) = do_execute_mcp(tx, rx, _config) {
        error!("Error while executing assistant: {e:?}");
    }
}

fn do_execute_mcp(
    tx: Sender<worker::Event>,
    rx: Receiver<Task>,
    config: ParsedConfig,
) -> SResult<()> {
    TOKIO_RUNTIME.spawn(start_servers(tx.clone(), config.clone()));
    while let Ok(task) = rx.recv() {
        match task {
            Task::UseTools(tool_calls) => {
                TOKIO_RUNTIME.spawn(execute_tools(tx.clone(), tool_calls, config.clone()));
            }
            Task::Cancel => {
                // Handle the cancel task
            }
        }
    }
    Ok(())
}

async fn start_servers(tx: Sender<worker::Event>, config: ParsedConfig) {
    if let Err(e) = do_start_servers(tx.clone(), config.clone()).await {
        error!("Error while starting MCP servers: {e:?}");
    }
}

async fn do_start_servers(tx: Sender<worker::Event>, config: ParsedConfig) -> MResult<()> {
    let mut client_list = HashMap::new();
    let mut mcp_funcs_to_server = HashMap::new();
    let mut tools = vec![];
    for (name, config) in config.mcp_servers {
        info!(server = %name, command = %config.command, args = ?config.args, "Starting MCP server");
        
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        
        let service = ()
            .serve(
                transport::TokioChildProcess::new(&mut cmd)
                    .with_context(|_| StartMCPSnafu {
                        server: name.clone(),
                        command: config.command.clone(),
                        args: config.args.clone(),
                    })?,
            )
            .await
            .with_context(|_| StartMCPSnafu {
                server: name.clone(),
                command: config.command.clone(),
                args: config.args.clone(),
            })?;

        // NOTE: list_tools also returns a cursor for fetching more results
        // For now assume there is only one page of tools
        let mcp_tools = service
            .list_tools(Default::default())
            .await
            .with_context(|_| ServiceSnafu {
                server: name.clone(),
            })?
            .tools;

        for tool in &mcp_tools {
            mcp_funcs_to_server.insert(tool.name.to_string(), name.clone());
        }

        info!(server = %name, tool_count = %mcp_tools.len(), "MCP server started successfully");
        
        tools.extend(mcp_tools.into_iter().map(|tool| Tool {
            name: tool.name.to_string(),
            description: tool.description.map(|x| x.to_string()),
            schema: Some(serde_json::to_value(tool.input_schema).unwrap()),
        }));

        client_list.insert(name, service);
    }
    tx.send(Event::MCPToolsInit(tools))
        .context(SendEventSnafu)?;
    MCP_SERVERS.set(client_list).unwrap();
    MCP_FUNCS_TO_SERVER.set(mcp_funcs_to_server).unwrap();
    Ok(())
}

async fn execute_tools(tx: Sender<worker::Event>, tool_calls: Vec<ToolCall>, config: ParsedConfig) {
    if let Err(e) = do_execute_tools(tx, tool_calls, config).await {
        error!("Error while executing MCP tool call: {e:?}");
    }
}

async fn do_execute_tools(
    tx: Sender<worker::Event>,
    tool_calls: Vec<ToolCall>,
    _config: ParsedConfig,
) -> MResult<()> {
    for tool_call in tool_calls {
        let call_id = tool_call.call_id.clone();
        
        let server_name = match MCP_FUNCS_TO_SERVER
            .get()
            .unwrap()
            .get(&tool_call.fn_name)
        {
            Some(name) => name,
            None => {
                // Send failure stage update
                tx.send(worker::Event::MCPStageUpdate {
                    call_id,
                    stage: crate::tools::MCPExecutionStage::Failed {
                        error: format!("Function not found: {}", tool_call.fn_name),
                    },
                })
                .context(SendEventSnafu)?;
                continue;
            }
        };

        let server = match MCP_SERVERS
            .get()
            .unwrap()
            .get(server_name)
        {
            Some(server) => server,
            None => {
                // Send failure stage update
                tx.send(worker::Event::MCPStageUpdate {
                    call_id,
                    stage: crate::tools::MCPExecutionStage::Failed {
                        error: format!("Server not found: {}", server_name),
                    },
                })
                .context(SendEventSnafu)?;
                continue;
            }
        };

        let tool_name = tool_call.fn_name.clone();
        
        info!(
            server = %server_name,
            tool = %tool_name,
            "Executing MCP tool"
        );
        
        match server
            .call_tool(CallToolRequestParam {
                name: tool_call.fn_name.into(),
                arguments: Some(serde_json::from_value(tool_call.fn_arguments).unwrap()),
            })
            .await
        {
            Ok(tool_response) => {
                info!(
                    server = %server_name,
                    tool = %tool_name,
                    "MCP tool execution completed"
                );

                if tool_response.is_error.is_some_and(|x| x) {
                    error!("Error while executing MCP tool call");
                }

                let content = tool_response
                    .content
                    .into_iter()
                    .map(|content| match content.raw {
                        rmcp::model::RawContent::Text(raw_text_content) => raw_text_content.text,
                        rmcp::model::RawContent::Image(_raw_image_content) => todo!(),
                        rmcp::model::RawContent::Resource(_raw_embedded_resource) => todo!(),
                        rmcp::model::RawContent::Audio(_annotated) => todo!(),
                    })
                    .collect::<Vec<String>>()
                    .join("\n\n\n\n");

                // Send completion stage update
                tx.send(worker::Event::MCPStageUpdate {
                    call_id,
                    stage: crate::tools::MCPExecutionStage::Completed {
                        result: content,
                    },
                })
                .context(SendEventSnafu)?;
            }
            Err(e) => {
                // Send failure stage update
                tx.send(worker::Event::MCPStageUpdate {
                    call_id,
                    stage: crate::tools::MCPExecutionStage::Failed {
                        error: format!("Failed to execute MCP tool: {}", e),
                    },
                })
                .context(SendEventSnafu)?;
            }
        }
    }

    Ok(())
}
