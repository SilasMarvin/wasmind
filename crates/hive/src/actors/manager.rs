use std::collections::HashMap;
use std::time::Duration;

use exports::hive::actor::actor::MessageEnvelope;
use hive::actor::{
    command::{self, Cmd},
    messaging,
};
use tokio::process::Command;
use tokio::time::timeout;
use wasmtime::{
    Config, Engine, Result, Store,
    component::{Component, HasSelf, Linker, Resource, ResourceAny, ResourceType, bindgen},
};
use wasmtime_wasi::{
    ResourceTable,
    p2::{IoView, WasiCtx, WasiCtxBuilder, WasiView},
};

use crate::scope::Scope;

pub type ActorId = String;

pub struct CommandResource {
    command: Command,
    timeout_seconds: Option<u32>,
}

bindgen!({
    world: "actor-world", async: true,
    with: {
        "hive:actor/command/cmd": CommandResource
    },
    path: "../hive_actor_bindings/wit/world.wit"
});

struct ActorState {
    ctx: WasiCtx,
    table: ResourceTable,
}

impl IoView for ActorState {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}
impl WasiView for ActorState {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.ctx
    }
}

// Implementation of the host interface defined in the wit file.
impl messaging::Host for ActorState {
    async fn broadcast(&mut self, payload: Vec<u8>) {
        let string = String::from_utf8(payload).unwrap();
        println!("GOT PAYLOAD: {}", string);
    }
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

pub struct Manager {
    actor_id: ActorId,
    actor_world: ActorWorld,
    actor_resource: ResourceAny,
    store: Store<ActorState>,
}

impl Manager {
    pub async fn new(actor_id: ActorId, scope: Scope, wasm: &[u8]) -> Self {
        let mut config = Config::new();
        config.async_support(true);
        let engine = Engine::new(&config).unwrap();

        let component = Component::from_binary(&engine, wasm).unwrap();

        let mut builder = WasiCtxBuilder::new();

        // Configure builder here

        let mut store = Store::new(
            &engine,
            ActorState {
                ctx: builder.build(),
                table: ResourceTable::new(),
            },
        );

        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();
        messaging::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state).unwrap();
        command::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state).unwrap();

        let actor_world = ActorWorld::instantiate_async(&mut store, &component, &linker)
            .await
            .unwrap();

        let actor_resource = actor_world
            .hive_actor_actor()
            .actor()
            .call_constructor(&mut store, &scope.to_string())
            .await
            .unwrap();

        let message_envelope = MessageEnvelope {
            from_actor_id: "FILLER".to_string(),
            from_scope: "FILLER".to_string(),
            payload: "TEST".as_bytes().to_vec(),
        };
        actor_world
            .hive_actor_actor()
            .actor()
            .call_handle_message(&mut store, actor_resource, &message_envelope)
            .await
            .unwrap();

        Manager {
            actor_id,
            store,
            actor_resource,
            actor_world,
        }
    }
}
