use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::atomic::AtomicU64;
#[cfg(feature = "test-utils")]
use std::sync::atomic::Ordering;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Scope(Uuid);

#[cfg(feature = "test-utils")]
static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

impl Scope {
    pub fn new() -> Self {
        #[cfg(feature = "test-utils")]
        {
            // In tests, use deterministic UUIDs based on atomic counter
            let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
            // Create a deterministic UUID from the counter
            let uuid_bytes = [
                (counter >> 56) as u8,
                (counter >> 48) as u8,
                (counter >> 40) as u8,
                (counter >> 32) as u8,
                (counter >> 24) as u8,
                (counter >> 16) as u8,
                (counter >> 8) as u8,
                counter as u8,
                0, 0, 0, 0, 0, 0, 0, 0
            ];
            Self(Uuid::from_bytes(uuid_bytes))
        }
        #[cfg(not(feature = "test-utils"))]
        {
            Self(Uuid::new_v4())
        }
    }
    
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for Scope {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl From<Scope> for Uuid {
    fn from(scope: Scope) -> Self {
        scope.0
    }
}