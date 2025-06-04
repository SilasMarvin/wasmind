use crossterm::event::KeyCode;

use crate::{
    actors::Action,
    config::{KeyBinding, ParsedKeyConfig},
};

#[cfg(feature = "gui")]
pub fn rdev_key_to_crossterm(key: rdev::Key) -> Option<KeyCode> {
    use rdev::Key;
    
    match key {
        // Letters
        Key::KeyA => Some(KeyCode::Char('a')),
        Key::KeyB => Some(KeyCode::Char('b')),
        Key::KeyC => Some(KeyCode::Char('c')),
        Key::KeyD => Some(KeyCode::Char('d')),
        Key::KeyE => Some(KeyCode::Char('e')),
        Key::KeyF => Some(KeyCode::Char('f')),
        Key::KeyG => Some(KeyCode::Char('g')),
        Key::KeyH => Some(KeyCode::Char('h')),
        Key::KeyI => Some(KeyCode::Char('i')),
        Key::KeyJ => Some(KeyCode::Char('j')),
        Key::KeyK => Some(KeyCode::Char('k')),
        Key::KeyL => Some(KeyCode::Char('l')),
        Key::KeyM => Some(KeyCode::Char('m')),
        Key::KeyN => Some(KeyCode::Char('n')),
        Key::KeyO => Some(KeyCode::Char('o')),
        Key::KeyP => Some(KeyCode::Char('p')),
        Key::KeyQ => Some(KeyCode::Char('q')),
        Key::KeyR => Some(KeyCode::Char('r')),
        Key::KeyS => Some(KeyCode::Char('s')),
        Key::KeyT => Some(KeyCode::Char('t')),
        Key::KeyU => Some(KeyCode::Char('u')),
        Key::KeyV => Some(KeyCode::Char('v')),
        Key::KeyW => Some(KeyCode::Char('w')),
        Key::KeyX => Some(KeyCode::Char('x')),
        Key::KeyY => Some(KeyCode::Char('y')),
        Key::KeyZ => Some(KeyCode::Char('z')),

        // Numbers
        Key::Num0 => Some(KeyCode::Char('0')),
        Key::Num1 => Some(KeyCode::Char('1')),
        Key::Num2 => Some(KeyCode::Char('2')),
        Key::Num3 => Some(KeyCode::Char('3')),
        Key::Num4 => Some(KeyCode::Char('4')),
        Key::Num5 => Some(KeyCode::Char('5')),
        Key::Num6 => Some(KeyCode::Char('6')),
        Key::Num7 => Some(KeyCode::Char('7')),
        Key::Num8 => Some(KeyCode::Char('8')),
        Key::Num9 => Some(KeyCode::Char('9')),

        // Special keys
        Key::Return => Some(KeyCode::Enter),
        Key::Escape => Some(KeyCode::Esc),
        Key::Space => Some(KeyCode::Char(' ')),
        Key::Tab => Some(KeyCode::Tab),
        Key::Backspace => Some(KeyCode::Backspace),
        Key::Delete => Some(KeyCode::Delete),
        Key::Insert => Some(KeyCode::Insert),
        Key::Home => Some(KeyCode::Home),
        Key::End => Some(KeyCode::End),
        Key::PageUp => Some(KeyCode::PageUp),
        Key::PageDown => Some(KeyCode::PageDown),
        Key::UpArrow => Some(KeyCode::Up),
        Key::DownArrow => Some(KeyCode::Down),
        Key::LeftArrow => Some(KeyCode::Left),
        Key::RightArrow => Some(KeyCode::Right),

        // Function keys
        Key::F1 => Some(KeyCode::F(1)),
        Key::F2 => Some(KeyCode::F(2)),
        Key::F3 => Some(KeyCode::F(3)),
        Key::F4 => Some(KeyCode::F(4)),
        Key::F5 => Some(KeyCode::F(5)),
        Key::F6 => Some(KeyCode::F(6)),
        Key::F7 => Some(KeyCode::F(7)),
        Key::F8 => Some(KeyCode::F(8)),
        Key::F9 => Some(KeyCode::F(9)),
        Key::F10 => Some(KeyCode::F(10)),
        Key::F11 => Some(KeyCode::F(11)),
        Key::F12 => Some(KeyCode::F(12)),

        // Ignore modifiers for now (they're handled differently in crossterm)
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct LiveBinding {
    binding: KeyBinding,
    action: Action,
    matching_event_index: usize,
}

impl LiveBinding {
    fn check_match(&mut self, event: KeyCode) -> bool {
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

    pub fn handle_event(&mut self, event: KeyCode) -> Vec<Action> {
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
