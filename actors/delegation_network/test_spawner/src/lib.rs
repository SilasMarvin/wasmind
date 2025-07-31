use hive_actor_bindings::{
    exports::hive::actor::actor::{Guest, GuestActor, MessageEnvelope},
    hive::actor::{agent, logger},
};
use hive_actor_utils::{logger::log_info, send_message};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct SpawnTest {
    actors_to_spawn: Vec<String>,
}

hive_actor_utils_common_messages::define_message!(SpawnTest);

struct TestSpawner {
    scope: String,
}

impl GuestActor for TestSpawner {
    fn new(scope: String, _config: String) -> Self {
        log_info!("TestSpawner actor created in scope: {}", scope);
        Self { scope }
    }

    fn handle_message(&self, message: MessageEnvelope) {
        log_info!("TestSpawner received message type: {}", message.message_type);
        
        if message.message_type == SpawnTest::MESSAGE_TYPE {
            if let Ok(spawn_test) = serde_json::from_slice::<SpawnTest>(&message.payload) {
                log_info!("Spawning new agent with actors: {:?}", spawn_test.actors_to_spawn);
                
                match agent::spawn_agent(&spawn_test.actors_to_spawn) {
                    Ok(new_scope) => {
                        log_info!("Successfully spawned new agent in scope: {}", new_scope);
                        
                        // Send a message to confirm spawn
                        #[derive(Serialize)]
                        struct SpawnResult {
                            spawned_scope: String,
                        }
                        
                        send_message!(
                            "test_spawner.SpawnResult",
                            SpawnResult { spawned_scope: new_scope }
                        );
                    }
                    Err(e) => {
                        log_info!("Failed to spawn agent: {}", e);
                    }
                }
            }
        }
    }
    
    fn drop(&self) {
        log_info!("TestSpawner actor destroyed");
    }
}

hive_actor_bindings::export!(TestSpawner with_types_in hive_actor_bindings);