use wasmtime_wasi::{
    ResourceTable,
    p2::{IoView, WasiCtx, WasiCtxBuilder, WasiView},
};

pub mod command;
pub mod messaging;

pub struct ActorState {
    pub ctx: WasiCtx,
    pub table: ResourceTable,
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
    pub fn new() -> Self {
        let mut builder = WasiCtxBuilder::new();
        ActorState {
            ctx: builder.build(),
            table: ResourceTable::new(),
        }
    }
}

