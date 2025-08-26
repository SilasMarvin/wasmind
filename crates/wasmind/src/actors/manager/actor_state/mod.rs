use std::sync::Arc;
use tokio::sync::broadcast;
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use super::ActorId;
use crate::{actors::MessageEnvelope, context::WasmindContext, scope::Scope};

pub mod agent;
pub mod command;
pub mod host_info;
pub mod http;
pub mod logger;
pub mod messaging;

pub struct ActorState {
    pub actor_id: ActorId,
    pub ctx: WasiCtx,
    pub table: ResourceTable,
    pub tx: broadcast::Sender<MessageEnvelope>,
    pub scope: Scope,
    pub context: Arc<WasmindContext>,
    pub current_message_id: Option<String>,
}

impl WasiView for ActorState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

impl ActorState {
    pub fn new(
        actor_id: ActorId,
        scope: Scope,
        tx: broadcast::Sender<MessageEnvelope>,
        context: Arc<WasmindContext>,
    ) -> Self {
        let mut builder = WasiCtxBuilder::new();

        builder
            .preopened_dir(
                "/",
                "/",
                wasmtime_wasi::DirPerms::all(),
                wasmtime_wasi::FilePerms::all(),
            )
            .unwrap();

        ActorState {
            actor_id,
            tx,
            scope,
            context,
            ctx: builder.build(),
            table: ResourceTable::new(),
            current_message_id: None,
        }
    }
}
