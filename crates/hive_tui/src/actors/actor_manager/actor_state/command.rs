use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;
use wasmtime::component::Resource;

use crate::actors::actor_manager::hive::actor::command;

use super::ActorState;

pub struct CommandResource {
    pub command: Command,
    pub timeout_seconds: Option<u32>,
}

impl command::Host for ActorState {}

impl command::HostCmd for ActorState {
    async fn new(&mut self, command: String) -> wasmtime::component::Resource<CommandResource> {
        let command_resource = CommandResource {
            command: Command::new(command),
            timeout_seconds: None,
        };
        let resource = self.table.push(command_resource).unwrap();
        resource
    }

    async fn env(
        &mut self,
        self_: Resource<CommandResource>,
        key: String,
        value: String,
    ) -> wasmtime::component::Resource<CommandResource> {
        let cmd = self.table.get_mut(&self_).unwrap();
        cmd.command.env(key, value);
        self_
    }

    async fn env_clear(&mut self, self_: Resource<CommandResource>) -> Resource<CommandResource> {
        let cmd = self.table.get_mut(&self_).unwrap();
        cmd.command.env_clear();
        self_
    }

    async fn args(
        &mut self,
        self_: Resource<CommandResource>,
        args: Vec<String>,
    ) -> Resource<CommandResource> {
        let cmd = self.table.get_mut(&self_).unwrap();
        cmd.command.args(args);
        self_
    }

    async fn current_dir(
        &mut self,
        self_: Resource<CommandResource>,
        dir: String,
    ) -> Resource<CommandResource> {
        let cmd = self.table.get_mut(&self_).unwrap();
        cmd.command.current_dir(dir);
        self_
    }

    async fn timeout(
        &mut self,
        self_: Resource<CommandResource>,
        seconds: u32,
    ) -> Resource<CommandResource> {
        let cmd = self.table.get_mut(&self_).unwrap();
        cmd.timeout_seconds = Some(seconds);
        self_
    }

    async fn run(
        &mut self,
        self_: Resource<CommandResource>,
    ) -> std::result::Result<command::CommandOutput, String> {
        let cmd_resource = self.table.get_mut(&self_).map_err(|e| e.to_string())?;

        let child = match cmd_resource.command.spawn() {
            Ok(child) => child,
            Err(e) => {
                return Ok(command::CommandOutput {
                    stdout: vec![],
                    stderr: vec![],
                    status: command::ExitStatus::FailedToStart(e.to_string()),
                });
            }
        };

        let output = if let Some(timeout_seconds) = cmd_resource.timeout_seconds {
            let duration = Duration::from_secs(timeout_seconds as u64);
            match timeout(duration, child.wait_with_output()).await {
                Ok(Ok(output)) => output,
                Ok(Err(e)) => {
                    return Ok(command::CommandOutput {
                        stdout: vec![],
                        stderr: vec![],
                        status: command::ExitStatus::FailedToStart(e.to_string()),
                    });
                }
                Err(_) => {
                    return Ok(command::CommandOutput {
                        stdout: vec![],
                        stderr: vec![],
                        status: command::ExitStatus::TimeoutExpired,
                    });
                }
            }
        } else {
            match child.wait_with_output().await {
                Ok(output) => output,
                Err(e) => {
                    return Ok(command::CommandOutput {
                        stdout: vec![],
                        stderr: vec![],
                        status: command::ExitStatus::FailedToStart(e.to_string()),
                    });
                }
            }
        };

        let status = if let Some(code) = output.status.code() {
            command::ExitStatus::Exited(code as u8)
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if let Some(signal) = output.status.signal() {
                    command::ExitStatus::Signaled(signal as u8)
                } else {
                    command::ExitStatus::FailedToStart("Unknown error".to_string())
                }
            }
            #[cfg(not(unix))]
            {
                command::ExitStatus::FailedToStart("Process terminated abnormally".to_string())
            }
        };

        Ok(command::CommandOutput {
            stdout: output.stdout,
            stderr: output.stderr,
            status,
        })
    }

    async fn drop(&mut self, self_: Resource<CommandResource>) -> wasmtime::Result<()> {
        self.table.delete(self_)?;
        Ok(())
    }
}
