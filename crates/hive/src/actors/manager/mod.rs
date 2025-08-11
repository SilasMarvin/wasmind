use hive::actor::{agent, command, host_info, http, logger, messaging};
use hive_actor_utils::{common_messages::actors, messages::Message};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{Instrument, Level};
use wasmtime::{
    Config, Engine, Store,
    component::{Component, HasSelf, Linker, ResourceAny, bindgen},
};

use crate::{context::HiveContext, scope::Scope};

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

impl PartialEq for MessageEnvelope {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl PartialOrd for MessageEnvelope {
    fn partial_cmp(&self, _other: &Self) -> Option<std::cmp::Ordering> {
        None
    }
}

impl Eq for MessageEnvelope {}

pub struct Manager {
    actor_id: ActorId,
    actor_world: ActorWorld,
    actor_resource: ResourceAny,
    store: Store<ActorState>,
    tx: broadcast::Sender<MessageEnvelope>,
    rx: broadcast::Receiver<MessageEnvelope>,
    scope: Scope,
}

impl Manager {
    pub async fn new(
        actor_id: ActorId,
        wasm: &[u8],
        scope: Scope,
        tx: broadcast::Sender<MessageEnvelope>,
        rx: broadcast::Receiver<MessageEnvelope>,
        context: Arc<HiveContext>,
        actor_config: Option<toml::Table>,
    ) -> Self {
        let mut config = Config::new();
        config.async_support(true);
        let engine = Engine::new(&config).unwrap();

        let component = match Component::from_binary(&engine, wasm) {
            Ok(component) => component,
            Err(e) => panic!("Error creating wasm component for: {actor_id} - {e:?}"),
        };

        let mut store = Store::new(
            &engine,
            ActorState::new(actor_id.clone(), scope.clone(), tx.clone(), context),
        );

        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();
        messaging::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state).unwrap();
        command::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state).unwrap();
        http::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state).unwrap();
        logger::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state).unwrap();
        agent::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state).unwrap();
        host_info::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state).unwrap();

        let actor_world = ActorWorld::instantiate_async(&mut store, &component, &linker)
            .await
            .unwrap();

        let config_str = actor_config
            .map(|c| toml::to_string(&c).unwrap_or_default())
            .unwrap_or_default();

        let actor_resource = actor_world
            .hive_actor_actor()
            .actor()
            .call_constructor(&mut store, &scope.to_string(), &config_str)
            .await
            .unwrap();

        Manager {
            actor_id,
            store,
            actor_resource,
            actor_world,
            tx,
            rx,
            scope,
        }
    }

    pub fn run(mut self) {
        tracing::info_span!("actor_lifecycle", actor_id = self.actor_id).in_scope(move || {
            tokio::spawn(async move {
                let _ = self.tx.send(MessageEnvelope {
                    id: crate::utils::generate_root_correlation_id(),
                    message_type: actors::ActorReady::MESSAGE_TYPE.to_string(),
                    from_actor_id: self.actor_id.to_string(),
                    from_scope: self.scope.to_string(),
                    payload: serde_json::to_string(&actors::ActorReady)
                        .unwrap()
                        .into_bytes(),
                });

                loop {
                    match self.rx.recv().await {
                        Ok(msg) => {
                            if msg.from_scope == self.scope
                                && msg.message_type == actors::Exit::MESSAGE_TYPE
                            {
                                break;
                            } else {
                                let span = tracing::span!(
                                    Level::ERROR,
                                    "hive_actor_manager",
                                    correlation_id = msg.id
                                );

                                self.store.data_mut().current_message_id = Some(msg.id.clone());

                                if let Err(e) = self
                                    .actor_world
                                    .hive_actor_actor()
                                    .actor()
                                    .call_handle_message(&mut self.store, self.actor_resource, &msg)
                                    .instrument(span)
                                    .await
                                {
                                    tracing::error!("Calling handle_message: {e:?}");
                                }

                                self.store.data_mut().current_message_id = None;
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
