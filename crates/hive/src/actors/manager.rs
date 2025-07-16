use wasmtime::{
    Config, Engine, Result, Store,
    component::{Component, HasSelf, Linker, bindgen},
};
use wasmtime_wasi::{
    ResourceTable,
    p2::{IoView, WasiCtx, WasiCtxBuilder, WasiView},
};

pub type ActorId = String;

bindgen!({world: "actor", async: true, path: "../hive_actor_bindings/wit/world.wit"});

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
impl host::Host for ActorState {
    async fn send_message(&mut self, a: u32) {
        println!("GOT MESSAGE: {}", a);
    }
}

pub struct Manager {
    actor_id: ActorId,
    actor: Actor,
    store: Store<ActorState>,
}

impl Manager {
    pub async fn new(actor_id: ActorId, wasm: &[u8]) -> Self {
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
        host::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state).unwrap();

        let actor = Actor::instantiate_async(&mut store, &component, &linker)
            .await
            .unwrap();

        let x = actor.call_add(&mut store, 10, 11).await;
        println!("GOT X: {:?}", x);

        Manager {
            actor_id,
            store,
            actor,
        }
    }
}
