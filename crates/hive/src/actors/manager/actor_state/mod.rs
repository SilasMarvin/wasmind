use tokio::sync::broadcast;
use wasmtime_wasi::{
    ResourceTable,
    p2::{IoView, WasiCtx, WasiCtxBuilder, WasiView},
};

use super::ActorId;
use crate::{actors::MessageEnvelope, scope::Scope};

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
    pub fn new(actor_id: ActorId, scope: Scope, tx: broadcast::Sender<MessageEnvelope>) -> Self {
        let mut builder = WasiCtxBuilder::new();
        ActorState {
            actor_id,
            tx,
            scope,
            ctx: builder.build(),
            table: ResourceTable::new(),
        }
    }
}
