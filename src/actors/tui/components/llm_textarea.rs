use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State, StateValue,
    command::{Cmd, CmdResult},
    props::{Alignment, Borders, Color, Style, TextModifiers},
    ratatui::{
        layout::Rect,
        widgets::{Block, Paragraph},
    },
};

use crate::actors::{ActorMessage, tui::model::TuiMessage};

pub fn get_block<'a>(props: Borders, title: (String, Alignment), focus: bool) -> Block<'a> {
    Block::default()
        .borders(props.sides)
        .border_style(if focus {
            props.style()
        } else {
            Style::default().fg(Color::Reset).bg(Color::Reset)
        })
        .border_type(props.modifiers)
        .title(title.0)
        .title_alignment(title.1)
}

#[derive(MockComponent)]
pub struct LLMTextAreaComponent {
    component: LLMTextArea,
}

impl LLMTextAreaComponent {
    pub fn new() -> Self {
        Self {
            component: LLMTextArea {
                props: Props::default(),
                state: State::One(StateValue::String("".to_string())),
            },
        }
    }
}

struct LLMTextArea {
    props: Props,
    state: State,
}

impl MockComponent for LLMTextArea {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        // Check if visible
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            // Get properties
            let text = self.state.clone().unwrap_one().unwrap_string();
            let alignment = self
                .props
                .get_or(Attribute::TextAlign, AttrValue::Alignment(Alignment::Left))
                .unwrap_alignment();
            let foreground = self
                .props
                .get_or(Attribute::Foreground, AttrValue::Color(Color::Reset))
                .unwrap_color();
            let background = self
                .props
                .get_or(Attribute::Background, AttrValue::Color(Color::Reset))
                .unwrap_color();
            let modifiers = self
                .props
                .get_or(
                    Attribute::TextProps,
                    AttrValue::TextModifiers(TextModifiers::empty()),
                )
                .unwrap_text_modifiers();
            let title = self
                .props
                .get_or(
                    Attribute::Title,
                    AttrValue::Title((String::default(), Alignment::Center)),
                )
                .unwrap_title();
            let borders = self
                .props
                .get_or(Attribute::Borders, AttrValue::Borders(Borders::default()))
                .unwrap_borders();
            let focus = self
                .props
                .get_or(Attribute::Focus, AttrValue::Flag(false))
                .unwrap_flag();
            frame.render_widget(
                Paragraph::new(text)
                    .block(get_block(borders, title, focus))
                    .style(
                        Style::default()
                            .fg(foreground)
                            .bg(background)
                            .add_modifier(modifiers),
                    )
                    .alignment(alignment),
                area,
            );
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

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        match cmd {
            Cmd::Submit => {
                // TODO: Handle the state change when we submit
                CmdResult::Changed(self.state())
            }
            Cmd::Type(char) => {
                let text = self.state().unwrap_one().unwrap_string();
                self.state = State::One(StateValue::String(format!("{text}{char}")));
                CmdResult::Changed(self.state())
            }
            _ => CmdResult::None,
        }
    }
}

impl Component<TuiMessage, ActorMessage> for LLMTextAreaComponent {
    fn on(&mut self, ev: Event<ActorMessage>) -> Option<TuiMessage> {
        let cmd = match ev {
            Event::Keyboard(key_event) => match key_event.code {
                tuirealm::event::Key::Backspace => todo!(),
                tuirealm::event::Key::Enter => todo!(),
                tuirealm::event::Key::Left => todo!(),
                tuirealm::event::Key::Right => todo!(),
                tuirealm::event::Key::Up => todo!(),
                tuirealm::event::Key::Down => todo!(),
                tuirealm::event::Key::Home => todo!(),
                tuirealm::event::Key::End => todo!(),
                tuirealm::event::Key::PageUp => todo!(),
                tuirealm::event::Key::PageDown => todo!(),
                tuirealm::event::Key::Tab => todo!(),
                tuirealm::event::Key::BackTab => todo!(),
                tuirealm::event::Key::Delete => todo!(),
                tuirealm::event::Key::Insert => todo!(),
                tuirealm::event::Key::Function(_) => todo!(),
                tuirealm::event::Key::Char(c) => Some(Cmd::Type(c)),
                tuirealm::event::Key::Null => todo!(),
                tuirealm::event::Key::CapsLock => todo!(),
                tuirealm::event::Key::ScrollLock => todo!(),
                tuirealm::event::Key::NumLock => todo!(),
                tuirealm::event::Key::PrintScreen => todo!(),
                tuirealm::event::Key::Pause => todo!(),
                tuirealm::event::Key::Menu => todo!(),
                tuirealm::event::Key::KeypadBegin => todo!(),
                tuirealm::event::Key::Media(media_key_code) => todo!(),
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
            },
            Event::User(msg) => return Some(TuiMessage::ActorMessage(msg)),
            _ => None,
        };

        if let Some(cmd) = cmd {
            match self.perform(cmd) {
                CmdResult::Changed(State::One(StateValue::String(typed_text))) => {
                    Some(TuiMessage::UpdatedUserTypedLLMMessage(typed_text))
                }
                _ => None,
            }
        } else {
            None
        }
    }
}
