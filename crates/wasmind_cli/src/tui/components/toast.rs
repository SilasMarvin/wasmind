use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use std::collections::VecDeque;
use std::time::Instant;
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State,
    command::{Cmd, CmdResult},
};
use wasmind::actors::MessageEnvelope;
use wasmind::utils::parse_common_message_as;
use wasmind_actor_utils::common_messages::ui::{NotificationLevel, UserNotification};

use crate::tui::model::TuiMessage;

#[derive(MockComponent)]
pub struct ToastComponent {
    component: ToastContainer,
}

struct ToastContainer {
    props: Props,
    state: State,
    toasts: VecDeque<Toast>,
}

struct Toast {
    level: NotificationLevel,
    title: String,
    message: String,
    source: Option<String>,
    created_at: Instant,
}

impl Default for ToastComponent {
    fn default() -> Self {
        Self {
            component: ToastContainer {
                props: Props::default(),
                state: State::None,
                toasts: VecDeque::new(),
            },
        }
    }
}

impl ToastComponent {
    const TOAST_TIMEOUT_MS: u64 = 5000;
    const MAX_TOASTS: usize = 5;
    const TOAST_WIDTH: u16 = 50;
    const TOAST_HEIGHT: u16 = 3;
}

impl MockComponent for ToastContainer {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        // Remove expired toasts
        let now = Instant::now();
        self.toasts.retain(|toast| {
            now.duration_since(toast.created_at).as_millis()
                < ToastComponent::TOAST_TIMEOUT_MS as u128
        });

        if self.toasts.is_empty() {
            return;
        }

        // Calculate position for toasts in top-right corner
        let toast_count = self.toasts.len().min(ToastComponent::MAX_TOASTS);
        let _total_height = toast_count as u16 * ToastComponent::TOAST_HEIGHT; // No spacing between toasts

        let start_x = area.width.saturating_sub(ToastComponent::TOAST_WIDTH + 2);
        let start_y = 1; // Leave 1 line from top

        // Render each toast
        for (i, toast) in self
            .toasts
            .iter()
            .enumerate()
            .take(ToastComponent::MAX_TOASTS)
        {
            let toast_y = start_y + (i as u16 * ToastComponent::TOAST_HEIGHT);

            // Skip if toast would go off screen
            if toast_y + ToastComponent::TOAST_HEIGHT > area.height {
                break;
            }

            let toast_area = Rect {
                x: start_x,
                y: toast_y,
                width: ToastComponent::TOAST_WIDTH,
                height: ToastComponent::TOAST_HEIGHT,
            };

            // Clear the area behind the toast
            frame.render_widget(Clear, toast_area);

            // Determine color based on notification level
            let (border_color, title_color) = match toast.level {
                NotificationLevel::Info => (Color::Blue, Color::LightBlue),
                NotificationLevel::Warning => (Color::Yellow, Color::LightYellow),
                NotificationLevel::Error => (Color::Red, Color::LightRed),
            };

            // Create the block with border and title
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .title(toast.title.clone())
                .title_style(
                    Style::default()
                        .fg(title_color)
                        .add_modifier(Modifier::BOLD),
                );

            // Format the content (for potential future use)
            let _content_text = if let Some(ref source) = toast.source {
                format!("[{}] {}: {}", source, toast.title, toast.message)
            } else {
                format!("{}: {}", toast.title, toast.message)
            };

            // Create spans for styling (title is now in block border)
            let content_spans = if let Some(ref source) = toast.source {
                vec![
                    Span::styled(format!("[{source}] "), Style::default().fg(Color::Gray)),
                    Span::styled(toast.message.clone(), Style::default().fg(Color::White)),
                ]
            } else {
                vec![Span::styled(
                    toast.message.clone(),
                    Style::default().fg(Color::White),
                )]
            };

            let paragraph = Paragraph::new(Line::from(content_spans))
                .block(block)
                .wrap(Wrap { trim: true })
                .alignment(Alignment::Left);

            frame.render_widget(paragraph, toast_area);
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

impl Component<TuiMessage, MessageEnvelope> for ToastComponent {
    fn on(&mut self, ev: Event<MessageEnvelope>) -> Option<TuiMessage> {
        match ev {
            Event::User(envelope) => {
                // Listen for UserNotification messages
                if let Some(notification) = parse_common_message_as::<UserNotification>(&envelope) {
                    // Add new toast
                    let toast = Toast {
                        level: notification.level,
                        title: notification.title,
                        message: notification.message,
                        source: notification.source,
                        created_at: Instant::now(),
                    };
                    self.component.toasts.push_front(toast); // Newest on top

                    // Limit to MAX_TOASTS
                    while self.component.toasts.len() > ToastComponent::MAX_TOASTS {
                        self.component.toasts.pop_back();
                    }

                    return Some(TuiMessage::Redraw);
                }
            }
            Event::Tick => {
                // Check if any toasts have expired and need redraw
                let now = Instant::now();
                let had_toasts = !self.component.toasts.is_empty();

                self.component.toasts.retain(|toast| {
                    now.duration_since(toast.created_at).as_millis()
                        < ToastComponent::TOAST_TIMEOUT_MS as u128
                });

                if had_toasts && self.component.toasts.is_empty() {
                    return Some(TuiMessage::Redraw);
                }
            }
            _ => {}
        }
        None
    }
}
