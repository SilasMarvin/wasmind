use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{
    actors::Action,
    config::{KeyBinding, ParsedKeyConfig},
};

#[cfg(feature = "gui")]
pub struct RdevToCrosstermConverter {
    modifiers: KeyModifiers,
}

#[cfg(feature = "gui")]
impl RdevToCrosstermConverter {
    pub fn new() -> Self {
        Self {
            modifiers: KeyModifiers::empty(),
        }
    }

    pub fn handle_key_press(&mut self, key: rdev::Key) -> Option<KeyEvent> {
        use rdev::Key;
        
        match key {
            // Handle modifiers by updating state
            Key::ControlLeft | Key::ControlRight => {
                self.modifiers |= KeyModifiers::CONTROL;
                None
            }
            Key::Alt => {
                self.modifiers |= KeyModifiers::ALT;
                None
            }
            Key::ShiftLeft | Key::ShiftRight => {
                self.modifiers |= KeyModifiers::SHIFT;
                None
            }
            // On macOS, Meta keys map to Super (Cmd)
            #[cfg(target_os = "macos")]
            Key::MetaLeft | Key::MetaRight => {
                self.modifiers |= KeyModifiers::SUPER;
                None
            }
            // On other platforms, Meta keys might be used differently
            #[cfg(not(target_os = "macos"))]
            Key::MetaLeft | Key::MetaRight => {
                self.modifiers |= KeyModifiers::SUPER;
                None
            }
            
            // Handle regular keys - create KeyEvent with current modifiers
            _ => {
                let key_code = match key {
                    // Letters
                    Key::KeyA => KeyCode::Char('a'),
                    Key::KeyB => KeyCode::Char('b'),
                    Key::KeyC => KeyCode::Char('c'),
                    Key::KeyD => KeyCode::Char('d'),
                    Key::KeyE => KeyCode::Char('e'),
                    Key::KeyF => KeyCode::Char('f'),
                    Key::KeyG => KeyCode::Char('g'),
                    Key::KeyH => KeyCode::Char('h'),
                    Key::KeyI => KeyCode::Char('i'),
                    Key::KeyJ => KeyCode::Char('j'),
                    Key::KeyK => KeyCode::Char('k'),
                    Key::KeyL => KeyCode::Char('l'),
                    Key::KeyM => KeyCode::Char('m'),
                    Key::KeyN => KeyCode::Char('n'),
                    Key::KeyO => KeyCode::Char('o'),
                    Key::KeyP => KeyCode::Char('p'),
                    Key::KeyQ => KeyCode::Char('q'),
                    Key::KeyR => KeyCode::Char('r'),
                    Key::KeyS => KeyCode::Char('s'),
                    Key::KeyT => KeyCode::Char('t'),
                    Key::KeyU => KeyCode::Char('u'),
                    Key::KeyV => KeyCode::Char('v'),
                    Key::KeyW => KeyCode::Char('w'),
                    Key::KeyX => KeyCode::Char('x'),
                    Key::KeyY => KeyCode::Char('y'),
                    Key::KeyZ => KeyCode::Char('z'),

                    // Numbers
                    Key::Num0 => KeyCode::Char('0'),
                    Key::Num1 => KeyCode::Char('1'),
                    Key::Num2 => KeyCode::Char('2'),
                    Key::Num3 => KeyCode::Char('3'),
                    Key::Num4 => KeyCode::Char('4'),
                    Key::Num5 => KeyCode::Char('5'),
                    Key::Num6 => KeyCode::Char('6'),
                    Key::Num7 => KeyCode::Char('7'),
                    Key::Num8 => KeyCode::Char('8'),
                    Key::Num9 => KeyCode::Char('9'),

                    // Special keys
                    Key::Return => KeyCode::Enter,
                    Key::Escape => KeyCode::Esc,
                    Key::Space => KeyCode::Char(' '),
                    Key::Tab => KeyCode::Tab,
                    Key::Backspace => KeyCode::Backspace,
                    Key::Delete => KeyCode::Delete,
                    Key::Insert => KeyCode::Insert,
                    Key::Home => KeyCode::Home,
                    Key::End => KeyCode::End,
                    Key::PageUp => KeyCode::PageUp,
                    Key::PageDown => KeyCode::PageDown,
                    Key::UpArrow => KeyCode::Up,
                    Key::DownArrow => KeyCode::Down,
                    Key::LeftArrow => KeyCode::Left,
                    Key::RightArrow => KeyCode::Right,

                    // Function keys
                    Key::F1 => KeyCode::F(1),
                    Key::F2 => KeyCode::F(2),
                    Key::F3 => KeyCode::F(3),
                    Key::F4 => KeyCode::F(4),
                    Key::F5 => KeyCode::F(5),
                    Key::F6 => KeyCode::F(6),
                    Key::F7 => KeyCode::F(7),
                    Key::F8 => KeyCode::F(8),
                    Key::F9 => KeyCode::F(9),
                    Key::F10 => KeyCode::F(10),
                    Key::F11 => KeyCode::F(11),
                    Key::F12 => KeyCode::F(12),

                    _ => return None,
                };
                
                Some(KeyEvent::new(key_code, self.modifiers))
            }
        }
    }

    pub fn handle_key_release(&mut self, key: rdev::Key) {
        use rdev::Key;
        
        match key {
            Key::ControlLeft | Key::ControlRight => {
                self.modifiers.remove(KeyModifiers::CONTROL);
            }
            Key::Alt => {
                self.modifiers.remove(KeyModifiers::ALT);
            }
            Key::ShiftLeft | Key::ShiftRight => {
                self.modifiers.remove(KeyModifiers::SHIFT);
            }
            #[cfg(target_os = "macos")]
            Key::MetaLeft | Key::MetaRight => {
                self.modifiers.remove(KeyModifiers::SUPER);
            }
            #[cfg(not(target_os = "macos"))]
            Key::MetaLeft | Key::MetaRight => {
                self.modifiers.remove(KeyModifiers::SUPER);
            }
            _ => {}
        }
    }

    pub fn clear(&mut self) {
        self.modifiers = KeyModifiers::empty();
    }
}

#[derive(Debug, Clone)]
struct LiveBinding {
    binding: KeyBinding,
    action: Action,
    matching_event_index: usize,
}

impl LiveBinding {
    fn check_match(&mut self, event: KeyEvent) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actors::Action;
    use crate::config::ParsedKeyConfig;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::collections::HashMap;

    #[cfg(feature = "gui")]
    #[test]
    fn test_rdev_to_crossterm_basic_keys() {
        let mut converter = RdevToCrosstermConverter::new();
        
        // Test basic letter conversion
        assert_eq!(
            converter.handle_key_press(rdev::Key::KeyA),
            Some(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty()))
        );
        
        // Test number conversion
        assert_eq!(
            converter.handle_key_press(rdev::Key::Num5),
            Some(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::empty()))
        );
        
        // Test special keys
        assert_eq!(
            converter.handle_key_press(rdev::Key::Return),
            Some(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
        );
        
        assert_eq!(
            converter.handle_key_press(rdev::Key::Space),
            Some(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::empty()))
        );
    }

    #[cfg(feature = "gui")]
    #[test]
    fn test_rdev_to_crossterm_modifiers() {
        let mut converter = RdevToCrosstermConverter::new();
        
        // Test that modifiers alone don't generate events
        assert_eq!(converter.handle_key_press(rdev::Key::ControlLeft), None);
        assert_eq!(converter.handle_key_press(rdev::Key::Alt), None);
        assert_eq!(converter.handle_key_press(rdev::Key::ShiftLeft), None);
        
        // Clear first before testing Ctrl+A
        converter.clear();
        converter.handle_key_press(rdev::Key::ControlLeft);
        assert_eq!(
            converter.handle_key_press(rdev::Key::KeyA),
            Some(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL))
        );
        
        // Test Alt+B (clear state first)
        converter.clear();
        converter.handle_key_press(rdev::Key::Alt);
        assert_eq!(
            converter.handle_key_press(rdev::Key::KeyB),
            Some(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT))
        );
    }

    #[test]
    fn test_config_key_parsing_basic() {
        use crate::config::parse_key_combination;
        
        // Test basic key
        let binding = parse_key_combination("a").unwrap();
        assert_eq!(binding.len(), 1);
        assert_eq!(binding[0], KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty()));
        
        // Test special keys
        let binding = parse_key_combination("enter").unwrap();
        assert_eq!(binding[0], KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
        
        let binding = parse_key_combination("escape").unwrap();
        assert_eq!(binding[0], KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
    }

    #[test]
    fn test_config_key_parsing_with_modifiers() {
        use crate::config::parse_key_combination;
        
        // Test Ctrl+A
        let binding = parse_key_combination("ctrl-a").unwrap();
        assert_eq!(binding[0], KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        
        // Test Alt+B
        let binding = parse_key_combination("alt-b").unwrap();
        assert_eq!(binding[0], KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT));
        
        // Test Shift+C
        let binding = parse_key_combination("shift-c").unwrap();
        assert_eq!(binding[0], KeyEvent::new(KeyCode::Char('c'), KeyModifiers::SHIFT));
        
        // Test Meta/Cmd+D
        let binding = parse_key_combination("cmd-d").unwrap();
        assert_eq!(binding[0], KeyEvent::new(KeyCode::Char('d'), KeyModifiers::SUPER));
        
        let binding = parse_key_combination("meta-e").unwrap();
        assert_eq!(binding[0], KeyEvent::new(KeyCode::Char('e'), KeyModifiers::SUPER));
    }

    #[test]
    fn test_key_binding_manager_single_key() {
        let mut bindings = HashMap::new();
        bindings.insert(
            vec![KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty())],
            Action::Assist,
        );
        
        let config = ParsedKeyConfig { bindings };
        let mut manager = KeyBindingManager::from(&config);
        
        // Test matching key
        let actions = manager.handle_event(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty()));
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], Action::Assist);
        
        // Test non-matching key
        let actions = manager.handle_event(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::empty()));
        assert_eq!(actions.len(), 0);
    }

    #[test]
    fn test_key_binding_manager_with_modifiers() {
        let mut bindings = HashMap::new();
        bindings.insert(
            vec![KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)],
            Action::Cancel,
        );
        
        let config = ParsedKeyConfig { bindings };
        let mut manager = KeyBindingManager::from(&config);
        
        // Test Ctrl+S
        let actions = manager.handle_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], Action::Cancel);
        
        // Test just S (should not match)
        let actions = manager.handle_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::empty()));
        assert_eq!(actions.len(), 0);
        
        // Test S with different modifier (should not match)
        let actions = manager.handle_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT));
        assert_eq!(actions.len(), 0);
    }
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

    pub fn handle_event(&mut self, event: KeyEvent) -> Vec<Action> {
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
