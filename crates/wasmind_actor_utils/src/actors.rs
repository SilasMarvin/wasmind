#[cfg(feature = "macros")]
pub mod macros {
    pub mod __private {
        pub use serde;
        pub use serde_json;
    }

    pub use wasmind_actor_utils_macros::Actor;
    pub use wasmind_actor_utils_macros::generate_actor_trait;
}
