// use std::{any::Any, collections::HashMap};
//
// use hive_actor_bindings::MessageEnvelope;
// use serde::{Deserialize, de::DeserializeOwned};
//
// type DeserializerFn = Box<dyn Fn(&str) -> Result<Box<dyn Any>, serde_json::Error>>;
//
// #[derive(Default)]
// pub struct MessageParser {
//     registry: HashMap<String, DeserializerFn>,
// }
//
// impl MessageParser {
//     pub fn register_message_type<T: DeserializeOwned + 'static>(&mut self, message_type: &str) {
//         let deserializer = Box::new(move |payload: &str| {
//             let message: T = serde_json::from_str(payload)?;
//             Ok(Box::new(message) as Box<dyn Any>)
//         });
//
//         self.registry.insert(message_type.to_string(), deserializer);
//     }
//
//     pub fn parse_as<T: DeserializeOwned + 'static>(&self, message: &MessageEnvelope) -> Option<T> {
//         self.registry
//             .get(&message.message_type)
//             .map(|deserializer| {
//                 String::from_utf8(message.payload.clone())
//                     .ok()
//                     .map(|json_string| {
//                         deserializer(&json_string)
//                             .ok()
//                             .map(|re| re.downcast::<T>().ok().map(|x| *x))
//                     })
//             })
//             .flatten()
//             .flatten()
//             .flatten()
//     }
//
//     pub fn parse_unregistered_as<T: DeserializeOwned + 'static>(
//         message_type: &str,
//         message: &MessageEnvelope,
//     ) -> Option<T> {
//         if message_type != message.message_type {
//             None
//         } else {
//             String::from_utf8(message.payload.clone())
//                 .ok()
//                 .map(|json_string| serde_json::from_str(&json_string).ok())
//                 .flatten()
//         }
//     }
// }
