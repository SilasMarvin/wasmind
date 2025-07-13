use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Style},
};
use tui_realm_textarea::{
    TEXTAREA_CMD_CLEAR, TEXTAREA_CMD_MOVE_WORD_BACK, TEXTAREA_CMD_MOVE_WORD_FORWARD,
    TEXTAREA_CMD_NEWLINE, TEXTAREA_CMD_REDO, TEXTAREA_CMD_UNDO, TextArea,
};
use tuirealm::{
    AttrValue, Attribute, Component, Event, MockComponent,
    command::{Cmd, Direction, Position},
    event::{Key, KeyEvent, KeyModifiers},
    props::Borders,
};

use crate::{
    actors::{ActorMessage, tui::model::TuiMessage},
    config::ParsedTuiConfig,
};

mod tui_realm_textarea;

#[derive(MockComponent)]
pub struct LLMTextAreaComponent {
    pub component: TextArea<'static>,
    config: ParsedTuiConfig,
}

impl LLMTextAreaComponent {
    pub fn new(config: ParsedTuiConfig) -> Self {
        let mut textarea = TextArea::new(vec![])
            .title("[ Input ]", Alignment::Left)
            .borders(Borders::default())
            .cursor_style(Style::new().bg(Color::Red).fg(Color::Red));
        textarea.attr(Attribute::Focus, AttrValue::Flag(true));

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

impl Component<TuiMessage, ActorMessage> for LLMTextAreaComponent {
    fn on(&mut self, ev: Event<ActorMessage>) -> Option<TuiMessage> {
        match ev {
            Event::Keyboard(key_event) => {
                if let Some(action) = self.config.chat.key_bindings.get(&key_event) {
                    match action {
                        super::chat::ChatUserAction::Assist => {
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
                        Some(Msg::None)
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
