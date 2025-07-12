use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State, StateValue,
    command::{Cmd, CmdResult},
    props::{Alignment, Style},
    ratatui::{
        layout::Rect,
        widgets::{Block, Paragraph, Wrap},
    },
};

use crate::{
    actors::{ActorMessage, tui::model::TuiMessage},
    config::ParsedTuiConfig,
};

#[derive(MockComponent)]
pub struct LLMTextAreaComponent {
    pub component: LLMTextArea,
    config: ParsedTuiConfig,
}

impl LLMTextAreaComponent {
    pub fn new(config: ParsedTuiConfig) -> Self {
        Self {
            component: LLMTextArea {
                props: Props::default(),
                state: State::One(StateValue::String("".to_string())),
            },
            config,
        }
    }

    pub fn get_height(&self, area: Rect) -> usize {
        let paragraph = self.component.build_paragraph();
        paragraph.line_count(area.width)
    }
}

pub struct LLMTextArea {
    props: Props,
    state: State,
}

impl LLMTextArea {
    fn build_paragraph(&self) -> Paragraph {
        let text = self.state().unwrap_one().unwrap_string();

        let _focus = self
            .props
            .get_or(Attribute::Focus, AttrValue::Flag(false))
            .unwrap_flag();

        Paragraph::new(text)
            .block(Block::bordered())
            .style(Style::new())
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: true })
    }
}

impl MockComponent for LLMTextArea {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        // Check if visible
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            frame.render_widget(self.build_paragraph(), area);
        }
    }

    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.props.get(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.props.set(attr, value);
    }

    fn state(&self) -> State {
        self.state.clone()
    }

    fn perform(&mut self, _cmd: Cmd) -> CmdResult {
        unreachable!()
    }
}

impl Component<TuiMessage, ActorMessage> for LLMTextAreaComponent {
    fn on(&mut self, ev: Event<ActorMessage>) -> Option<TuiMessage> {
        match ev {
            Event::Keyboard(key_event) => {
                if let Some(action) = self.config.chat.key_bindings.get(&key_event) {
                    match action {
                        super::chat::ChatUserAction::Assist => {
                            let text = self.component.state().unwrap_one().unwrap_string();
                            self.component.state = State::One(StateValue::String("".to_string()));
                            Some(TuiMessage::SubmittedUserTypedLLMMessage(text))
                        }
                    }
                } else {
                    match key_event.code {
                        tuirealm::event::Key::Backspace | tuirealm::event::Key::Delete => {
                            let mut new_text = self.component.state().unwrap_one().unwrap_string();
                            new_text.pop();
                            self.component.state = State::One(StateValue::String(new_text.clone()));
                            Some(TuiMessage::UpdatedUserTypedLLMMessage(new_text))
                        }
                        tuirealm::event::Key::Enter => {
                            let text = self.component.state().unwrap_one().unwrap_string();
                            let new_text = format!("{text}\n");
                            self.component.state = State::One(StateValue::String(new_text.clone()));
                            Some(TuiMessage::UpdatedUserTypedLLMMessage(new_text))
                        }
                        tuirealm::event::Key::Left => todo!(),
                        tuirealm::event::Key::Right => todo!(),
                        tuirealm::event::Key::Up => None,
                        tuirealm::event::Key::Down => None,
                        tuirealm::event::Key::Home => todo!(),
                        tuirealm::event::Key::End => todo!(),
                        tuirealm::event::Key::PageUp => todo!(),
                        tuirealm::event::Key::PageDown => todo!(),
                        tuirealm::event::Key::Tab => todo!(),
                        tuirealm::event::Key::BackTab => todo!(),
                        tuirealm::event::Key::Insert => todo!(),
                        tuirealm::event::Key::Function(_) => todo!(),
                        tuirealm::event::Key::Char(c) => {
                            let text = self.component.state().unwrap_one().unwrap_string();
                            let new_text = format!("{text}{c}");
                            self.component.state = State::One(StateValue::String(new_text.clone()));
                            Some(TuiMessage::UpdatedUserTypedLLMMessage(new_text))
                        }
                        tuirealm::event::Key::Null => todo!(),
                        tuirealm::event::Key::CapsLock => todo!(),
                        tuirealm::event::Key::ScrollLock => todo!(),
                        tuirealm::event::Key::NumLock => todo!(),
                        tuirealm::event::Key::PrintScreen => todo!(),
                        tuirealm::event::Key::Pause => todo!(),
                        tuirealm::event::Key::Menu => todo!(),
                        tuirealm::event::Key::KeypadBegin => todo!(),
                        tuirealm::event::Key::Media(_) => todo!(),
                        tuirealm::event::Key::Esc => todo!(),
                        tuirealm::event::Key::ShiftLeft => todo!(),
                        tuirealm::event::Key::AltLeft => todo!(),
                        tuirealm::event::Key::CtrlLeft => todo!(),
                        tuirealm::event::Key::ShiftRight => todo!(),
                        tuirealm::event::Key::AltRight => todo!(),
                        tuirealm::event::Key::CtrlRight => todo!(),
                        tuirealm::event::Key::ShiftUp => todo!(),
                        tuirealm::event::Key::AltUp => todo!(),
                        tuirealm::event::Key::CtrlUp => todo!(),
                        tuirealm::event::Key::ShiftDown => todo!(),
                        tuirealm::event::Key::AltDown => todo!(),
                        tuirealm::event::Key::CtrlDown => todo!(),
                        tuirealm::event::Key::CtrlHome => todo!(),
                        tuirealm::event::Key::CtrlEnd => todo!(),
                    }
                }
            }
            Event::User(msg) => return Some(TuiMessage::ActorMessage(msg)),
            _ => None,
        }
    }
}
