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
use ratatui::buffer::Buffer;
use ratatui::layout::Alignment;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Widget, WidgetRef, Wrap};
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};

use super::dashboard::SCOPE_ATTR;

// Constants from the main hive
const STARTING_SCOPE: &str = hive_actor_utils::STARTING_SCOPE;
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

fn create_user_widget(content: &str, area: Rect) -> (Box<dyn WidgetRef>, u16) {
    let borders = tuirealm::props::Borders::default();
    let block = create_block_with_title(
        format!("[ {} You ]", icons::USER_ICON),
        borders,
        false,
        Some(Padding::horizontal(1)),
    );
    let message_paragraph = Paragraph::new(content.to_string())
        .block(block)
        .style(Style::new())
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true });
    let min_height = message_paragraph.line_count(area.width) as u16;

    (Box::new(message_paragraph), min_height)
}

fn create_system_widget(content: &str, area: Rect) -> (Box<dyn WidgetRef>, u16) {
    let borders = tuirealm::props::Borders::default();
    let block = create_block_with_title(
        format!("[ {} System ]", icons::USER_ICON),
        borders,
        false,
        Some(Padding::horizontal(1)),
    );
    let message_paragraph = Paragraph::new(content.to_string())
        .block(block)
        .style(Style::new())
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true });
    let min_height = message_paragraph.line_count(area.width) as u16;

    (Box::new(message_paragraph), min_height)
}

fn create_tool_widget(
    tool_call: &ToolCall,
    status: &ToolCallStatus,
    area: Rect,
    is_expanded: bool,
) -> (Box<dyn WidgetRef>, u16) {
    let default_expanded_content = serde_json::to_string_pretty(&tool_call.function.arguments)
        .unwrap_or(tool_call.function.arguments.clone());

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
    message: &AssistantChatMessage,
    area: Rect,
    tool_call_updates: &HashMap<String, ToolCallStatus>,
) -> Vec<(Box<dyn WidgetRef>, u16)> {
    let mut widgets: Vec<(Box<dyn WidgetRef>, u16)> = vec![];

    if let Some(text_content) = &message.content
        && !text_content.is_empty()
    {
        let borders = tuirealm::props::Borders::default();
        let block = create_block_with_title(
            format!("[ {} Assistant ]", icons::LLM_ICON),
            borders,
            false,
            Some(Padding::horizontal(1)),
        );
        let p = Paragraph::new(text_content.clone())
            .block(block)
            .style(Style::new())
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: true });
        let min_height = p.line_count(area.width) as u16;
        widgets.push((Box::new(p), min_height));
    }

    if let Some(tool_calls) = &message.tool_calls {
        for tool_call in tool_calls {
            if let Some(status) = tool_call_updates.get(&tool_call.id) {
                widgets.push(create_tool_widget(tool_call, status, area, false));
            }
        }
    }

    widgets
}

struct ChatMessageWidgetState {
    message: ChatMessage,
    height: Option<u16>,
    buffer: Option<Buffer>,
    widgets: Vec<(Box<dyn WidgetRef>, u16)>,
}

impl ChatMessageWidgetState {
    fn get_height(
        &mut self,
        area: Rect,
        tool_call_updates: &HashMap<String, ToolCallStatus>,
    ) -> u16 {
        if let Some(height) = self.height {
            height
        } else {
            self.build_widgets(area, tool_call_updates);
            self.get_height(area, tool_call_updates)
        }
    }

    fn build_widgets(&mut self, area: Rect, tool_call_updates: &HashMap<String, ToolCallStatus>) {
        let widgets = match &self.message {
            ChatMessage::System(system_msg) => {
                vec![create_system_widget(&system_msg.content, area)]
            }
            ChatMessage::User(user_msg) => {
                vec![create_user_widget(&user_msg.content, area)]
            }
            ChatMessage::Assistant(assistant_chat_message) => {
                create_assistant_widgets(assistant_chat_message, area, tool_call_updates)
            }
            ChatMessage::Tool(_) => vec![],
        };
        let mut total_height = 0;
        for (i, (widget, height)) in widgets.into_iter().enumerate() {
            total_height += height + if i > 0 { MESSAGE_GAP } else { 0 };
            self.widgets.push((widget, height));
        }
        self.height = Some(total_height);
    }

    fn get_buff(
        &mut self,
        mut area: Rect,
        tool_call_updates: &HashMap<String, ToolCallStatus>,
    ) -> &Buffer {
        // Hack around Rust's borrow checker
        if self.buffer.is_some() {
            self.buffer.as_ref().unwrap()
        } else {
            area.height = self.get_height(area, tool_call_updates);
            area.x = 0;
            area.y = 0;
            let mut buff = Buffer::empty(area);
            for (widget, height) in &self.widgets {
                area.height = *height;
                widget.render_ref(area, &mut buff);
                area = offset_y(area, height + MESSAGE_GAP);
            }
            self.buffer = Some(buff);
            self.buffer.as_ref().unwrap()
        }
    }
}

fn convert_from_chat_state_to_chat_message_widget_state(
    chat_state: ChatState,
) -> Vec<ChatMessageWidgetState> {
    let mut msgs = vec![];
    msgs.push(ChatMessageWidgetState {
        message: ChatMessage::System(chat_state.system),
        height: None,
        buffer: None,
        widgets: vec![],
    });

    for msg in chat_state.messages {
        msgs.push(ChatMessageWidgetState {
            message: msg,
            height: None,
            buffer: None,
            widgets: vec![],
        });
    }

    msgs
}

struct AssistantInfo {
    role: String,
    chat_message_widget_state: Vec<ChatMessageWidgetState>,
    pending_user_message: Option<String>,
    tool_call_updates: HashMap<String, ToolCallStatus>,
}

impl AssistantInfo {
    fn new(role: String, _task_description: Option<String>) -> Self {
        Self {
            role,
            chat_message_widget_state: vec![],
            pending_user_message: None,
            tool_call_updates: HashMap::new(),
        }
    }
}

impl AssistantInfo {
    // Helper function to copy lines from message buffer to main buffer
    fn copy_buffer_lines(
        message_buffer: &Buffer,
        buf: &mut Buffer,
        area: Rect,
        source_start_line: u16,
        dest_start_line: u16,
        line_count: u16,
    ) {
        for line_offset in 0..line_count {
            let source_y = source_start_line + line_offset;
            let dest_y = dest_start_line + line_offset;

            let source_start = (source_y * area.width) as usize;
            let source_end = source_start + area.width as usize;

            let dest_start = (dest_y as usize) * (buf.area.width as usize) + (area.x as usize);
            let dest_end = dest_start + area.width as usize;

            // Bounds checking before attempting copy
            if source_y >= message_buffer.area.height {
                tracing::error!(
                    "Source Y out of bounds: source_y={}, message_buffer.height={}",
                    source_y,
                    message_buffer.area.height
                );
                continue;
            }

            if dest_y >= buf.area.height {
                tracing::error!(
                    "Dest Y out of bounds: dest_y={}, buf.height={}",
                    dest_y,
                    buf.area.height
                );
                continue;
            }

            if let Some(src_slice) = message_buffer.content.get(source_start..source_end)
                && let Some(dst_slice) = buf.content.get_mut(dest_start..dest_end)
            {
                dst_slice.clone_from_slice(src_slice);
            } else {
                tracing::error!(
                    "Buffer copy failed: source_y={}, dest_y={}, source_range={}..{}, dest_range={}..{}, msg_buf_size={}, dest_buf_size={}, area.width={}, buf.area.width={}",
                    source_y,
                    dest_y,
                    source_start,
                    source_end,
                    dest_start,
                    dest_end,
                    message_buffer.content.len(),
                    buf.content.len(),
                    area.width,
                    buf.area.width
                );
            }
        }
    }

    // This render function tracks total content height and supports scrolling
    fn render_ref_mut(&mut self, mut area: Rect, buf: &mut Buffer, scroll_offset: u16) -> u16 {
        let mut total_content_height = 0;

        // Render top role title
        let title_paragraph = Paragraph::new(format!("[ {} ]", self.role.clone()))
            .style(Style::new())
            .alignment(Alignment::Center);
        let title_height = title_paragraph.line_count(area.width) as u16;
        total_content_height += title_height + MESSAGE_GAP;

        // Only render title if it's visible
        if total_content_height > scroll_offset {
            title_paragraph.render(area, buf);
        }
        area = offset_y(area, title_height + MESSAGE_GAP);

        if self.chat_message_widget_state.is_empty() && self.pending_user_message.is_none() {
            let content = "It's quiet, too quiet...\n\nSend a message - don't be shy!".to_string();
            let block = Block::new()
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1));
            let paragraph = Paragraph::new(content)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .block(block);

            let width = paragraph.line_width();
            let mut empty_area = center_horizontal(area, width as u16);
            let height = paragraph.line_count(empty_area.width) as u16;
            empty_area.y += 10;
            empty_area.height = height;
            total_content_height += height + 10; // Include the padding

            // Only render if visible
            if total_content_height > scroll_offset {
                paragraph.render(empty_area, buf);
            }
        } else {
            // Render chat history
            let mut y_offset = 0;
            for message in &mut self.chat_message_widget_state {
                let height = message.get_height(area, &self.tool_call_updates);
                total_content_height += height + MESSAGE_GAP;

                tracing::error!("{} + {} - {}", y_offset, height, scroll_offset);

                if y_offset + height > scroll_offset && y_offset < scroll_offset + area.height {
                    let message_buffer = message.get_buff(area, &self.tool_call_updates);

                    // Calculate clipping for any message (handles all cases including both top and bottom clipping)
                    let top_clipping = if y_offset < scroll_offset {
                        scroll_offset - y_offset
                    } else {
                        0
                    };

                    let bottom_clipping = if y_offset + height > scroll_offset + area.height {
                        (y_offset + height) - (scroll_offset + area.height)
                    } else {
                        0
                    };

                    let visible_height = height - top_clipping - bottom_clipping;

                    // Only proceed if there's something visible
                    if visible_height > 0 {
                        let source_start_line = top_clipping;
                        let dest_start_line = area.y
                            + if y_offset > scroll_offset {
                                y_offset - scroll_offset
                            } else {
                                0
                            };

                        tracing::error!(
                            "RENDERING: source_start_line={}, dest_start_line={}, visible_height={}",
                            source_start_line,
                            dest_start_line,
                            visible_height
                        );

                        Self::copy_buffer_lines(
                            message_buffer,
                            buf,
                            area,
                            source_start_line,
                            dest_start_line,
                            visible_height,
                        );
                    }
                }

                y_offset += height + MESSAGE_GAP;
            }

            // Handle pending message
            if let Some(ref pending_message) = self.pending_user_message {
                let (widget, height) =
                    create_pending_user_message_widget(pending_message.clone(), area);
                total_content_height += height + MESSAGE_GAP;

                // Only render if visible (simplified check for now)
                if y_offset >= scroll_offset && y_offset - scroll_offset < area.height {
                    let mut pending_area = area;
                    pending_area.y += y_offset - scroll_offset;
                    pending_area.height = height;
                    widget.render_ref(pending_area, buf);
                }
            }
        }

        total_content_height
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
                chat_history_map: HashMap::from([(
                    STARTING_SCOPE.to_string(),
                    manager_assistant_info,
                )]),
                is_modified: false,
                scroll_offset: 0,
                content_height: 0,
                last_render_area: None,
            },
        }
    }
}

struct ChatHistory {
    props: Props,
    state: State,
    chat_history_map: HashMap<Scope, AssistantInfo>,
    is_modified: bool,
    scroll_offset: u16,
    content_height: u16,
    last_render_area: Option<Rect>,
}

impl MockComponent for ChatHistory {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true)
            && let Some(active_scope) = self.props.get(Attribute::Custom(SCOPE_ATTR))
        {
            let active_scope = active_scope.unwrap_string();
            let active_scope = Scope::try_from(active_scope.as_str()).unwrap();

            if let Some(info) = self.chat_history_map.get_mut(&active_scope) {
                // Store render area and get total content height
                self.last_render_area = Some(area);
                self.content_height =
                    info.render_ref_mut(area, frame.buffer_mut(), self.scroll_offset);
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
                    {
                        let scope = &add_message.agent;
                        if let Some(actor_info) = self.component.chat_history_map.get_mut(scope) {
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
                    {
                        let scope = &envelope.from_scope;
                        if let Some(actor_info) = self.component.chat_history_map.get_mut(scope) {
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
                    {
                        let scope = &envelope.from_scope;
                        if let Some(actor_info) = self.component.chat_history_map.get_mut(scope) {
                            actor_info.chat_message_widget_state =
                                convert_from_chat_state_to_chat_message_widget_state(
                                    chat_updated.chat_state,
                                );
                            self.component.is_modified = true;
                            return Some(TuiMessage::Redraw);
                        }
                    }
                }
                // Handle ToolCallStatusUpdate
                else if let Some(tool_update) =
                    parse_common_message_as::<ToolCallStatusUpdate>(&envelope)
                {
                    {
                        let scope = &envelope.from_scope;
                        if let Some(actor_info) = self.component.chat_history_map.get_mut(scope) {
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
                    {
                        let agent_scope = agent_spawned.agent_id.clone();
                        self.component
                            .chat_history_map
                            .insert(agent_scope, AssistantInfo::new(agent_spawned.name, None));
                        self.component.is_modified = true;
                        return Some(TuiMessage::Redraw);
                    }
                }
                None
            }
            Event::Mouse(mouse_event) => match mouse_event.kind {
                tuirealm::event::MouseEventKind::ScrollDown => {
                    // Scroll down (increase offset)
                    let scroll_speed = 3; // Lines to scroll per event
                    let max_offset = self.component.content_height.saturating_sub(
                        self.component
                            .last_render_area
                            .map(|a| a.height)
                            .unwrap_or(0),
                    );

                    self.component.scroll_offset = self
                        .component
                        .scroll_offset
                        .saturating_add(scroll_speed)
                        .min(max_offset);

                    Some(TuiMessage::Redraw)
                }
                tuirealm::event::MouseEventKind::ScrollUp => {
                    // Scroll up (decrease offset)
                    let scroll_speed = 3; // Lines to scroll per event
                    self.component.scroll_offset =
                        self.component.scroll_offset.saturating_sub(scroll_speed);

                    Some(TuiMessage::Redraw)
                }
                _ => None,
            },

            _ => None,
        }
    }
}
