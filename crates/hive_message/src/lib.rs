use serde::{Serialize, de::DeserializeOwned};

pub trait HiveMessage: Serialize + DeserializeOwned + Send + Sync + 'static {
    fn type_name() -> &'static str;
}
