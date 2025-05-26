use super::events::TuiEvent;
use super::widgets::EventWidget;

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
}

impl App {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            input: String::new(),
            scroll_position: 0,
            visible_height: 10,
            visible_width: 80,
            auto_scroll: true,
            waiting_for_response: false,
            waiting_for_confirmation: false,
        }
    }

    pub fn add_event(&mut self, event: TuiEvent) {
        match &event {
            // Handle partial assistant responses by updating the last event
            TuiEvent::AssistantResponse { text, is_partial, .. } => {
                if *is_partial {
                    // Find the last assistant response and update it
                    for e in self.events.iter_mut().rev() {
                        if let TuiEvent::AssistantResponse { text: existing_text, is_partial: existing_partial, .. } = e {
                            if *existing_partial {
                                *existing_text = text.clone();
                                return;
                            }
                        }
                    }
                }
                self.events.push(event);
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
                if self.auto_scroll {
                    self.scroll_to_bottom();
                }
            }
        }
    }

    pub fn set_input(&mut self, input: String) {
        self.input = input;
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
        let total_height: usize = self.events.iter().map(|e| e.height(self.visible_width) as usize).sum();
        let max_scroll = total_height.saturating_sub(self.visible_height as usize);
        self.scroll_position = (self.scroll_position + amount).min(max_scroll);
        
        // Re-enable auto-scroll if we've scrolled to the bottom
        if self.scroll_position >= max_scroll {
            self.auto_scroll = true;
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        let total_height: usize = self.events.iter().map(|e| e.height(self.visible_width) as usize).sum();
        self.scroll_position = total_height.saturating_sub(self.visible_height as usize);
        self.auto_scroll = true;
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll_position = 0;
        self.auto_scroll = false;
    }

    pub fn set_visible_height(&mut self, height: u16) {
        self.visible_height = height;
        // Adjust scroll position if needed
        if self.auto_scroll {
            self.scroll_to_bottom();
        }
    }
    
    pub fn set_visible_dimensions(&mut self, width: u16, height: u16) {
        self.visible_width = width;
        self.visible_height = height;
        // Adjust scroll position if needed
        if self.auto_scroll {
            self.scroll_to_bottom();
        }
    }

    pub fn set_waiting_for_response(&mut self, waiting: bool) {
        self.waiting_for_response = waiting;
    }

    pub fn set_waiting_for_confirmation(&mut self, waiting: bool) {
        self.waiting_for_confirmation = waiting;
    }
}