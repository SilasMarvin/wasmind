use hive::actor::{command, http, messaging};
use hive_actor_utils_common_messages::{Message, actors};
use tokio::sync::broadcast;
use wasmtime::{
    Config, Engine, Store,
    component::{Component, HasSelf, Linker, ResourceAny, bindgen},
};

use crate::scope::Scope;

pub mod actor_state;
pub use actor_state::{ActorState, command::CommandResource};

use super::MessageEnvelope;

pub type ActorId = String;

bindgen!({
    world: "actor-world", async: true,
    with: {
        "hive:actor/command/cmd": CommandResource,
        "hive:actor/http/request": actor_state::http::HttpRequestResource,
    },
    path: "../hive_actor_bindings/wit/world.wit"
});

pub struct Manager {
    actor_id: ActorId,
    actor_world: ActorWorld,
    actor_resource: ResourceAny,
    store: Store<ActorState>,
    tx: broadcast::Sender<MessageEnvelope>,
    scope: Scope,
}

impl Manager {
    pub async fn new(
        actor_id: ActorId,
        wasm: &[u8],
        scope: Scope,
        tx: broadcast::Sender<MessageEnvelope>,
    ) -> Self {
        let mut config = Config::new();
        config.async_support(true);
        let engine = Engine::new(&config).unwrap();

        let component = Component::from_binary(&engine, wasm).unwrap();

        let mut store = Store::new(
            &engine,
            ActorState::new(actor_id.clone(), scope.clone(), tx.clone()),
        );

        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();
        messaging::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state).unwrap();
        command::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state).unwrap();
        http::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state).unwrap();

        let actor_world = ActorWorld::instantiate_async(&mut store, &component, &linker)
            .await
            .unwrap();

        let actor_resource = actor_world
            .hive_actor_actor()
            .actor()
            .call_constructor(&mut store, &scope.to_string())
            .await
            .unwrap();

        Manager {
            actor_id,
            store,
            actor_resource,
            actor_world,
            tx,
            scope,
        }
    }

    pub fn run(mut self) {
        let mut rx = self.tx.subscribe();
        tracing::info_span!("actor_lifecycle", actor_id = self.actor_id).in_scope(move || {
            tokio::spawn(async move {
                let _ = self.tx.send(MessageEnvelope {
                    message_type: actors::ActorReady::MESSAGE_TYPE.to_string(),
                    from_actor_id: self.actor_id.to_string(),
                    from_scope: self.scope.to_string(),
                    payload: serde_json::to_string(&actors::ActorReady)
                        .unwrap()
                        .into_bytes(),
                });

                loop {
                    match rx.recv().await {
                        Ok(msg) => {
                            // This message doesn't hold anything so we just need to check if the message_type matches
                            if msg.message_type == actors::Exit::MESSAGE_TYPE {
                                break;
                            } else {
                                if let Err(e) = self
                                    .actor_world
                                    .hive_actor_actor()
                                    .actor()
                                    .call_handle_message(&mut self.store, self.actor_resource, &msg)
                                    .await
                                {
                                    tracing::error!("Calling handle_message: {e:?}");
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::error!(
                                "Receiver lagged by {n} messages! This was unexpected.",
                            );
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            tracing::error!("Channel closed");
                        }
                    }
                }

                if let Err(e) = self
                    .actor_world
                    .hive_actor_actor()
                    .actor()
                    .call_destructor(&mut self.store, self.actor_resource)
                    .await
                {
                    tracing::error!("Calling destructor: {e:?}");
                }
            });
        });
    }
}
