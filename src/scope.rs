use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Scope(Uuid);

impl Scope {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
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