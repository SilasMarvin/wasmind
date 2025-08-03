use std::collections::HashMap;

use crate::tui::utils::{center_horizontal, create_block_with_title, offset_y};
use crate::tui::{icons, model::TuiMessage};
use hive::{actors::MessageEnvelope, scope::Scope, utils::parse_common_message_as};
use hive_actor_utils_common_messages::{
    actors::AgentSpawned,
    assistant::{AddMessage, ChatState, ChatStateUpdated, Request as AssistantRequest},
    tools::{ToolCallStatus, ToolCallStatusUpdate},
};
use hive_llm_types::types::{AssistantChatMessage, ChatMessage, ToolCall};
use ratatui::layout::Alignment;
use ratatui::style::{Color, Style};
use ratatui::widgets::{
    Block, Borders, Padding, Paragraph, StatefulWidget, Widget, WidgetRef, Wrap,
};
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};

use super::dashboard::SCOPE_ATTR;
use super::scrollable::ScrollableComponentTrait;

// Constants from the main hive
const STARTING_SCOPE: Scope = hive::hive::STARTING_SCOPE;
const ROOT_AGENT_NAME: &str = "Root Agent";

const MESSAGE_GAP: u16 = 1;

fn create_pending_user_message_widget(content: String, area: Rect) -> (Box<dyn WidgetRef>, u16) {
    let borders = tuirealm::props::Borders::default();
    let block = create_block_with_title(
        format!("[ {} You - PENDING ]", icons::USER_ICON),
        borders,
        false,
        Some(Padding::horizontal(1)),
    );
    let message_paragraph = Paragraph::new(content)
        .block(block)
        .style(Style::new())
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true });
    let min_height = message_paragraph.line_count(area.width) as u16;

    (Box::new(message_paragraph), min_height)
}

fn create_user_widget(content: String, area: Rect) -> (Box<dyn WidgetRef>, u16) {
    let borders = tuirealm::props::Borders::default();
    let block = create_block_with_title(
        format!("[ {} You ]", icons::USER_ICON),
        borders,
        false,
        Some(Padding::horizontal(1)),
    );
    let message_paragraph = Paragraph::new(content)
        .block(block)
        .style(Style::new())
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true });
    let min_height = message_paragraph.line_count(area.width) as u16;

    (Box::new(message_paragraph), min_height)
}

fn create_system_widget(content: String, area: Rect) -> (Box<dyn WidgetRef>, u16) {
    let borders = tuirealm::props::Borders::default();
    let block = create_block_with_title(
        format!("[ {} System ]", icons::USER_ICON),
        borders,
        false,
        Some(Padding::horizontal(1)),
    );
    let message_paragraph = Paragraph::new(content)
        .block(block)
        .style(Style::new())
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true });
    let min_height = message_paragraph.line_count(area.width) as u16;

    (Box::new(message_paragraph), min_height)
}

fn create_tool_widget(
    tool_call: ToolCall,
    status: &ToolCallStatus,
    area: Rect,
    is_expanded: bool,
) -> (Box<dyn WidgetRef>, u16) {
    let default_expanded_content = serde_json::to_string_pretty(&tool_call.function.arguments)
        .unwrap_or(tool_call.function.arguments);

    let (errored, title, content, expanded_content) = match status {
        ToolCallStatus::Received { display_info } => {
            let (content, expanded_content) = (
                display_info.collapsed.clone(),
                display_info.expanded.clone(),
            );
            (
                false,
                format!("[ {} Tool: {} ]", icons::TOOL_ICON, tool_call.function.name),
                content,
                expanded_content,
            )
        }
        ToolCallStatus::AwaitingSystem { details } => {
            let content = format!("Awaiting system: {}", details.ui_display_info.collapsed);
            (
                false,
                format!("[ {} Tool: {} ]", icons::TOOL_ICON, tool_call.function.name),
                content,
                details.ui_display_info.expanded.clone(),
            )
        }
        ToolCallStatus::Done { result } => {
            let (errored, content, expanded_content) = match result {
                Ok(res) => (
                    false,
                    res.ui_display_info.collapsed.clone(),
                    res.ui_display_info.expanded.clone(),
                ),
                Err(res) => (
                    true,
                    res.ui_display_info.collapsed.clone(),
                    res.ui_display_info.expanded.clone(),
                ),
            };

            (
                errored,
                format!("[ {} Tool: {} ]", icons::TOOL_ICON, tool_call.function.name),
                content,
                expanded_content,
            )
        }
    };

    let content = if is_expanded {
        expanded_content.unwrap_or(default_expanded_content)
    } else {
        content
    };

    let border_color = if errored { Color::Red } else { Color::Yellow };
    let borders = tuirealm::props::Borders::default().color(border_color);
    let block = create_block_with_title(title, borders, false, Some(Padding::horizontal(1)));
    let p = Paragraph::new(content)
        .block(block)
        .style(Style::new())
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true });
    let min_height = p.line_count(area.width) as u16;

    (Box::new(p), min_height)
}

fn create_assistant_widgets(
    message: AssistantChatMessage,
    area: Rect,
    tool_call_updates: &HashMap<String, ToolCallStatus>,
) -> Vec<(Box<dyn WidgetRef>, u16)> {
    let mut widgets: Vec<(Box<dyn WidgetRef>, u16)> = vec![];

    if let Some(text_content) = message.content
        && !text_content.is_empty()
    {
        let borders = tuirealm::props::Borders::default();
        let block = create_block_with_title(
            format!("[ {} Assistant ]", icons::LLM_ICON),
            borders,
            false,
            Some(Padding::horizontal(1)),
        );
        let p = Paragraph::new(text_content)
            .block(block)
            .style(Style::new())
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: true });
        let min_height = p.line_count(area.width) as u16;
        widgets.push((Box::new(p), min_height));
    }

    if let Some(tool_calls) = message.tool_calls {
        for tool_call in tool_calls {
            if let Some(status) = tool_call_updates.get(&tool_call.id) {
                widgets.push(create_tool_widget(tool_call, status, area, false));
            }
        }
    }

    widgets
}

#[derive(Clone)]
struct AssistantInfo {
    role: String,
    chat_state: Option<ChatState>,
    pending_user_message: Option<String>,
    tool_call_updates: HashMap<String, ToolCallStatus>,
}

impl AssistantInfo {
    fn new(role: String, _task_description: Option<String>) -> Self {
        Self {
            role,
            chat_state: None,
            pending_user_message: None,
            tool_call_updates: HashMap::new(),
        }
    }
}

impl AssistantInfo {
    fn render_and_return_total_height(
        self,
        mut area: Rect,
        buf: &mut ratatui::prelude::Buffer,
    ) -> u16 {
        let mut total_height = 0;

        // Render top role title
        let title_paragraph = Paragraph::new(format!("[ {} ]", self.role.clone()))
            .style(Style::new())
            .alignment(Alignment::Center);
        let min_height = title_paragraph.line_count(area.width) as u16;
        area.height = min_height;
        title_paragraph.render(area, buf);
        area = offset_y(area, min_height + MESSAGE_GAP);
        total_height += min_height + MESSAGE_GAP;

        if self.chat_state.is_none() && self.pending_user_message.is_none() {
            let content = "It's quiet, too quiet...\n\nSend a message - don't be shy!".to_string();
            let block = Block::new()
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1));
            let paragraph = Paragraph::new(content)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .block(block);

            let width = paragraph.line_width();
            let mut area = center_horizontal(area, width as u16);
            let height = paragraph.line_count(area.width) as u16;
            area.y += 10;
            area.height = height;
            paragraph.render(area, buf);

            total_height += height + 10;
        } else {
            if let Some(chat_state) = self.chat_state {
                let (widget, height) = create_system_widget(chat_state.system.content, area);
                area.height = height;
                widget.render_ref(area, buf);
                area = offset_y(area, height + MESSAGE_GAP);
                total_height += height + MESSAGE_GAP;

                // Render chat history
                for message in chat_state.messages {
                    let widgets = match message {
                        ChatMessage::System(system_msg) => {
                            vec![create_system_widget(system_msg.content, area)]
                        }
                        ChatMessage::User(user_msg) => {
                            vec![create_user_widget(user_msg.content, area)]
                        }
                        ChatMessage::Assistant(assistant_chat_message) => create_assistant_widgets(
                            assistant_chat_message,
                            area,
                            &self.tool_call_updates,
                        ),
                        ChatMessage::Tool(_) => vec![],
                    };
                    for (widget, height) in widgets {
                        area.height = height;
                        widget.render_ref(area, buf);
                        area = offset_y(area, height + MESSAGE_GAP);
                        total_height += height + MESSAGE_GAP;
                    }
                }
            }

            // Render pending message
            if let Some(pending_message) = self.pending_user_message {
                let (widget, height) = create_pending_user_message_widget(pending_message, area);
                area.height = height;
                widget.render_ref(area, buf);
                total_height += height + MESSAGE_GAP;
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
    pub fn new(initial_prompt: Option<String>) -> Self {
        let mut props = Props::default();
        props.set(
            Attribute::Custom(SCOPE_ATTR),
            AttrValue::String(STARTING_SCOPE.to_string()),
        );

        let mut manager_assistant_info = AssistantInfo::new(ROOT_AGENT_NAME.to_string(), None);
        manager_assistant_info.pending_user_message = initial_prompt;

        Self {
            component: ChatHistory {
                props,
                state: State::None,
                chat_history_map: HashMap::from([(STARTING_SCOPE.clone(), manager_assistant_info)]),
                last_content_height: None,
                is_modified: false,
            },
        }
    }
}

struct ChatHistory {
    props: Props,
    state: State,
    chat_history_map: HashMap<Scope, AssistantInfo>,
    last_content_height: Option<u16>,
    is_modified: bool,
}

impl MockComponent for ChatHistory {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true)
            && let Some(active_scope) = self.props.get(Attribute::Custom(SCOPE_ATTR))
        {
            let active_scope = active_scope.unwrap_string();
            let active_scope = Scope::try_from(active_scope.as_str()).unwrap();

            if let Some(info) = self.chat_history_map.get(&active_scope) {
                let mut next_content_height = 0;
                frame.render_stateful_widget(info.clone(), area, &mut next_content_height);
                self.last_content_height = Some(next_content_height);
                self.is_modified = false;
            } else {
                tracing::error!(
                    "Trying to retrieve a scope that does not exist: {}",
                    active_scope
                );
            }
        }
    }

    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.props.get(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.props.set(attr, value);
        self.is_modified = true;
    }

    fn state(&self) -> State {
        self.state.clone()
    }

    fn perform(&mut self, _cmd: Cmd) -> CmdResult {
        unreachable!()
    }
}

impl Component<TuiMessage, MessageEnvelope> for ChatHistoryComponent {
    fn on(&mut self, ev: Event<MessageEnvelope>) -> Option<TuiMessage> {
        match ev {
            Event::User(envelope) => {
                // Handle AddMessage for user input
                if let Some(add_message) = parse_common_message_as::<AddMessage>(&envelope) {
                    if let Ok(scope) = add_message.agent.parse::<Scope>() {
                        if let Some(actor_info) = self.component.chat_history_map.get_mut(&scope) {
                            if let ChatMessage::User(user_msg) = add_message.message {
                                actor_info.pending_user_message = Some(user_msg.content);
                                self.component.is_modified = true;
                                return Some(TuiMessage::Redraw);
                            }
                        }
                    }
                }
                // Handle AssistantRequest to clear pending message
                else if let Some(_) = parse_common_message_as::<AssistantRequest>(&envelope) {
                    if let Ok(scope) = envelope.from_scope.parse::<Scope>() {
                        if let Some(actor_info) = self.component.chat_history_map.get_mut(&scope) {
                            actor_info.pending_user_message = None;
                            self.component.is_modified = true;
                            return Some(TuiMessage::Redraw);
                        }
                    }
                }
                // Handle ChatStateUpdated
                else if let Some(chat_updated) =
                    parse_common_message_as::<ChatStateUpdated>(&envelope)
                {
                    if let Ok(scope) = envelope.from_scope.parse::<Scope>() {
                        if let Some(actor_info) = self.component.chat_history_map.get_mut(&scope) {
                            actor_info.chat_state = Some(chat_updated.chat_state);
                            self.component.is_modified = true;
                            return Some(TuiMessage::Redraw);
                        }
                    }
                }
                // Handle ToolCallStatusUpdate
                else if let Some(tool_update) =
                    parse_common_message_as::<ToolCallStatusUpdate>(&envelope)
                {
                    if let Ok(scope) = envelope.from_scope.parse::<Scope>() {
                        if let Some(actor_info) = self.component.chat_history_map.get_mut(&scope) {
                            actor_info
                                .tool_call_updates
                                .insert(tool_update.id, tool_update.status);
                            self.component.is_modified = true;
                            return Some(TuiMessage::Redraw);
                        }
                    }
                }
                // Handle AgentSpawned to track new agent creation
                else if let Some(agent_spawned) =
                    parse_common_message_as::<AgentSpawned>(&envelope)
                {
                    if let Ok(agent_scope) = agent_spawned.agent_id.parse::<Scope>() {
                        self.component
                            .chat_history_map
                            .insert(agent_scope, AssistantInfo::new(agent_spawned.name, None));
                        self.component.is_modified = true;
                        return Some(TuiMessage::Redraw);
                    }
                }
                None
            }
            _ => None,
        }
    }
}

impl ScrollableComponentTrait<TuiMessage, MessageEnvelope> for ChatHistoryComponent {
    fn is_modified(&self) -> bool {
        self.component.is_modified
    }

    fn get_content_height(&self, _area: Rect) -> u16 {
        self.component.last_content_height.unwrap_or(0)
    }
}
