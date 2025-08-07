use std::path::Path;
use tuirealm::event::{Key, KeyEvent, KeyModifiers};

pub fn count_cached_actors(cache_dir: &Path) -> Result<usize, std::io::Error> {
    if !cache_dir.exists() {
        return Ok(0);
    }

    let count = std::fs::read_dir(cache_dir)?.count();
    Ok(count)
}

pub fn remove_actors_cache(cache_dir: &Path) -> Result<(), std::io::Error> {
    if cache_dir.exists() {
        std::fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

pub fn parse_key_combination(input: &str) -> Option<KeyEvent> {
    let parts: Vec<&str> = input.split('-').collect();
    let mut modifiers = KeyModifiers::empty();

    // Handle the last part as the key, everything before it as modifiers
    let (modifier_parts, key_part) = parts.split_at(parts.len() - 1);

    // Parse modifiers
    for modifier in modifier_parts {
        match modifier.to_lowercase().as_str() {
            "ctrl" => modifiers.insert(KeyModifiers::CONTROL),
            "alt" | "cmd" => modifiers.insert(KeyModifiers::ALT),
            "shift" => modifiers.insert(KeyModifiers::SHIFT),
            _ => return None,
        }
    }

    // Parse the actual key (case insensitive)
    let key_str = key_part[0].to_lowercase();
    let code = match key_str.as_str() {
        // Single character
        s if s.len() == 1 => Key::Char(s.chars().next().unwrap()),
        // Special keys
        "esc" | "escape" => Key::Esc,
        "enter" | "return" => Key::Enter,
        "tab" => Key::Tab,
        "backtab" => Key::BackTab,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "insert" | "ins" => Key::Insert,
        "down" => Key::Down,
        "up" => Key::Up,
        "left" => Key::Left,
        "right" => Key::Right,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "pgup" => Key::PageUp,
        "pagedown" | "pgdn" => Key::PageDown,
        "space" => Key::Char(' '),
        _ => return None,
    };

    Some(KeyEvent::new(code, modifiers))
}

pub fn key_event_to_string(event: &KeyEvent) -> String {
    let mut parts = Vec::new();

    // Add modifiers in consistent order: ctrl-shift-alt
    if event.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl");
    }
    if event.modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("shift");
    }
    if event.modifiers.contains(KeyModifiers::ALT) {
        parts.push("alt");
    }

    // Add the key itself
    let key_str = match event.code {
        Key::Char(' ') => "space".to_string(),
        Key::Char(c) => c.to_lowercase().to_string(),
        Key::Esc => "esc".to_string(),
        Key::Enter => "enter".to_string(),
        Key::Tab => "tab".to_string(),
        Key::BackTab => "backtab".to_string(),
        Key::Backspace => "backspace".to_string(),
        Key::Delete => "delete".to_string(),
        Key::Insert => "insert".to_string(),
        Key::Down => "down".to_string(),
        Key::Up => "up".to_string(),
        Key::Left => "left".to_string(),
        Key::Right => "right".to_string(),
        Key::Home => "home".to_string(),
        Key::End => "end".to_string(),
        Key::PageUp => "pageup".to_string(),
        Key::PageDown => "pagedown".to_string(),
        _ => return String::new(), // Unsupported key
    };

    parts.push(&key_str);
    parts.join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_keys() {
        // Single key
        let event = parse_key_combination("a").unwrap();
        assert_eq!(event.code, Key::Char('a'));
        assert_eq!(event.modifiers, KeyModifiers::NONE);

        // Special key
        let event = parse_key_combination("esc").unwrap();
        assert_eq!(event.code, Key::Esc);
        assert_eq!(event.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn test_parse_with_modifiers() {
        // Ctrl+A
        let event = parse_key_combination("ctrl-a").unwrap();
        assert_eq!(event.code, Key::Char('a'));
        assert_eq!(event.modifiers, KeyModifiers::CONTROL);

        // Ctrl+Shift+Tab
        let event = parse_key_combination("ctrl-shift-tab").unwrap();
        assert_eq!(event.code, Key::Tab);
        assert_eq!(event.modifiers, KeyModifiers::CONTROL | KeyModifiers::SHIFT);

        // Cmd+Enter (cmd maps to alt)
        let event = parse_key_combination("cmd-enter").unwrap();
        assert_eq!(event.code, Key::Enter);
        assert_eq!(event.modifiers, KeyModifiers::ALT);
    }

    #[test]
    fn test_case_insensitive() {
        let event1 = parse_key_combination("CTRL-A").unwrap();
        let event2 = parse_key_combination("ctrl-a").unwrap();
        assert_eq!(event1.code, event2.code);
        assert_eq!(event1.modifiers, event2.modifiers);
    }

    #[test]
    fn test_key_event_to_string() {
        // Simple key
        let event = KeyEvent::new(Key::Char('a'), KeyModifiers::NONE);
        assert_eq!(key_event_to_string(&event), "a");

        // With modifiers
        let event = KeyEvent::new(Key::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(key_event_to_string(&event), "ctrl-a");

        // Multiple modifiers (consistent order)
        let event = KeyEvent::new(Key::Tab, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        assert_eq!(key_event_to_string(&event), "ctrl-shift-tab");

        // Special keys
        let event = KeyEvent::new(Key::Char(' '), KeyModifiers::ALT);
        assert_eq!(key_event_to_string(&event), "alt-space");
    }

    #[test]
    fn test_roundtrip() {
        let test_cases = vec![
            "a",
            "ctrl-a",
            "ctrl-shift-tab",
            "alt-enter",
            "ctrl-shift-alt-space",
            "esc",
            "delete",
        ];

        for input in test_cases {
            let event = parse_key_combination(input).unwrap();
            let output = key_event_to_string(&event);
            let parsed_again = parse_key_combination(&output).unwrap();
            assert_eq!(event.code, parsed_again.code);
            assert_eq!(event.modifiers, parsed_again.modifiers);
        }
    }
}
