use super::events::TuiEvent;

/// Main application state
pub struct App {
    /// All events to display
    pub events: Vec<TuiEvent>,
    /// Current input buffer
    pub input: String,
    /// Scroll offset for events
    pub scroll_offset: usize,
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
            scroll_offset: 0,
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
                self.scroll_to_bottom();
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
                self.scroll_to_bottom();
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
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_offset = (self.scroll_offset + amount).min(
            self.events.len().saturating_sub(1)
        );
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn set_waiting_for_response(&mut self, waiting: bool) {
        self.waiting_for_response = waiting;
    }

    pub fn set_waiting_for_confirmation(&mut self, waiting: bool) {
        self.waiting_for_confirmation = waiting;
    }
}