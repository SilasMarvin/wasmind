use std::collections::HashMap;

use crate::actors::tui::utils::offset_y;
use crate::actors::{ActorMessage, tui::model::TuiMessage};
use crate::actors::{AgentMessage, AgentType, ToolCallStatus};
use crate::hive::{MAIN_MANAGER_ROLE, MAIN_MANAGER_SCOPE};
use crate::llm_client::ChatMessage;
use crate::{actors::AssistantRequest, scope::Scope};
use ratatui::layout::Alignment;
use ratatui::style::Style;
use ratatui::widgets::{Block, Paragraph, StatefulWidget, Widget, Wrap};
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State, StateValue,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};

use super::scrollable::ScrollableComponentTrait;

const MESSAGE_GAP: u16 = 1;

// TODO: We need to create display widgets for plans, generic tool calls, file read and edited, etc...

#[derive(Clone)]
struct AssistantInfo {
    role: String,
    _assistant_type: AgentType,
    _task_description: Option<String>,
    last_assistant_request: Option<AssistantRequest>,
    pending_user_message: Option<String>,
    tool_call_updates: HashMap<String, ToolCallStatus>,
}

impl AssistantInfo {
    fn render_and_return_total_height(
        self,
        mut area: Rect,
        buf: &mut ratatui::prelude::Buffer,
    ) -> u16 {
        let mut total_height = 0;

        // Render top role title
        for i in 0..100 {
            let title_paragraph = Paragraph::new(format!("{} | {} ", self.role.clone(), i))
                .style(Style::new())
                .alignment(Alignment::Center);
            let min_height = title_paragraph.line_count(area.width) as u16;
            area.height = min_height;
            title_paragraph.render(area, buf);
            area = offset_y(area, min_height + MESSAGE_GAP);
            total_height += min_height + MESSAGE_GAP;
        }

        if self.last_assistant_request.is_none() && self.pending_user_message.is_none() {
            let content = "Type to send a message".to_string();
            let paragraph = Paragraph::new(content)
                .block(Block::bordered())
                .style(Style::new())
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });
            area.height = 3;
            paragraph.render(area, buf);
            total_height += 3;
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
                    total_height += min_height + MESSAGE_GAP;
                }
            }

            // Render pending message
            if let Some(pending_message) = self.pending_user_message {
                let pending_message_paragraph = Paragraph::new(pending_message)
                    .block(Block::bordered())
                    .style(Style::new())
                    .alignment(Alignment::Center);
                let min_height = pending_message_paragraph.line_count(area.width) as u16;
                area.height = min_height;
                pending_message_paragraph.render(area, buf);
                total_height += min_height + MESSAGE_GAP;
            }
        }

        total_height
    }
}

impl StatefulWidget for AssistantInfo {
    type State = u16;

    // This render function assumes the area height is infinite
    fn render(self, area: Rect, buf: &mut ratatui::prelude::Buffer, state: &mut Self::State)
    where
        Self: Sized,
    {
        *state = self.render_and_return_total_height(area, buf);
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
                        _assistant_type: AgentType::MainManager,
                        _task_description: None,
                        last_assistant_request: None,
                        pending_user_message: None,
                        tool_call_updates: HashMap::new(),
                    },
                )]),
                active_scope: MAIN_MANAGER_SCOPE.clone(),
                last_content_height: None,
                is_modified: true,
            },
        }
    }
}

struct ChatHistory {
    props: Props,
    state: State,
    active_scope: Scope,
    chat_history_map: HashMap<Scope, AssistantInfo>,
    last_content_height: Option<u16>,
    is_modified: bool,
}

impl MockComponent for ChatHistory {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        // Check if visible
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            if let Some(info) = self.chat_history_map.get(&self.active_scope) {
                let mut next_content_height = 0;
                frame.render_stateful_widget(info.clone(), area, &mut next_content_height);
                self.last_content_height = Some(next_content_height);
                self.is_modified = false;
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
                        self.component.is_modified = true;
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
                        self.component.is_modified = true;
                    }
                }
                // These are intermediary artifacts that may be rolled back or changed by the real source of truth
                crate::actors::Message::AssistantToolCall(_) => (),
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
                        self.component.is_modified = true;
                    }
                }
                crate::actors::Message::ToolCallUpdate(tool_call_update) => {
                    if let Some(actor_info) = self
                        .component
                        .chat_history_map
                        .get_mut(&actor_message.scope)
                    {
                        actor_info
                            .tool_call_updates
                            .insert(tool_call_update.call_id, tool_call_update.status);
                    }
                }
                // This let's us track new agent creation
                crate::actors::Message::Agent(AgentMessage { .. }) => (),
                _ => (),
            },
            _ => (),
        }
        None
    }
}

impl ScrollableComponentTrait<TuiMessage, ActorMessage> for ChatHistoryComponent {
    fn is_modified(&self) -> bool {
        self.component.is_modified
    }

    fn get_content_height(&self, _area: Rect) -> u16 {
        self.component.last_content_height.unwrap_or(0)
    }
}
