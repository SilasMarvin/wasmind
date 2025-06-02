use super::events::{ToolExecution, TuiEvent};
use super::widgets::EventWidget;
use crate::actors::{ToolCallStatus, ToolCallType, ToolCallUpdate};
use std::collections::HashMap;

const SPLASH: &str = r#"|WELCOME USER| 

HELPER

human x ai <3
"#;

/// Main application state
pub struct App {
    /// All events to display
    pub events: Vec<TuiEvent>,
    /// Current input buffer
    pub input: String,
    /// Scroll position (0 = top of content)
    pub scroll_position: usize,
    /// Visible height of the chat area
    pub visible_height: u16,
    /// Visible width of the chat area
    pub visible_width: u16,
    /// Whether we should auto-scroll to bottom on new messages
    pub auto_scroll: bool,
    /// Whether we're waiting for assistant response
    pub waiting_for_response: bool,
    /// Whether we're waiting for command confirmation
    pub waiting_for_confirmation: bool,
    /// Cached total height to avoid recalculation
    total_height_cache: usize,
    /// Whether the cache needs to be invalidated
    cache_dirty: bool,
    /// Track ongoing tool executions by call_id
    pub tool_executions: HashMap<String, ToolExecution>,
}

impl App {
    pub fn new() -> Self {
        let mut app = Self {
            events: Vec::new(),
            input: String::new(),
            scroll_position: 0,
            visible_height: 10,
            visible_width: 80,
            auto_scroll: true,
            waiting_for_response: false,
            waiting_for_confirmation: false,
            total_height_cache: 0,
            cache_dirty: true,
            tool_executions: HashMap::new(),
        };

        // Add splash message as the first event
        app.add_event(TuiEvent::system(SPLASH.to_string()));

        app
    }

    pub fn add_event(&mut self, event: TuiEvent) {
        match &event {
            // Handle partial assistant responses by updating the last event
            TuiEvent::AssistantResponse {
                text, is_partial, ..
            } => {
                if *is_partial {
                    // Find the last assistant response and update it
                    for e in self.events.iter_mut().rev() {
                        if let TuiEvent::AssistantResponse {
                            text: existing_text,
                            is_partial: existing_partial,
                            ..
                        } = e
                        {
                            if *existing_partial {
                                *existing_text = text.clone();
                                self.cache_dirty = true;
                                return;
                            }
                        }
                    }
                }
                self.events.push(event);
                self.cache_dirty = true;
                if self.auto_scroll {
                    self.scroll_to_bottom();
                }
            }
            // Handle state changes
            TuiEvent::SetWaitingForResponse { waiting } => {
                self.waiting_for_response = *waiting;
            }
            TuiEvent::SetWaitingForConfirmation { waiting } => {
                self.waiting_for_confirmation = *waiting;
            }
            // All other events
            _ => {
                self.events.push(event);
                self.cache_dirty = true;
                if self.auto_scroll {
                    self.scroll_to_bottom();
                }
            }
        }
    }

    pub fn get_input(&self) -> &str {
        &self.input
    }

    pub fn clear_input(&mut self) {
        self.input.clear();
    }

    pub fn push_char(&mut self, c: char) {
        self.input.push(c);
    }

    pub fn pop_char(&mut self) {
        self.input.pop();
    }

    pub fn scroll_up(&mut self, amount: usize) {
        // Scrolling up means decreasing position (towards top)
        self.scroll_position = self.scroll_position.saturating_sub(amount);
        self.auto_scroll = false; // Disable auto-scroll when user scrolls manually
    }

    pub fn scroll_down(&mut self, amount: usize) {
        // Scrolling down means increasing position (towards bottom)
        let total_height = self.get_total_height();
        let max_scroll = total_height.saturating_sub(self.visible_height as usize);
        self.scroll_position = (self.scroll_position + amount).min(max_scroll);

        // Re-enable auto-scroll if we've scrolled to the bottom
        if self.scroll_position >= max_scroll {
            self.auto_scroll = true;
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        let total_height = self.get_total_height();
        self.scroll_position = total_height.saturating_sub(self.visible_height as usize);
        self.auto_scroll = true;
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll_position = 0;
        self.auto_scroll = false;
    }

    pub fn set_visible_dimensions(&mut self, width: u16, height: u16) {
        if self.visible_width != width {
            self.cache_dirty = true;
        }
        self.visible_width = width;
        self.visible_height = height;
        // Adjust scroll position if needed
        if self.auto_scroll {
            self.scroll_to_bottom();
        }
    }

    fn get_total_height(&mut self) -> usize {
        if self.cache_dirty {
            // Calculate total height from events
            let events_height: usize = self
                .events
                .iter()
                .map(|e| e.height(self.visible_width) as usize)
                .sum();

            // Calculate total height from tool executions
            let tools_height: usize = self
                .tool_executions
                .values()
                .map(|exec| {
                    use crate::actors::tui::widgets::ToolExecutionWidget;
                    let widget = ToolExecutionWidget { execution: exec };
                    widget.height(self.visible_width) as usize
                })
                .sum();

            self.total_height_cache = events_height + tools_height;
            self.cache_dirty = false;
        }
        self.total_height_cache
    }

    /// Track a tool call update
    pub fn track_tool_update(&mut self, update: ToolCallUpdate) {
        // Get or create the tool execution
        let execution = self
            .tool_executions
            .entry(update.call_id.clone())
            .or_insert_with(|| {
                // Extract name and type from the first update
                let (name, tool_type) = match &update.status {
                    ToolCallStatus::Received {
                        r#type,
                        friendly_command_display,
                    } => {
                        // Extract tool name from friendly display (e.g., "command: ls -la" -> "command")
                        let name = friendly_command_display
                            .split(':')
                            .next()
                            .unwrap_or("unknown")
                            .to_string();
                        (name, r#type.clone())
                    }
                    _ => ("unknown".to_string(), ToolCallType::MCP), // Fallback
                };
                ToolExecution::new(update.call_id.clone(), name, tool_type)
            });

        // Add the status update
        execution.add_update(update.status.clone());

        // Handle special status updates
        match &update.status {
            ToolCallStatus::AwaitingUserYNConfirmation => {
                self.waiting_for_confirmation = true;
            }
            ToolCallStatus::ReceivedUserYNConfirmation(_) => {
                self.waiting_for_confirmation = false;
            }
            _ => {}
        }

        // Mark cache as dirty to trigger redraw
        self.cache_dirty = true;
        if self.auto_scroll {
            self.scroll_to_bottom();
        }
    }
}
