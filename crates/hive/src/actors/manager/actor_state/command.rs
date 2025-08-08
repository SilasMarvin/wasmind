use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;
use wasmtime::component::Resource;

use crate::actors::manager::hive::actor::command;

use super::ActorState;

pub struct CommandResource {
    pub inner: Arc<Mutex<CommandResourceInner>>,
}

pub struct CommandResourceInner {
    pub command: Command,
    pub timeout_seconds: Option<u32>,
}

impl command::Host for ActorState {}

impl command::HostCmd for ActorState {
    async fn new(&mut self, command: String) -> wasmtime::component::Resource<CommandResource> {
        let command_resource = CommandResource {
            inner: Arc::new(Mutex::new(CommandResourceInner {
                command: Command::new(command),
                timeout_seconds: None,
            })),
        };

        self.table.push(command_resource).unwrap()
    }

    async fn env(
        &mut self,
        self_: Resource<CommandResource>,
        key: String,
        value: String,
    ) -> wasmtime::component::Resource<CommandResource> {
        let cmd = self.table.get(&self_).unwrap();
        let mut inner = cmd.inner.lock().unwrap();
        inner.command.env(key, value);
        drop(inner);

        let new_resource = CommandResource {
            inner: Arc::clone(&cmd.inner),
        };
        self.table.push(new_resource).unwrap()
    }

    async fn env_clear(&mut self, self_: Resource<CommandResource>) -> Resource<CommandResource> {
        let cmd = self.table.get(&self_).unwrap();
        let mut inner = cmd.inner.lock().unwrap();
        inner.command.env_clear();
        drop(inner);

        let new_resource = CommandResource {
            inner: Arc::clone(&cmd.inner),
        };
        self.table.push(new_resource).unwrap()
    }

    async fn args(
        &mut self,
        self_: Resource<CommandResource>,
        args: Vec<String>,
    ) -> Resource<CommandResource> {
        let cmd = self.table.get(&self_).unwrap();
        let mut inner = cmd.inner.lock().unwrap();
        inner.command.args(args);
        drop(inner);

        let new_resource = CommandResource {
            inner: Arc::clone(&cmd.inner),
        };
        self.table.push(new_resource).unwrap()
    }

    async fn current_dir(
        &mut self,
        self_: Resource<CommandResource>,
        dir: String,
    ) -> Resource<CommandResource> {
        let cmd = self.table.get(&self_).unwrap();
        let mut inner = cmd.inner.lock().unwrap();
        inner.command.current_dir(dir);
        drop(inner);

        let new_resource = CommandResource {
            inner: Arc::clone(&cmd.inner),
        };
        self.table.push(new_resource).unwrap()
    }

    async fn timeout(
        &mut self,
        self_: Resource<CommandResource>,
        seconds: u32,
    ) -> Resource<CommandResource> {
        let cmd = self.table.get(&self_).unwrap();
        let mut inner = cmd.inner.lock().unwrap();
        inner.timeout_seconds = Some(seconds);
        drop(inner);

        let new_resource = CommandResource {
            inner: Arc::clone(&cmd.inner),
        };
        self.table.push(new_resource).unwrap()
    }

    async fn run(
        &mut self,
        self_: Resource<CommandResource>,
    ) -> std::result::Result<command::CommandOutput, String> {
        let cmd_resource = self.table.get(&self_).map_err(|e| e.to_string())?;

        let (mut new_command, timeout_seconds) = {
            let inner = cmd_resource.inner.lock().unwrap();

            let program = inner.command.as_std().get_program();
            let mut new_command = Command::new(program);

            let args: Vec<_> = inner.command.as_std().get_args().collect();
            for arg in args {
                new_command.arg(arg);
            }

            let envs: Vec<_> = inner
                .command
                .as_std()
                .get_envs()
                .filter_map(|(k, v)| match (k.to_str(), v) {
                    (Some(k), Some(v)) => v.to_str().map(|v| (k.to_string(), v.to_string())),
                    _ => None,
                })
                .collect();
            for (k, v) in envs {
                new_command.env(k, v);
            }

            if let Some(dir) = inner.command.as_std().get_current_dir() {
                new_command.current_dir(dir);
            }

            new_command.stdout(std::process::Stdio::piped());
            new_command.stderr(std::process::Stdio::piped());
            new_command.stdin(std::process::Stdio::null());

            (new_command, inner.timeout_seconds)
        };

        let child = match new_command.spawn() {
            Ok(child) => child,
            Err(e) => {
                return Ok(command::CommandOutput {
                    stdout: vec![],
                    stderr: vec![],
                    status: command::ExitStatus::FailedToStart(e.to_string()),
                });
            }
        };

        let output = if let Some(timeout_seconds) = timeout_seconds {
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
