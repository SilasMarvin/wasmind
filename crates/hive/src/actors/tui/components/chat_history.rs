use std::collections::HashMap;

use crate::actors::tui::icons;
use crate::actors::tui::utils::{center, center_horizontal, create_block_with_title, offset_y};
use crate::actors::{ActorMessage, tui::model::TuiMessage};
use crate::actors::{AgentMessage, AgentMessageType, AgentType, ToolCallStatus};
use crate::hive::{MAIN_MANAGER_ROLE, MAIN_MANAGER_SCOPE};
use crate::llm_client::{AssistantChatMessage, ChatMessage, ToolCall};
use crate::scope::Scope;
use ratatui::layout::{Alignment, Constraint};
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

const MESSAGE_GAP: u16 = 1;

fn create_pending_user_message_widget(content: String, area: Rect) -> (Box<dyn WidgetRef>, u16) {
    let borders = tuirealm::props::Borders::default();
    let block = create_block_with_title(
        format!("[ {} You - PENDING ]", icons::USER_ICON),
        borders,
        false,
        Some(Padding::uniform(1)),
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
        Some(Padding::uniform(1)),
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
        Some(Padding::uniform(1)),
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

    let (title, content, expanded_content) = match status {
        ToolCallStatus::Received => (
            format!("[ {} Tool: {} ]", icons::TOOL_ICON, tool_call.function.name),
            "Processing".to_string(),
            None,
        ),
        ToolCallStatus::AwaitingUserYNConfirmation => (
            format!("[ {} Tool: {} ]", icons::TOOL_ICON, tool_call.function.name),
            "Awaiting user confirmation".to_string(),
            None,
        ),
        ToolCallStatus::ReceivedUserYNConfirmation(_) => (
            format!("[ {} Tool: {} ]", icons::TOOL_ICON, tool_call.function.name),
            "ReceivedUserYNConfirmation".to_string(),
            None,
        ),
        ToolCallStatus::Finished {
            result,
            tui_display,
        } => {
            let (content, expanded_content) = match tui_display {
                Some(tui_display) => (tui_display.collapsed.clone(), tui_display.expanded.clone()),
                None => match result {
                    Ok(res) => ("Success".to_string(), Some(res.to_string())),
                    Err(e) => ("Error".to_string(), Some(e.to_string())),
                },
            };

            (
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

    let borders = tuirealm::props::Borders::default().color(Color::Yellow);
    let block = create_block_with_title(title, borders, false, Some(Padding::uniform(1)));
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
            Some(Padding::uniform(1)),
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
    _assistant_type: AgentType,
    _task_description: Option<String>,
    chat_history: Vec<ChatMessage>,
    pending_user_message: Option<String>,
    tool_call_updates: HashMap<String, ToolCallStatus>,
}

impl AssistantInfo {
    fn new(role: String, _assistant_type: AgentType, _task_description: Option<String>) -> Self {
        Self {
            role,
            _assistant_type,
            _task_description,
            chat_history: vec![],
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

        if self.chat_history.is_empty() && self.pending_user_message.is_none() {
            let content = "It's quiet, too quiet...\n\nSend a message - don't be shy!".to_string();
            let block = Block::new()
                .borders(Borders::ALL)
                .padding(Padding::uniform(1));
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
            // Render chat history
            for message in self.chat_history {
                let widgets = match message {
                    ChatMessage::System { content } => {
                        vec![create_system_widget(content, area)]
                    }
                    ChatMessage::User { content } => {
                        vec![create_user_widget(content, area)]
                    }
                    ChatMessage::Assistant(assistant_chat_message) => create_assistant_widgets(
                        assistant_chat_message,
                        area,
                        &self.tool_call_updates,
                    ),
                    ChatMessage::Tool { .. } => vec![],
                };
                for (widget, height) in widgets {
                    area.height = height;
                    widget.render_ref(area, buf);
                    area = offset_y(area, height + MESSAGE_GAP);
                    total_height += height + MESSAGE_GAP;
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
    pub fn new() -> Self {
        let mut props = Props::default();
        props.set(
            Attribute::Custom(SCOPE_ATTR),
            AttrValue::String(MAIN_MANAGER_SCOPE.to_string()),
        );
        Self {
            component: ChatHistory {
                props,
                state: State::None,
                chat_history_map: HashMap::from([(
                    MAIN_MANAGER_SCOPE.clone(),
                    AssistantInfo::new(MAIN_MANAGER_ROLE.to_string(), AgentType::MainManager, None),
                )]),
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

                        return Some(TuiMessage::Redraw);
                    }
                }
                crate::actors::Message::AssistantRequest(_) => {
                    if let Some(actor_info) = self
                        .component
                        .chat_history_map
                        .get_mut(&actor_message.scope)
                    {
                        actor_info.pending_user_message = None;
                        self.component.is_modified = true;
                        return Some(TuiMessage::Redraw);
                    }
                }
                crate::actors::Message::AssistantChatUpdated(messages) => {
                    if let Some(actor_info) = self
                        .component
                        .chat_history_map
                        .get_mut(&actor_message.scope)
                    {
                        actor_info.chat_history = messages;
                        self.component.is_modified = true;

                        return Some(TuiMessage::Redraw);
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
                        self.component.is_modified = true;

                        return Some(TuiMessage::Redraw);
                    }
                }
                // This let's us track new agent creation
                crate::actors::Message::Agent(AgentMessage {
                    agent_id,
                    message:
                        AgentMessageType::AgentSpawned {
                            agent_type,
                            role,
                            task_description,
                            ..
                        },
                }) => {
                    self.component.chat_history_map.insert(
                        agent_id,
                        AssistantInfo::new(role, agent_type, Some(task_description)),
                    );
                }
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
