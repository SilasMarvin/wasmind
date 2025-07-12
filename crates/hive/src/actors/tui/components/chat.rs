// use crate::actors::tui::components::llm_textarea::LLMTextAreaComponent;
use crate::actors::tui::utils;
use crate::actors::{ActorMessage, tui::model::TuiMessage};
use crate::config::ParsedTuiConfig;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::widgets::Padding;
use tuirealm::props::{BorderSides, Borders};
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State, StateValue,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};

use super::chat_history::ChatHistoryComponent;
use super::scrollable::ScrollableComponent;
use super::textarea::LLMTextAreaComponent;

/// Actions the user can bind keys to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChatUserAction {
    Assist,
}

impl TryFrom<&str> for ChatUserAction {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "Assist" => Ok(ChatUserAction::Assist),
            _ => Err(()),
        }
    }
}

#[derive(MockComponent)]
pub struct ChatAreaComponent {
    component: ChatArea,
}

impl ChatAreaComponent {
    pub fn new(config: ParsedTuiConfig) -> Self {
        Self {
            component: ChatArea {
                props: Props::default(),
                state: State::One(StateValue::String("".to_string())),
                llm_textarea: LLMTextAreaComponent::new(config.clone()),
                chat_history: ScrollableComponent::new(Box::new(ChatHistoryComponent::new()), true),
            },
        }
    }
}

struct ChatArea {
    props: Props,
    state: State,
    llm_textarea: LLMTextAreaComponent,
    chat_history: ScrollableComponent,
}

impl MockComponent for ChatArea {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        // Check if visible
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            let textarea_height = self.llm_textarea.get_height(area);

            let borders = Borders::default().sides(BorderSides::LEFT);
            let div = utils::create_block(borders, false, Some(Padding::uniform(1)));
            frame.render_widget(div, area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints(
                    [
                        Constraint::Percentage(100),
                        Constraint::Min(1),
                        Constraint::Min(textarea_height as u16),
                    ]
                    .as_ref(),
                )
                .split(area);

            self.chat_history.view(frame, chunks[0]);
            self.llm_textarea.view(frame, chunks[2]);
        }
    }

    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.props.get(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.llm_textarea.component.attr(attr, value.clone());
        self.chat_history.attr(attr, value);
    }

    fn state(&self) -> State {
        self.state.clone()
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        CmdResult::Batch(vec![self.llm_textarea.perform(cmd)])
    }
}

impl Component<TuiMessage, ActorMessage> for ChatAreaComponent {
    fn on(&mut self, ev: Event<ActorMessage>) -> Option<TuiMessage> {
        match (
            self.component.chat_history.on(ev.clone()),
            self.component.llm_textarea.on(ev),
        ) {
            (None, None) => None,
            (None, Some(msg)) => Some(msg),
            (Some(msg), None) => Some(msg),
            (Some(msg1), Some(msg2)) => Some(TuiMessage::Batch(vec![msg1, msg2])),
        }
    }
}
