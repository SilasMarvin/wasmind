use ratatui::layout::Alignment;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Widget, WidgetRef, Wrap};
use ratatui::{buffer::Buffer, widgets::Clear};
use std::cell::RefCell;
use std::collections::HashMap;
use throbber_widgets_tui::{OGHAM_C, Throbber, VERTICAL_BLOCK, symbols::throbber};
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};
use wasmind::{actors::MessageEnvelope, scope::Scope, utils::parse_common_message_as};
use wasmind_actor_utils::common_messages::{
    actors::AgentSpawned,
    assistant::{AddMessage, ChatState, ChatStateUpdated, Request as AssistantRequest},
    tools::{ToolCallStatus, ToolCallStatusUpdate},
};
use wasmind_actor_utils::llm_client_types::{
    AssistantChatMessage, ChatMessage, ChatMessageWithRequestId, ToolCall,
};

use crate::tui::throbber_in_title_ext::ThrobberInTitleExt;
use crate::tui::utils::{center_horizontal, create_block_with_title};
use crate::tui::{icons, model::TuiMessage};

use super::dashboard::SCOPE_ATTR;

// Constants from the main wasmind system
const STARTING_SCOPE: &str = wasmind_actor_utils::STARTING_SCOPE;
const ROOT_AGENT_NAME: &str = "Root Agent";

const MESSAGE_GAP: u16 = 1;

// Thread-local storage for throbber state during rendering (legacy - can be removed)
thread_local! {
    static CURRENT_THROBBER_STATE: RefCell<Option<()>> = const { RefCell::new(None) };
}

// =============================================================================
// THROBBER PARAGRAPH WIDGET
// =============================================================================

/// A paragraph widget that can render with animated throbbers in the title
struct ThrobberParagraph {
    paragraph: Paragraph<'static>,
    throbber_pos: usize,
    throbber_set: throbber::Set,
}

impl ThrobberParagraph {
    fn new(
        paragraph: Paragraph<'static>,
        throbber_pos: usize,
        throbber_set: throbber::Set,
    ) -> Self {
        Self {
            paragraph,
            throbber_pos,
            throbber_set,
        }
    }
}

impl WidgetRef for ThrobberParagraph {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        let throbber = Throbber::default().throbber_set(self.throbber_set.clone());
        self.paragraph
            .clone()
            .render_buf_with_throbber(buf, area, self.throbber_pos, throbber);
    }
}

// =============================================================================
// UTILITY FUNCTIONS
// =============================================================================

/// Trims leading and trailing newlines from message content while preserving internal formatting
fn trim_message_content(content: &str) -> &str {
    content.trim_start_matches('\n').trim_end_matches('\n')
}

// =============================================================================
// CACHING SYSTEM
// =============================================================================

/// Trait for items that can be rendered with caching support
trait CacheableRenderItem {
    type Context<'a>;

    fn get_height<'a>(&mut self, area: Rect, context: &Self::Context<'a>) -> u16;
    fn get_buffer<'a>(&mut self, area: Rect, context: &Self::Context<'a>) -> &Buffer;
    fn invalidate_cache(&mut self);
    fn has_active_throbber<'a>(&self, _context: &Self::Context<'a>) -> bool {
        false
    } // Default implementation: no throbber
}

// Cached wrapper for simple Paragraph widgets
struct CachedParagraph {
    paragraph: Paragraph<'static>,
    height: Option<u16>,
    buffer: Option<Buffer>,
}

impl CachedParagraph {
    fn new(paragraph: Paragraph<'static>) -> Self {
        Self {
            paragraph,
            height: None,
            buffer: None,
        }
    }
}

impl CacheableRenderItem for CachedParagraph {
    type Context<'a> = ();

    fn get_height<'a>(&mut self, area: Rect, _context: &()) -> u16 {
        if let Some(height) = self.height {
            height
        } else {
            let height = self.paragraph.line_count(area.width) as u16;
            self.height = Some(height);
            height
        }
    }

    fn get_buffer<'a>(&mut self, area: Rect, context: &()) -> &Buffer {
        if self.buffer.is_some() {
            self.buffer.as_ref().unwrap()
        } else {
            let height = self.get_height(area, context);
            let mut buffer_area = area;
            buffer_area.height = height;
            buffer_area.x = 0;
            buffer_area.y = 0;
            let mut buf = Buffer::empty(buffer_area);
            self.paragraph.clone().render(buffer_area, &mut buf);
            self.buffer = Some(buf);
            self.buffer.as_ref().unwrap()
        }
    }

    fn invalidate_cache(&mut self) {
        self.height = None;
        self.buffer = None;
    }
}

// =============================================================================
// WIDGET CREATION FUNCTIONS
// =============================================================================

/// Creates a user message widget
fn create_user_widget(content: &str, area: Rect) -> (Box<dyn WidgetRef>, u16) {
    let borders = tuirealm::props::Borders::default();
    let block = create_block_with_title(
        format!("[ {} You ]", icons::USER_ICON),
        borders,
        false,
        Some(Padding::horizontal(1)),
    );
    let message_paragraph = Paragraph::new(trim_message_content(content).to_string())
        .block(block)
        .style(Style::new())
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: false });
    let min_height = message_paragraph.line_count(area.width.saturating_sub(4)) as u16;

    (Box::new(message_paragraph), min_height)
}

/// Creates a system message widget
fn create_system_widget(content: &str, area: Rect) -> (Box<dyn WidgetRef>, u16) {
    let borders = tuirealm::props::Borders::default();
    let block = create_block_with_title(
        format!("[ {} System ]", icons::USER_ICON),
        borders,
        false,
        Some(Padding::horizontal(1)),
    );
    let message_paragraph = Paragraph::new(trim_message_content(content).to_string())
        .block(block)
        .style(Style::new())
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: false });
    let min_height = message_paragraph.line_count(area.width.saturating_sub(4)) as u16;

    (Box::new(message_paragraph), min_height)
}

/// Creates a tool call widget with dynamic status and expansion support
fn create_tool_widget(
    tool_call: &ToolCall,
    status: Option<&ToolCallStatus>,
    area: Rect,
    is_expanded: bool,
) -> (Box<dyn WidgetRef>, u16) {
    let default_expanded_content = serde_json::to_string_pretty(&tool_call.function.arguments)
        .unwrap_or(tool_call.function.arguments.clone());

    let expand_icon = if is_expanded { "-" } else { "+" };
    let (errored, title, content, expanded_content) = match status {
        None => {
            // Tool call exists but no status update received yet - show throbber
            (
                false,
                format!("[ {} Tool: {} ⌘ ]", expand_icon, tool_call.function.name),
                "Queued for execution".to_string(),
                Some(default_expanded_content.clone()),
            )
        }
        Some(ToolCallStatus::Received { display_info }) => {
            let (content, expanded_content) = (
                display_info.collapsed.clone(),
                display_info.expanded.clone(),
            );
            // Tool is running - show throbber
            (
                false,
                format!("[ {} Tool: {} ⌘ ]", expand_icon, tool_call.function.name),
                content,
                expanded_content,
            )
        }
        Some(ToolCallStatus::AwaitingSystem { details }) => {
            let content = format!("Awaiting system: {}", details.ui_display_info.collapsed);
            // Tool is waiting - show throbber
            (
                false,
                format!("[ {} Tool: {} ⌘ ]", expand_icon, tool_call.function.name),
                content,
                details.ui_display_info.expanded.clone(),
            )
        }
        Some(ToolCallStatus::Done { result }) => {
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

            // Tool is done - no throbber
            (
                errored,
                format!("[ {} Tool: {} ]", expand_icon, tool_call.function.name),
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

    let border_color = if errored {
        Color::Red
    } else if status.is_some() {
        Color::Yellow
    } else {
        Color::default() // Gray/default for pending state
    };
    let borders = tuirealm::props::Borders::default().color(border_color);

    // Check if we need to render a throbber (if title contains ⌘)
    let maybe_throbber_pos = title.chars().position(|c| c == '⌘');

    let block =
        create_block_with_title(title.clone(), borders, false, Some(Padding::horizontal(1)));
    let p = Paragraph::new(content)
        .block(block)
        .style(Style::new())
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: false });
    let min_height = p.line_count(area.width.saturating_sub(4)) as u16;

    // If we have a throbber position, create a ThrobberParagraph
    if let Some(throbber_pos) = maybe_throbber_pos {
        // Choose throbber type based on status
        let throbber_set = match status {
            None => VERTICAL_BLOCK, // Queued - use tool execution throbber
            Some(ToolCallStatus::Received { .. }) => VERTICAL_BLOCK, // Running - use tool execution throbber
            Some(ToolCallStatus::AwaitingSystem { .. }) => OGHAM_C, // Waiting - use system wait throbber
            Some(ToolCallStatus::Done { .. }) => VERTICAL_BLOCK, // Done - shouldn't happen but fallback
        };

        let throbber_paragraph = ThrobberParagraph::new(p, throbber_pos, throbber_set);
        (Box::new(throbber_paragraph), min_height)
    } else {
        // No throbber needed, return regular paragraph
        (Box::new(p), min_height)
    }
}

/// Creates widgets for assistant messages, including text content and tool calls
fn create_assistant_widgets(
    message: &AssistantChatMessage,
    area: Rect,
    tool_call_updates: &HashMap<String, HashMap<String, ToolCallStatus>>,
    request_id: &str,
    tools_expanded: bool,
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
        let p = Paragraph::new(trim_message_content(text_content).to_string())
            .block(block)
            .style(Style::new())
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false });
        let min_height = p.line_count(area.width.saturating_sub(4)) as u16;
        widgets.push((Box::new(p), min_height));
    }

    if let Some(tool_calls) = &message.tool_calls {
        for tool_call in tool_calls {
            let status = tool_call_updates
                .get(request_id)
                .and_then(|request_updates| request_updates.get(&tool_call.id));

            // Always create a widget for the tool call, regardless of status
            widgets.push(create_tool_widget(tool_call, status, area, tools_expanded));
        }
    }

    widgets
}

// =============================================================================
// CHAT MESSAGE STATE MANAGEMENT
// =============================================================================

/// Represents a chat message with cached rendering state
struct ChatMessageWidgetState {
    message: wasmind_actor_utils::llm_client_types::ChatMessageWithRequestId,
    height: Option<u16>,
    buffer: Option<Buffer>,
    widgets: Vec<(Box<dyn WidgetRef>, u16)>,
}

impl ChatMessageWidgetState {
    fn build_widgets(
        &mut self,
        area: Rect,
        tool_call_updates: &HashMap<String, HashMap<String, ToolCallStatus>>,
        tools_expanded: bool,
    ) {
        let widgets = match &self.message {
            ChatMessageWithRequestId::System(system_msg) => {
                vec![create_system_widget(&system_msg.content, area)]
            }
            ChatMessageWithRequestId::User(user_msg) => {
                vec![create_user_widget(&user_msg.content, area)]
            }
            ChatMessageWithRequestId::Assistant(assistant_with_request_id) => {
                create_assistant_widgets(
                    &assistant_with_request_id.message,
                    area,
                    tool_call_updates,
                    &assistant_with_request_id.originating_request_id,
                    tools_expanded,
                )
            }
            ChatMessageWithRequestId::Tool(_) => vec![],
        };
        let mut total_height = 0;
        for (widget, height) in widgets.into_iter() {
            total_height += height;
            self.widgets.push((widget, height));
        }
        // Add MESSAGE_GAP between widgets if there are multiple
        if self.widgets.len() > 1 {
            total_height += (self.widgets.len() as u16 - 1) * MESSAGE_GAP;
        }
        self.height = Some(total_height);
    }
}

impl CacheableRenderItem for ChatMessageWidgetState {
    type Context<'a> = (&'a HashMap<String, HashMap<String, ToolCallStatus>>, bool); // (tool_call_updates, tools_expanded)

    fn get_height<'a>(&mut self, area: Rect, context: &Self::Context<'a>) -> u16 {
        if let Some(height) = self.height {
            height
        } else {
            self.build_widgets(area, context.0, context.1);
            self.height.unwrap()
        }
    }

    fn get_buffer<'a>(&mut self, area: Rect, context: &Self::Context<'a>) -> &Buffer {
        if self.buffer.is_some() {
            self.buffer.as_ref().unwrap()
        } else {
            let height = self.get_height(area, context);
            let mut buffer_area = area;
            buffer_area.height = height;
            buffer_area.x = 0;
            buffer_area.y = 0;
            let mut buff = Buffer::empty(buffer_area);

            let mut render_area = buffer_area;
            for (i, (widget, widget_height)) in self.widgets.iter().enumerate() {
                render_area.height = *widget_height;
                widget.render_ref(render_area, &mut buff);
                render_area.y += widget_height;
                // Add MESSAGE_GAP between widgets (but not after the last one)
                if i < self.widgets.len() - 1 {
                    render_area.y += MESSAGE_GAP;
                }
            }

            self.buffer = Some(buff);
            self.buffer.as_ref().unwrap()
        }
    }

    fn invalidate_cache(&mut self) {
        self.height = None;
        self.buffer = None;
        self.widgets.clear();
    }

    fn has_active_throbber<'a>(&self, context: &Self::Context<'a>) -> bool {
        // Check if this message contains assistant with tool calls that are loading
        if let ChatMessageWithRequestId::Assistant(assistant_with_request_id) = &self.message {
            if let Some(tool_calls) = &assistant_with_request_id.message.tool_calls {
                let tool_call_updates = context.0;
                let request_id = &assistant_with_request_id.originating_request_id;

                // Check if any tool call is in a loading state
                for tool_call in tool_calls {
                    let status = tool_call_updates
                        .get(request_id)
                        .and_then(|request_updates| request_updates.get(&tool_call.id));

                    // Tool is loading if:
                    // 1. No status (None) - queued for execution
                    // 2. Status is Received - actively running
                    // 3. Status is AwaitingSystem - waiting for system
                    match status {
                        None => return true,                                        // Queued
                        Some(ToolCallStatus::Received { .. }) => return true,       // Running
                        Some(ToolCallStatus::AwaitingSystem { .. }) => return true, // Waiting
                        Some(ToolCallStatus::Done { .. }) => {} // Completed, check next
                    }
                }
            }
        }
        false
    }
}

fn convert_from_chat_state_to_chat_message_widget_state(
    chat_state: ChatState,
) -> Vec<ChatMessageWidgetState> {
    let mut msgs = vec![];
    msgs.push(ChatMessageWidgetState {
        message: wasmind_actor_utils::llm_client_types::ChatMessageWithRequestId::System(
            chat_state.system,
        ),
        height: None,
        buffer: None,
        widgets: vec![],
    });

    for msg in chat_state.messages {
        if matches!(
            msg,
            wasmind_actor_utils::llm_client_types::ChatMessageWithRequestId::Tool(_)
        ) {
            continue;
        }
        msgs.push(ChatMessageWidgetState {
            message: msg,
            height: None,
            buffer: None,
            widgets: vec![],
        });
    }

    msgs
}

// =============================================================================
// ASSISTANT INFO AND RENDERING
// =============================================================================

/// Contains all information needed to render an assistant's chat interface
struct AssistantInfo {
    role: String,
    chat_message_widget_state: Vec<ChatMessageWidgetState>,
    pending_user_message: Option<String>,
    tool_call_updates: HashMap<String, HashMap<String, ToolCallStatus>>, // request_id -> tool_call_id -> status
    // Cached render items
    cached_title: Option<CachedParagraph>,
    cached_empty_state: Option<CachedParagraph>,
    cached_pending: Option<CachedParagraph>,
}

impl AssistantInfo {
    fn new(role: String, _task_description: Option<String>) -> Self {
        Self {
            role,
            chat_message_widget_state: vec![],
            pending_user_message: None,
            tool_call_updates: HashMap::new(),
            cached_title: None,
            cached_empty_state: None,
            cached_pending: None,
        }
    }
}

impl AssistantInfo {
    /// Invalidates all cached render items for this assistant
    fn invalidate_all_caches(&mut self) {
        if let Some(ref mut cached) = self.cached_title {
            cached.invalidate_cache();
        }
        if let Some(ref mut cached) = self.cached_empty_state {
            cached.invalidate_cache();
        }
        if let Some(ref mut cached) = self.cached_pending {
            cached.invalidate_cache();
        }

        for message in &mut self.chat_message_widget_state {
            message.invalidate_cache();
        }
    }

    fn set_pending_user_message(&mut self, message: Option<String>) {
        self.pending_user_message = message;
        self.cached_pending = None;
    }

    /// Finds and invalidates cache for message containing specific tool call
    fn invalidate_message_with_tool_call(&mut self, tool_call_id: &str) {
        for message in &mut self.chat_message_widget_state {
            // Check if this message contains the tool call
            if let wasmind_actor_utils::llm_client_types::ChatMessageWithRequestId::Assistant(
                assistant_with_request_id,
            ) = &message.message
                && let Some(tool_calls) = &assistant_with_request_id.message.tool_calls
            {
                for tool_call in tool_calls {
                    if tool_call.id == tool_call_id {
                        message.invalidate_cache();
                        return;
                    }
                }
            }
        }
    }

    /// Helper function to copy lines from message buffer to main buffer
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
                    "Source Y out of bounds: source_y={source_y}, message_buffer.height={}",
                    message_buffer.area.height
                );
                continue;
            }

            if dest_y >= buf.area.height {
                tracing::error!(
                    "Dest Y out of bounds: dest_y={dest_y}, buf.height={}",
                    buf.area.height
                );
                continue;
            }

            if let Some(src_slice) = message_buffer.content.get(source_start..source_end)
                && let Some(dst_slice) = buf.content.get_mut(dest_start..dest_end)
            {
                dst_slice.clone_from_slice(src_slice);
            } else {
                tracing::error!("Buffer copy failed: invalid slice ranges");
            }
        }
    }

    /// Helper to render an item if visible and track offsets - eliminates repetition
    fn render_and_track<'a, T: CacheableRenderItem>(
        item: &mut T,
        context: &T::Context<'a>,
        area: Rect,
        buf: &mut Buffer,
        scroll_offset: u16,
        y_offset: &mut u16,
        add_gap: bool,
    ) {
        let height = item.get_height(area, context);

        // Check visibility
        if *y_offset + height > scroll_offset && *y_offset < scroll_offset + area.height {
            let buffer = item.get_buffer(area, context);
            Self::render_item_with_clipping(buffer, height, *y_offset, scroll_offset, area, buf);
        }

        *y_offset += height;
        if add_gap {
            *y_offset += MESSAGE_GAP;
        }
    }

    // Universal clipping and rendering method for any renderable item
    fn render_item_with_clipping(
        item_buffer: &Buffer,
        item_height: u16,
        y_offset: u16,
        scroll_offset: u16,
        area: Rect,
        buf: &mut Buffer,
    ) {
        let top_clipping = scroll_offset.saturating_sub(y_offset);

        let bottom_clipping = (y_offset + item_height).saturating_sub(scroll_offset + area.height);

        let visible_height = item_height - top_clipping - bottom_clipping;

        if visible_height > 0 {
            let source_start_line = top_clipping;
            let dest_start_line = area.y + y_offset.saturating_sub(scroll_offset);

            Self::copy_buffer_lines(
                item_buffer,
                buf,
                area,
                source_start_line,
                dest_start_line,
                visible_height,
            );
        }
    }

    // This render function tracks total content height and supports scrolling
    fn render_ref_mut(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        scroll_offset: u16,
        tools_expanded: bool,
    ) -> u16 {
        // Check each message for active throbbers and invalidate their cache
        // Since throbber state is now global, we need to invalidate caches for any message with active throbbers
        for message in &mut self.chat_message_widget_state {
            if message.has_active_throbber(&(&self.tool_call_updates, tools_expanded)) {
                message.invalidate_cache();
            }
        }

        let mut y_offset = 0;

        // Create or get cached title
        if self.cached_title.is_none() {
            let title_paragraph = Paragraph::new(format!("[ {} ]", self.role))
                .style(Style::new())
                .alignment(Alignment::Center);
            self.cached_title = Some(CachedParagraph::new(title_paragraph));
        }

        // Render title using helper
        Self::render_and_track(
            self.cached_title.as_mut().unwrap(),
            &(),
            area,
            buf,
            scroll_offset,
            &mut y_offset,
            true,
        );

        if self.chat_message_widget_state.is_empty() && self.pending_user_message.is_none() {
            // Create or get cached empty state
            if self.cached_empty_state.is_none() {
                let content =
                    "It's quiet, too quiet...\n\nSend a message - don't be shy!".to_string();
                let block = Block::new()
                    .borders(Borders::ALL)
                    .padding(Padding::horizontal(1));
                let paragraph = Paragraph::new(content)
                    .alignment(Alignment::Center)
                    .wrap(Wrap { trim: false })
                    .block(block);
                self.cached_empty_state = Some(CachedParagraph::new(paragraph));
            }

            // Special handling for empty state centering
            let width = self
                .cached_empty_state
                .as_ref()
                .unwrap()
                .paragraph
                .line_width();
            let centered_area = center_horizontal(area, width as u16);

            // Adjust y_offset for centering
            y_offset += 10;

            Self::render_and_track(
                self.cached_empty_state.as_mut().unwrap(),
                &(),
                centered_area,
                buf,
                scroll_offset,
                &mut y_offset,
                false,
            );
        } else {
            // Render chat history
            let message_count = self.chat_message_widget_state.len();
            for (i, message) in self.chat_message_widget_state.iter_mut().enumerate() {
                let is_last_message = i == message_count - 1 && self.pending_user_message.is_none();

                Self::render_and_track(
                    message,
                    &(&self.tool_call_updates, tools_expanded),
                    area,
                    buf,
                    scroll_offset,
                    &mut y_offset,
                    !is_last_message,
                );
            }

            // Handle pending message using unified clipping system
            if let Some(ref pending_message) = self.pending_user_message {
                // Create or get cached pending paragraph
                if self.cached_pending.is_none() {
                    let borders = tuirealm::props::Borders::default();
                    let block = create_block_with_title(
                        format!("[ {} You - PENDING ]", icons::USER_ICON),
                        borders,
                        false,
                        Some(Padding::horizontal(1)),
                    );
                    let message_paragraph =
                        Paragraph::new(trim_message_content(pending_message).to_string())
                            .block(block)
                            .style(Style::new())
                            .alignment(Alignment::Left)
                            .wrap(Wrap { trim: false });
                    self.cached_pending = Some(CachedParagraph::new(message_paragraph));
                }

                Self::render_and_track(
                    self.cached_pending.as_mut().unwrap(),
                    &(),
                    area,
                    buf,
                    scroll_offset,
                    &mut y_offset,
                    false,
                );
            }
        }

        // Return the total content height
        y_offset
    }
}

// =============================================================================
// MAIN TUI COMPONENT
// =============================================================================

/// The main chat history component that integrates with tuirealm
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
                scroll_offset: 0,
                content_height: 0,
                last_render_area: None,
                tools_expanded: false,
            },
        }
    }

    pub fn toggle_tool_expansion(&mut self) {
        self.component.tools_expanded = !self.component.tools_expanded;
        self.component.invalidate_all_caches();
    }
}

struct ChatHistory {
    props: Props,
    state: State,
    chat_history_map: HashMap<Scope, AssistantInfo>,
    scroll_offset: u16,
    content_height: u16,
    last_render_area: Option<Rect>,
    tools_expanded: bool,
}

impl ChatHistory {
    /// Invalidates all caches for all assistants
    fn invalidate_all_caches(&mut self) {
        for assistant_info in self.chat_history_map.values_mut() {
            assistant_info.invalidate_all_caches();
        }
    }
}

impl MockComponent for ChatHistory {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true)
            && let Some(active_scope) = self.props.get(Attribute::Custom(SCOPE_ATTR))
        {
            let active_scope = active_scope.unwrap_string();
            let active_scope = Scope::from(active_scope.as_str());

            if let Some(info) = self.chat_history_map.get_mut(&active_scope) {
                // Check if we are scrolled all the way down
                let last_content_height = self.content_height;
                let is_scrolled_to_bottom = self
                    .content_height
                    .saturating_sub(self.last_render_area.map(|a| a.height).unwrap_or(0))
                    == self.scroll_offset;

                // Store render area and get total content height
                self.last_render_area = Some(area);
                self.content_height = info.render_ref_mut(
                    area,
                    frame.buffer_mut(),
                    self.scroll_offset,
                    self.tools_expanded,
                );

                if is_scrolled_to_bottom && last_content_height != self.content_height {
                    frame.render_widget(Clear, area);
                    self.scroll_offset = self
                        .content_height
                        .saturating_sub(self.last_render_area.map(|a| a.height).unwrap_or(0));
                    info.render_ref_mut(
                        area,
                        frame.buffer_mut(),
                        self.scroll_offset,
                        self.tools_expanded,
                    );
                }
            } else {
                tracing::error!("Trying to retrieve a scope that does not exist: {active_scope}");
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
        unreachable!()
    }
}

impl Component<TuiMessage, MessageEnvelope> for ChatHistoryComponent {
    fn on(&mut self, ev: Event<MessageEnvelope>) -> Option<TuiMessage> {
        match ev {
            Event::WindowResize(_, _) => {
                // Invalidate all caches since terminal width changes affect text wrapping
                self.component.invalidate_all_caches();
                Some(TuiMessage::Redraw)
            }
            Event::User(envelope) => {
                // Handle AddMessage for user input
                if let Some(add_message) = parse_common_message_as::<AddMessage>(&envelope) {
                    let scope = &add_message.agent;
                    if let Some(actor_info) = self.component.chat_history_map.get_mut(scope)
                        && let ChatMessage::User(user_msg) = add_message.message
                    {
                        actor_info.set_pending_user_message(Some(user_msg.content));
                    }
                }
                // Handle AssistantRequest to clear pending message
                else if parse_common_message_as::<AssistantRequest>(&envelope).is_some() {
                    let scope = &envelope.from_scope;
                    if let Some(actor_info) = self.component.chat_history_map.get_mut(scope) {
                        actor_info.set_pending_user_message(None);
                    }
                }
                // Handle ChatStateUpdated
                else if let Some(chat_updated) =
                    parse_common_message_as::<ChatStateUpdated>(&envelope)
                {
                    let scope = &envelope.from_scope;
                    if let Some(actor_info) = self.component.chat_history_map.get_mut(scope) {
                        actor_info.chat_message_widget_state =
                            convert_from_chat_state_to_chat_message_widget_state(
                                chat_updated.chat_state,
                            );
                    }
                }
                // Handle AddMessage if its a SystemMessage
                else if let Some(add_message) = parse_common_message_as::<AddMessage>(&envelope)
                    && let Some(actor_info) =
                        self.component.chat_history_map.get_mut(&add_message.agent)
                {
                    match add_message.message {
                        ChatMessage::System(system_message) => {
                            actor_info
                                .chat_message_widget_state
                                .push(ChatMessageWidgetState {
                                    message: ChatMessageWithRequestId::System(system_message),
                                    height: None,
                                    buffer: None,
                                    widgets: vec![],
                                });
                        }
                        ChatMessage::Tool(tool_message) => {
                            actor_info
                                .chat_message_widget_state
                                .push(ChatMessageWidgetState {
                                    message: ChatMessageWithRequestId::Tool(tool_message),
                                    height: None,
                                    buffer: None,
                                    widgets: vec![],
                                });
                        }
                        ChatMessage::Assistant(_) => (), // This will get caught almost immediatly with the ChatStateUpdated broadcast
                        ChatMessage::User(_) => (),      // This is handled by pending messages
                    }
                }
                // Handle ToolCallStatusUpdate
                else if let Some(tool_update) =
                    parse_common_message_as::<ToolCallStatusUpdate>(&envelope)
                {
                    let scope = &envelope.from_scope;
                    if let Some(actor_info) = self.component.chat_history_map.get_mut(scope) {
                        actor_info
                            .tool_call_updates
                            .entry(tool_update.originating_request_id.clone())
                            .or_insert_with(HashMap::new)
                            .insert(tool_update.id.clone(), tool_update.status);

                        // Invalidate cache for the specific message containing this tool call
                        actor_info.invalidate_message_with_tool_call(&tool_update.id);
                    }
                }
                // Handle AgentSpawned to track new agent creation
                else if let Some(agent_spawned) =
                    parse_common_message_as::<AgentSpawned>(&envelope)
                {
                    let agent_scope = agent_spawned.agent_id.clone();
                    self.component
                        .chat_history_map
                        .insert(agent_scope, AssistantInfo::new(agent_spawned.name, None));
                }
                None
            }
            Event::Mouse(mouse_event) => match mouse_event.kind {
                tuirealm::event::MouseEventKind::ScrollDown => {
                    let scroll_speed = 3;
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
                    let scroll_speed = 3;
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
