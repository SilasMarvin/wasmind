pub mod actor_manager;
pub mod agent;
// TODO: Re-enable after HTTP interface migration
// pub mod litellm_manager;

// Re-exports for convenience
pub use actor_manager::exports::hive::actor::actor::MessageEnvelope;
