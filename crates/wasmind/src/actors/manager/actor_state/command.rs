use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;
use wasmtime::component::Resource;

use crate::actors::manager::wasmind::actor::command;

use super::ActorState;

pub struct CommandResource {
    pub inner: Arc<Mutex<CommandResourceInner>>,
}

pub struct CommandResourceInner {
    pub command: Command,
    pub timeout_seconds: Option<u32>,
    pub max_output_bytes: Option<u32>,
}

impl command::Host for ActorState {}

impl command::HostCmd for ActorState {
    async fn new(&mut self, command: String) -> wasmtime::component::Resource<CommandResource> {
        let command_resource = CommandResource {
            inner: Arc::new(Mutex::new(CommandResourceInner {
                command: Command::new(command),
                timeout_seconds: None,
                max_output_bytes: Some(100_000), // Default 100KB
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

    async fn max_output_bytes(
        &mut self,
        self_: Resource<CommandResource>,
        bytes: u32,
    ) -> Resource<CommandResource> {
        let cmd = self.table.get(&self_).unwrap();
        let mut inner = cmd.inner.lock().unwrap();
        inner.max_output_bytes = Some(bytes);
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

        let (mut new_command, timeout_seconds, max_output_bytes) = {
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

            (
                new_command,
                inner.timeout_seconds,
                inner.max_output_bytes.unwrap_or(100_000),
            )
        };

        let mut child = match new_command.spawn() {
            Ok(child) => child,
            Err(e) => {
                return Ok(command::CommandOutput {
                    stdout: vec![],
                    stderr: vec![],
                    status: command::ExitStatus::FailedToStart(e.to_string()),
                    stdout_truncated: false,
                    stderr_truncated: false,
                });
            }
        };

        // Take ownership of stdout and stderr streams
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        // Read streams with size limits
        let read_limited = async {
            let mut stdout_data = Vec::new();
            let mut stderr_data = Vec::new();
            let mut stdout_truncated = false;
            let mut stderr_truncated = false;

            // Read stdout with limit
            if let Some(stdout) = stdout {
                let mut reader = BufReader::new(stdout);
                let mut buffer = vec![0u8; 8192]; // 8KB chunks

                while stdout_data.len() < max_output_bytes as usize {
                    match reader.read(&mut buffer).await {
                        Ok(0) => break, // EOF
                        Ok(n) => {
                            let remaining =
                                (max_output_bytes as usize).saturating_sub(stdout_data.len());
                            if n <= remaining {
                                stdout_data.extend_from_slice(&buffer[..n]);
                            } else {
                                stdout_data.extend_from_slice(&buffer[..remaining]);
                                stdout_truncated = true;
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }

            // Read stderr with limit
            if let Some(stderr) = stderr {
                let mut reader = BufReader::new(stderr);
                let mut buffer = vec![0u8; 8192]; // 8KB chunks

                while stderr_data.len() < max_output_bytes as usize {
                    match reader.read(&mut buffer).await {
                        Ok(0) => break, // EOF
                        Ok(n) => {
                            let remaining =
                                (max_output_bytes as usize).saturating_sub(stderr_data.len());
                            if n <= remaining {
                                stderr_data.extend_from_slice(&buffer[..n]);
                            } else {
                                stderr_data.extend_from_slice(&buffer[..remaining]);
                                stderr_truncated = true;
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }

            // Kill the process if we hit the limit
            if stdout_truncated || stderr_truncated {
                let _ = child.kill().await;
            }

            // Wait for the child to exit
            let status = match child.wait().await {
                Ok(status) => status,
                Err(e) => {
                    return Err(format!("Failed to wait for process: {e}"));
                }
            };

            Ok((
                stdout_data,
                stderr_data,
                stdout_truncated,
                stderr_truncated,
                status,
            ))
        };

        let result = if let Some(timeout_seconds) = timeout_seconds {
            let duration = Duration::from_secs(timeout_seconds as u64);
            match timeout(duration, read_limited).await {
                Ok(Ok((stdout_data, stderr_data, stdout_truncated, stderr_truncated, status))) => (
                    stdout_data,
                    stderr_data,
                    stdout_truncated,
                    stderr_truncated,
                    status,
                ),
                Ok(Err(e)) => {
                    return Ok(command::CommandOutput {
                        stdout: vec![],
                        stderr: vec![],
                        status: command::ExitStatus::FailedToStart(e),
                        stdout_truncated: false,
                        stderr_truncated: false,
                    });
                }
                Err(_) => {
                    let _ = child.kill().await;
                    return Ok(command::CommandOutput {
                        stdout: vec![],
                        stderr: vec![],
                        status: command::ExitStatus::TimeoutExpired,
                        stdout_truncated: false,
                        stderr_truncated: false,
                    });
                }
            }
        } else {
            match read_limited.await {
                Ok((stdout_data, stderr_data, stdout_truncated, stderr_truncated, status)) => (
                    stdout_data,
                    stderr_data,
                    stdout_truncated,
                    stderr_truncated,
                    status,
                ),
                Err(e) => {
                    return Ok(command::CommandOutput {
                        stdout: vec![],
                        stderr: vec![],
                        status: command::ExitStatus::FailedToStart(e),
                        stdout_truncated: false,
                        stderr_truncated: false,
                    });
                }
            }
        };

        let (stdout_data, stderr_data, stdout_truncated, stderr_truncated, status) = result;

        let exit_status = if let Some(code) = status.code() {
            command::ExitStatus::Exited(code as u8)
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if let Some(signal) = status.signal() {
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
            stdout: stdout_data,
            stderr: stderr_data,
            status: exit_status,
            stdout_truncated,
            stderr_truncated,
        })
    }

    async fn drop(&mut self, self_: Resource<CommandResource>) -> wasmtime::Result<()> {
        self.table.delete(self_)?;
        Ok(())
    }
}
