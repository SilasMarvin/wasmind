use std::collections::HashMap;

use crate::actors::tui::utils;
use crate::actors::{ActorMessage, tui::model::TuiMessage};
use crate::{
    actors::{AssistantRequest, tui::components::llm_textarea::LLMTextAreaComponent},
    scope::Scope,
};
use ratatui::layout::{Constraint, Direction, Layout};
use tuirealm::props::Borders;
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State, StateValue,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};

use super::chat_history::ChatHistoryComponent;

pub const CHAT_SCOPE: Scope = Scope::from_uuid(uuid::uuid!("00000000-0000-0000-0000-d68b0e6c4cf1"));

#[derive(MockComponent)]
pub struct ChatAreaComponent {
    component: ChatArea,
}

impl ChatAreaComponent {
    pub fn new() -> Self {
        Self {
            component: ChatArea {
                props: Props::default(),
                state: State::One(StateValue::String("".to_string())),
                llm_textarea: LLMTextAreaComponent::new(),
                chat_history: ChatHistoryComponent::new(),
            },
        }
    }
}

struct ChatArea {
    props: Props,
    state: State,
    llm_textarea: LLMTextAreaComponent,
    chat_history: ChatHistoryComponent,
}

impl MockComponent for ChatArea {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        // Check if visible
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            let textarea_height = self.llm_textarea.get_height(area);

            let borders = Borders::default();
            let div = utils::get_block(borders, false);
            frame.render_widget(div, area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints(
                    [
                        Constraint::Percentage(100),
                        Constraint::Min(textarea_height as u16),
                    ]
                    .as_ref(),
                )
                .split(area);

            self.chat_history.view(frame, chunks[0]);
            self.llm_textarea.view(frame, chunks[1]);
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
        // This pass through may be unnecessary as I believe we are the only ones that call perform and that is passed through in the `on` function
        CmdResult::Batch(vec![self.llm_textarea.perform(cmd)])
    }
}

impl Component<TuiMessage, ActorMessage> for ChatAreaComponent {
    fn on(&mut self, ev: Event<ActorMessage>) -> Option<TuiMessage> {
        self.component.llm_textarea.on(ev)
    }
}
