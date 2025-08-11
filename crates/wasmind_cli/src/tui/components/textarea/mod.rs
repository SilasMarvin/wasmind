use wasmind::actors::MessageEnvelope;
use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
};
#[cfg(feature = "clipboard")]
use tui_realm_textarea::TEXTAREA_CMD_PASTE;
use tui_realm_textarea::{
    INACTIVE_BORDERS, TEXTAREA_CMD_CLEAR, TEXTAREA_CMD_MOVE_WORD_BACK,
    TEXTAREA_CMD_MOVE_WORD_FORWARD, TEXTAREA_CMD_NEWLINE, TEXTAREA_CMD_REDO, TEXTAREA_CMD_UNDO,
    TEXTAREA_CURSOR_LINE_STYLE, TITLE_STYLE, TextArea,
};
use tuirealm::{
    AttrValue, Attribute, Component, Event, MockComponent,
    command::{Cmd, Direction, Position},
    event::{Key, KeyEvent, KeyModifiers},
    props::{BorderType, Borders},
};

use crate::{config::ParsedTuiConfig, tui::model::TuiMessage, utils::key_event_to_string};

use super::chat::ChatUserAction;

pub mod tui_realm_textarea;

#[derive(MockComponent)]
pub struct LLMTextAreaComponent {
    pub component: TextArea<'static>,
    config: ParsedTuiConfig,
}

impl LLMTextAreaComponent {
    pub fn new(config: ParsedTuiConfig) -> Self {
        let (binding, _) = config.chat.key_bindings.iter().find(|(_, action)| **action == ChatUserAction::Assist).expect("No binding to chat action: Assist - this should be impossible? File a bug please thank you!");

        let mut textarea = TextArea::new(vec![])
            .title(
                format!("[ Prompt | ({}) to submit ]", key_event_to_string(binding)),
                Alignment::Left,
            )
            .borders(Borders::default().modifiers(BorderType::Thick))
            .cursor_style(Style::new().bg(Color::Red).fg(Color::Red));
        textarea.attr(Attribute::Focus, AttrValue::Flag(true));
        textarea.attr(
            Attribute::Custom(INACTIVE_BORDERS),
            AttrValue::Borders(Borders::default()),
        );
        textarea.attr(
            Attribute::Custom(TITLE_STYLE),
            AttrValue::Style(Style::new().add_modifier(Modifier::BOLD)),
        );
        textarea.attr(
            Attribute::Custom(TEXTAREA_CURSOR_LINE_STYLE),
            AttrValue::Style(Style::new()),
        );

        Self {
            component: textarea,
            config,
        }
    }

    pub fn get_height(&self, _area: Rect) -> usize {
        // Add two for the borders
        (self.component.state().unwrap_vec().len() + 2).min(20)
    }
}

impl Component<TuiMessage, MessageEnvelope> for LLMTextAreaComponent {
    fn on(&mut self, ev: Event<MessageEnvelope>) -> Option<TuiMessage> {
        match ev {
            Event::Keyboard(key_event) => {
                if let Some(action) = self.config.chat.key_bindings.get(&key_event) {
                    match action {
                        ChatUserAction::Assist => {
                            let content = self
                                .component
                                .state()
                                .unwrap_vec()
                                .into_iter()
                                .map(|line| line.unwrap_string())
                                .collect::<Vec<String>>()
                                .join("\n");
                            self.perform(Cmd::Custom(TEXTAREA_CMD_CLEAR));
                            return Some(TuiMessage::SubmittedUserTypedLLMMessage(content));
                        }
                    }
                }

                match key_event {
                    KeyEvent {
                        code: Key::Backspace,
                        ..
                    }
                    | KeyEvent {
                        code: Key::Char('h'),
                        modifiers: KeyModifiers::CONTROL,
                    } => {
                        self.perform(Cmd::Delete);
                        Some(TuiMessage::Redraw)
                    }
                    KeyEvent {
                        code: Key::Delete, ..
                    } => {
                        self.perform(Cmd::Cancel);
                        Some(TuiMessage::Redraw)
                    }
                    KeyEvent {
                        code: Key::PageDown,
                        ..
                    }
                    | KeyEvent {
                        code: Key::Down,
                        modifiers: KeyModifiers::SHIFT,
                    } => {
                        self.perform(Cmd::Scroll(Direction::Down));
                        Some(TuiMessage::Redraw)
                    }
                    KeyEvent {
                        code: Key::PageUp, ..
                    }
                    | KeyEvent {
                        code: Key::Up,
                        modifiers: KeyModifiers::SHIFT,
                    } => {
                        self.perform(Cmd::Scroll(Direction::Up));
                        Some(TuiMessage::Redraw)
                    }
                    KeyEvent {
                        code: Key::Down, ..
                    } => {
                        self.perform(Cmd::Move(Direction::Down));
                        Some(TuiMessage::Redraw)
                    }
                    KeyEvent {
                        code: Key::Left,
                        modifiers: KeyModifiers::SHIFT,
                    } => {
                        self.perform(Cmd::Custom(TEXTAREA_CMD_MOVE_WORD_BACK));
                        Some(TuiMessage::Redraw)
                    }
                    KeyEvent {
                        code: Key::Left, ..
                    } => {
                        self.perform(Cmd::Move(Direction::Left));
                        Some(TuiMessage::Redraw)
                    }
                    KeyEvent {
                        code: Key::Right,
                        modifiers: KeyModifiers::SHIFT,
                    } => {
                        self.perform(Cmd::Custom(TEXTAREA_CMD_MOVE_WORD_FORWARD));
                        Some(TuiMessage::Redraw)
                    }
                    KeyEvent {
                        code: Key::Right, ..
                    } => {
                        self.perform(Cmd::Move(Direction::Right));
                        Some(TuiMessage::Redraw)
                    }
                    KeyEvent { code: Key::Up, .. } => {
                        self.perform(Cmd::Move(Direction::Up));
                        Some(TuiMessage::Redraw)
                    }
                    KeyEvent { code: Key::End, .. }
                    | KeyEvent {
                        code: Key::Char('e'),
                        modifiers: KeyModifiers::CONTROL,
                    } => {
                        self.perform(Cmd::GoTo(Position::End));
                        Some(TuiMessage::Redraw)
                    }
                    KeyEvent {
                        code: Key::Enter, ..
                    }
                    | KeyEvent {
                        code: Key::Char('m'),
                        modifiers: KeyModifiers::CONTROL,
                    } => {
                        self.perform(Cmd::Custom(TEXTAREA_CMD_NEWLINE));
                        Some(TuiMessage::Redraw)
                    }
                    KeyEvent {
                        code: Key::Home, ..
                    }
                    | KeyEvent {
                        code: Key::Char('a'),
                        modifiers: KeyModifiers::CONTROL,
                    } => {
                        self.perform(Cmd::GoTo(Position::Begin));
                        Some(TuiMessage::Redraw)
                    }
                    #[cfg(feature = "clipboard")]
                    KeyEvent {
                        code: Key::Char('v'),
                        modifiers: KeyModifiers::CONTROL,
                    } => {
                        self.perform(Cmd::Custom(TEXTAREA_CMD_PASTE));
                        Some(TuiMessage::Redraw)
                    }
                    KeyEvent {
                        code: Key::Char('z'),
                        modifiers: KeyModifiers::CONTROL,
                    } => {
                        self.perform(Cmd::Custom(TEXTAREA_CMD_UNDO));
                        Some(TuiMessage::Redraw)
                    }
                    KeyEvent {
                        code: Key::Char('y'),
                        modifiers: KeyModifiers::CONTROL,
                    } => {
                        self.perform(Cmd::Custom(TEXTAREA_CMD_REDO));
                        Some(TuiMessage::Redraw)
                    }
                    KeyEvent { code: Key::Tab, .. } => {
                        self.perform(Cmd::Type('\t'));
                        Some(TuiMessage::Redraw)
                    }
                    KeyEvent {
                        code: Key::Char(ch),
                        ..
                    } => {
                        self.perform(Cmd::Type(ch));
                        Some(TuiMessage::Redraw)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
}
