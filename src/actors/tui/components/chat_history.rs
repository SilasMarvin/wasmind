use std::collections::HashMap;

use crate::actors::{ActorMessage, tui::model::TuiMessage};
use crate::{
    actors::{AssistantRequest, tui::components::llm_textarea::LLMTextAreaComponent},
    scope::Scope,
};
use ratatui::layout::{Constraint, Direction, Layout};
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State, StateValue,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};

// TODO: We need to create display widgets for plans, generic tool calls, file read and edited,
// etc...

#[derive(MockComponent)]
pub struct ChatHistoryComponent {
    component: ChatHistory,
}

impl ChatHistoryComponent {
    pub fn new() -> Self {
        Self {
            component: ChatHistory {
                props: Props::default(),
                state: State::One(StateValue::String("".to_string())),
                chat_history_map: HashMap::new(),
            },
        }
    }
}

struct ChatHistory {
    props: Props,
    state: State,
    chat_history_map: HashMap<Scope, AssistantRequest>,
}

impl MockComponent for ChatHistory {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        // Check if visible
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {}
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
        CmdResult::None
    }
}

impl Component<TuiMessage, ActorMessage> for ChatHistoryComponent {
    fn on(&mut self, ev: Event<ActorMessage>) -> Option<TuiMessage> {
        match ev {
            Event::User(actor_message) => match actor_message.message {
                // This is the real source of truth for what just got submitted by the LLM
                crate::actors::Message::AssistantSpawned { scope, role, task } => None,
                crate::actors::Message::AssistantRequest(assistant_request) => None,
                // These are intermediary artifacts that may be rolled back or changed by the real source of truth
                crate::actors::Message::AssistantToolCall(tool_call) => None,
                crate::actors::Message::AssistantResponse { id, content } => None,
                crate::actors::Message::ToolCallUpdate(tool_call_update) => None,
                crate::actors::Message::FileRead {
                    path,
                    content,
                    last_modified,
                } => None,
                crate::actors::Message::FileEdited {
                    path,
                    content,
                    last_modified,
                } => None,
                crate::actors::Message::PlanUpdated(task_plan) => None,
                _ => None,
            },
            _ => None,
        }
    }
}
