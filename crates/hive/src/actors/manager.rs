use exports::hive::actor::actor_interface::{Actor, MessageEnvelope};
use hive::actor::runtime_interface::{self, Host};

use wasmtime::{
    Config, Engine, Result, Store,
    component::{Component, HasSelf, Linker, ResourceAny, bindgen},
};
use wasmtime_wasi::{
    ResourceTable,
    p2::{IoView, WasiCtx, WasiCtxBuilder, WasiView},
};

use crate::scope::Scope;

pub type ActorId = String;

bindgen!({world: "actor-world", async: true, path: "../hive_actor_bindings/wit/world.wit"});

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
impl Host for ActorState {
    async fn broadcast(&mut self, payload: Vec<u8>) {
        let string = String::from_utf8(payload).unwrap();
        println!("GOT PAYLOAD: {}", string);
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
        runtime_interface::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state).unwrap();

        let actor_world = ActorWorld::instantiate_async(&mut store, &component, &linker)
            .await
            .unwrap();

        let actor_resource = actor_world
            .hive_actor_actor_interface()
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
            .hive_actor_actor_interface()
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
