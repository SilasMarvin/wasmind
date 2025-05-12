use crossbeam::channel::Sender;
use rdev::{Event as RdevEvent, EventType, Key, listen};
use tracing::error;

use crate::{
    config::KeyBindings,
    key_bindings::KeyBindingManager,
    worker::Event,
};

pub struct InputManager {
    key_binding_manager: KeyBindingManager,
    event_tx: Sender<Event>,
}

impl InputManager {
    pub fn new(key_bindings: &KeyBindings, event_tx: Sender<Event>) -> Self {
        Self {
            key_binding_manager: KeyBindingManager::from(key_bindings),
            event_tx,
        }
    }

    pub fn start(&mut self) {
        let event_tx = self.event_tx.clone();
        let mut key_binding_manager = self.key_binding_manager.clone();

        let callback = move |event: RdevEvent| match event.event_type {
            EventType::KeyPress(key) => {
                let actions = key_binding_manager.handle_event(key);
                for action in actions {
                    if let Err(e) = event_tx.send(Event::Action(action)) {
                        error!("Error sending action to worker: {e:?}");
                    }
                }
            }
            EventType::KeyRelease(_) => {
                key_binding_manager.clear();
            }
            _ => (),
        };

        // This will block and has to be in the main thread
        if let Err(error) = listen(callback) {
            error!("Error listening for global key events: {:?}", error)
        }
    }

    pub fn send_input(&self, input: String) {
        if !input.is_empty() {
            if let Err(e) = self.event_tx.send(Event::UserTUIInput(input)) {
                error!("Error sending user input to worker: {e:?}");
            }
        }
    }
}