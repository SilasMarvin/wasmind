use std::collections::HashMap;

use crate::actors::tui::utils::offset_y;
use crate::actors::{ActorMessage, tui::model::TuiMessage};
use crate::actors::{AgentMessage, AgentMessageType, AgentType};
use crate::hive::{MAIN_MANAGER_ROLE, MAIN_MANAGER_SCOPE};
use crate::llm_client::ChatMessage;
use crate::{
    actors::{AssistantRequest, tui::components::llm_textarea::LLMTextAreaComponent},
    scope::Scope,
};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Offset};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State, StateValue,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};

const MESSAGE_GAP: u16 = 1;

// TODO: We need to create display widgets for plans, generic tool calls, file read and edited,
// etc...

#[derive(Clone)]
struct AssistantInfo {
    role: String,
    assistant_type: AgentType,
    task_description: Option<String>,
    last_assistant_request: Option<AssistantRequest>,
    pending_user_message: Option<String>,
}

impl Widget for AssistantInfo {
    // This render function assumes the area height is infinite
    fn render(self, mut area: Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        // Render top role title
        let title_paragraph = Paragraph::new(self.role)
            .style(Style::new())
            .alignment(Alignment::Center);
        let min_height = title_paragraph.line_count(area.width) as u16;
        area.height = min_height;
        tracing::error!("RENDERING TOP AREA: {:?}", area);
        title_paragraph.render(area, buf);
        area = offset_y(area, min_height + MESSAGE_GAP);

        if self.last_assistant_request.is_none() && self.pending_user_message.is_none() {
            let content = "Type to send a message".to_string();
            let paragraph = Paragraph::new(content)
                .block(Block::bordered())
                .style(Style::new())
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });
            area.height = 1;
            paragraph.render(area, buf);
        } else {
            // Render chat history
            if let Some(last_assistant_request) = self.last_assistant_request {
                for message in last_assistant_request.messages {
                    let content = serde_json::to_string_pretty(&message).unwrap();
                    let message_paragraph = Paragraph::new(content)
                        .block(Block::bordered())
                        .style(Style::new())
                        .alignment(Alignment::Left)
                        .wrap(Wrap { trim: true });
                    let min_height = message_paragraph.line_count(area.width) as u16;
                    area.height = min_height;
                    message_paragraph.render(area, buf);
                    area = offset_y(area, min_height + MESSAGE_GAP);
                }
            }

            // Render pending message
            if let Some(pending_message) = self.pending_user_message {
                let pending_message_paragraph = Paragraph::new(pending_message)
                    .block(Block::bordered())
                    .style(Style::new())
                    .alignment(Alignment::Center);
                pending_message_paragraph.render(area, buf);
            }
        }
    }
}

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
                chat_history_map: HashMap::from([(
                    MAIN_MANAGER_SCOPE.clone(),
                    AssistantInfo {
                        role: MAIN_MANAGER_ROLE.to_string(),
                        assistant_type: AgentType::MainManager,
                        task_description: None,
                        last_assistant_request: None,
                        pending_user_message: None,
                    },
                )]),
                active_scope: MAIN_MANAGER_SCOPE.clone(),
            },
        }
    }
}

struct ChatHistory {
    props: Props,
    state: State,
    active_scope: Scope,
    chat_history_map: HashMap<Scope, AssistantInfo>,
}

impl MockComponent for ChatHistory {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        // Check if visible
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            if let Some(info) = self.chat_history_map.get(&self.active_scope) {
                frame.render_widget(info.clone(), area);
            } else {
                tracing::error!(
                    "Trying to retrieve a scope that does not exist: {}",
                    self.active_scope
                );
            }
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
        CmdResult::None
    }
}

impl Component<TuiMessage, ActorMessage> for ChatHistoryComponent {
    fn on(&mut self, ev: Event<ActorMessage>) -> Option<TuiMessage> {
        match ev {
            Event::User(actor_message) => match actor_message.message {
                crate::actors::Message::UserContext(crate::actors::UserContext::UserTUIInput(
                    input,
                )) => {
                    if let Some(actor_info) = self
                        .component
                        .chat_history_map
                        .get_mut(&actor_message.scope)
                    {
                        actor_info.pending_user_message = Some(input);
                    }
                }
                // This is the real source of truth for what just got submitted by the LLM
                crate::actors::Message::AssistantRequest(assistant_request) => {
                    if let Some(actor_info) = self
                        .component
                        .chat_history_map
                        .get_mut(&actor_message.scope)
                    {
                        // TODO: Can we always assume the pending_user_message was submitted?
                        actor_info.pending_user_message = None;
                        actor_info.last_assistant_request = Some(assistant_request);
                    }
                }
                // These are intermediary artifacts that may be rolled back or changed by the real source of truth
                crate::actors::Message::AssistantToolCall(tool_call) => (),
                crate::actors::Message::AssistantResponse { message, .. } => {
                    if let Some(actor_info) = self
                        .component
                        .chat_history_map
                        .get_mut(&actor_message.scope)
                        && let Some(last_assistant_request) = &mut actor_info.last_assistant_request
                    {
                        last_assistant_request
                            .messages
                            .push(ChatMessage::Assistant(message));
                    }
                }
                crate::actors::Message::ToolCallUpdate(tool_call_update) => (),
                crate::actors::Message::FileRead {
                    path,
                    content,
                    last_modified,
                } => (),
                crate::actors::Message::FileEdited {
                    path,
                    content,
                    last_modified,
                } => (),
                crate::actors::Message::PlanUpdated(task_plan) => (),
                // This let's us track new agent creation
                crate::actors::Message::Agent(AgentMessage { agent_id, message }) => (),
                _ => (),
            },
            _ => (),
        }
        None
    }
}
