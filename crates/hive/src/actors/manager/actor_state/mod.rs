use std::sync::Arc;
use tokio::sync::broadcast;
use wasmtime_wasi::{
    ResourceTable,
    p2::{IoView, WasiCtx, WasiCtxBuilder, WasiView},
};

use super::ActorId;
use crate::{actors::MessageEnvelope, context::HiveContext, scope::Scope};

pub mod agent;
pub mod command;
pub mod http;
pub mod logger;
pub mod messaging;

pub struct ActorState {
    pub actor_id: ActorId,
    pub ctx: WasiCtx,
    pub table: ResourceTable,
    pub tx: broadcast::Sender<MessageEnvelope>,
    pub scope: Scope,
    pub context: Arc<HiveContext>,
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

impl ActorState {
    pub fn new(
        actor_id: ActorId,
        scope: Scope,
        tx: broadcast::Sender<MessageEnvelope>,
        context: Arc<HiveContext>,
    ) -> Self {
        let mut builder = WasiCtxBuilder::new();

        // If you need access to the current working directory:
        // if let Ok(cwd) = std::env::current_dir() {
        builder
            .preopened_dir(
                "/",
                "/",
                wasmtime_wasi::DirPerms::all(),
                wasmtime_wasi::FilePerms::all(),
            )
            .unwrap();
        // }

        ActorState {
            actor_id,
            tx,
            scope,
            context,
            ctx: builder.build(),
            table: ResourceTable::new(),
        }
    }
}
