use rdev::Key;

use crate::{
    config::{KeyBinding, ParsedKeyConfig},
    worker::Action,
};

#[derive(Debug, Clone)]
struct LiveBinding {
    binding: KeyBinding,
    action: Action,
    matching_event_index: usize,
}

impl LiveBinding {
    fn check_match(&mut self, event: Key) -> bool {
        if self.binding[self.matching_event_index] == event {
            if self.matching_event_index + 1 == self.binding.len() {
                return true;
            }
            self.matching_event_index += 1;
        } else {
            self.matching_event_index = 0;
        }

        false
    }

    fn reset(&mut self) {
        self.matching_event_index = 0;
    }
}

pub struct KeyBindingManager {
    live_key_bindings: Vec<LiveBinding>,
}

impl From<&ParsedKeyConfig> for KeyBindingManager {
    fn from(value: &ParsedKeyConfig) -> Self {
        let bindings: Vec<LiveBinding> = value
            .bindings
            .iter()
            .map(|(key_binding, action)| LiveBinding {
                binding: key_binding.clone(),
                action: action.clone(),
                matching_event_index: 0,
            })
            .collect();
        KeyBindingManager::new(bindings)
    }
}

impl KeyBindingManager {
    fn new(key_bindings: Vec<LiveBinding>) -> Self {
        Self {
            live_key_bindings: key_bindings,
        }
    }

    pub fn clear(&mut self) {
        for binding in self.live_key_bindings.iter_mut() {
            binding.reset();
        }
    }

    pub fn handle_event(&mut self, event: Key) -> Vec<Action> {
        let mut matches = vec![];
        for binding in self.live_key_bindings.iter_mut() {
            if binding.check_match(event) {
                matches.push(binding.action);
            }
        }

        if !matches.is_empty() {
            self.clear();
        }

        matches
    }
}
