use ratatui::layout::{Constraint, Direction, Layout};
use tuirealm::props::{BorderSides, Borders};
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State, StateValue,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};
use wasmind::actors::MessageEnvelope;

use crate::config::ParsedTuiConfig;
use crate::tui::{model::TuiMessage, utils};

use super::chat_history::ChatHistoryComponent;
use super::textarea::LLMTextAreaComponent;

/// Actions the user can bind keys to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChatUserAction {
    Assist,
    ToggleToolExpansion,
}

impl ChatUserAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChatUserAction::Assist => "Assist",
            ChatUserAction::ToggleToolExpansion => "ToggleToolExpansion",
        }
    }
}

impl TryFrom<&str> for ChatUserAction {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "Assist" => Ok(ChatUserAction::Assist),
            "ToggleToolExpansion" => Ok(ChatUserAction::ToggleToolExpansion),
            _ => Err(()),
        }
    }
}

#[derive(MockComponent)]
pub struct ChatAreaComponent {
    component: ChatArea,
}

impl ChatAreaComponent {
    pub fn new(config: ParsedTuiConfig, initial_prompt: Option<String>) -> Self {
        Self {
            component: ChatArea {
                props: Props::default(),
                state: State::One(StateValue::String("".to_string())),
                llm_textarea: LLMTextAreaComponent::new(config.clone()),
                chat_history: ChatHistoryComponent::new(initial_prompt),
                config,
            },
        }
    }
}

struct ChatArea {
    props: Props,
    state: State,
    llm_textarea: LLMTextAreaComponent,
    chat_history: ChatHistoryComponent,
    config: ParsedTuiConfig,
}

impl MockComponent for ChatArea {
    fn view(&mut self, frame: &mut Frame, mut area: Rect) {
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            let textarea_height = self.llm_textarea.get_height(area);

            let borders = Borders::default().sides(BorderSides::LEFT);
            let div = utils::create_block(borders, false, None);
            frame.render_widget(div, area);

            // Adjust the x for the border on the left
            area.x += 2;
            area.width -= 2;

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Percentage(100),
                        Constraint::Min(2),
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

impl Component<TuiMessage, MessageEnvelope> for ChatAreaComponent {
    fn on(&mut self, ev: Event<MessageEnvelope>) -> Option<TuiMessage> {
        // Handle keyboard events for chat actions
        // TODO: Pull the assist action here and out of the llm text area section
        if let Event::Keyboard(key_event) = &ev
            && let Some(action) = self.component.config.chat.key_bindings.get(key_event)
            && action == &ChatUserAction::ToggleToolExpansion
        {
            self.component.chat_history.toggle_tool_expansion();
            return Some(TuiMessage::Redraw);
        }

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
