use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

use super::app::App;
use super::widgets::{EventWidget, ToolExecutionWidget};

/// Main UI drawing function
pub fn draw(f: &mut Frame, app: &App) {
    // Create main layout with gap
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // Chat history
            Constraint::Length(1), // Gap
            Constraint::Length(3), // Input box
        ])
        .split(f.area());

    // Draw chat history
    draw_chat_history(f, app, chunks[0]);

    // Draw input box (chunks[1] is the gap)
    draw_input(f, app, chunks[2]);
}

fn draw_chat_history(f: &mut Frame, app: &App, area: Rect) {
    let inner = area;
    let visible_height = inner.height as usize;

    // Create a combined list of renderable items (events and tool executions)
    enum RenderItem<'a> {
        Event(&'a super::events::TuiEvent),
        ToolExecution(&'a super::events::ToolExecution),
    }

    // Build a list of all items to render with their timestamps
    let mut items: Vec<(chrono::DateTime<chrono::Utc>, RenderItem)> = Vec::new();

    // Add events
    for event in &app.events {
        let timestamp = match event {
            super::events::TuiEvent::UserInput { timestamp, .. }
            | super::events::TuiEvent::UserMicrophoneInput { timestamp, .. }
            | super::events::TuiEvent::AssistantResponse { timestamp, .. }
            | super::events::TuiEvent::Screenshot { timestamp, .. }
            | super::events::TuiEvent::ClipboardCapture { timestamp, .. }
            | super::events::TuiEvent::FunctionCall { timestamp, .. }
            | super::events::TuiEvent::FunctionResult { timestamp, .. }
            | super::events::TuiEvent::CommandPrompt { timestamp, .. }
            | super::events::TuiEvent::CommandResult { timestamp, .. }
            | super::events::TuiEvent::Error { timestamp, .. }
            | super::events::TuiEvent::SystemMessage { timestamp, .. }
            | super::events::TuiEvent::TaskPlanCreated { timestamp, .. }
            | super::events::TuiEvent::TaskPlanUpdated { timestamp, .. }
            | super::events::TuiEvent::MicrophoneStarted { timestamp }
            | super::events::TuiEvent::MicrophoneStopped { timestamp } => *timestamp,
            _ => continue, // Skip non-timestamped events
        };
        items.push((timestamp, RenderItem::Event(event)));
    }

    // Add tool executions
    for execution in app.tool_executions.values() {
        items.push((execution.start_time, RenderItem::ToolExecution(execution)));
    }

    // Sort by timestamp
    items.sort_by_key(|(timestamp, _)| *timestamp);

    // Calculate total content height
    let _total_height: usize = items
        .iter()
        .map(|(_, item)| match item {
            RenderItem::Event(event) => event.height(inner.width) as usize,
            RenderItem::ToolExecution(exec) => {
                let widget = ToolExecutionWidget { execution: exec };
                widget.height(inner.width) as usize
            }
        })
        .sum();

    // Render items from oldest to newest
    let mut current_y = 0;

    for (_, item) in items.iter() {
        let item_height = match item {
            RenderItem::Event(event) => event.height(inner.width) as usize,
            RenderItem::ToolExecution(exec) => {
                let widget = ToolExecutionWidget { execution: exec };
                widget.height(inner.width) as usize
            }
        };

        // Skip items that are completely above the visible area
        if current_y + item_height <= app.scroll_position {
            current_y += item_height;
            continue;
        }

        // Skip items that are completely below the visible area
        if current_y >= app.scroll_position + visible_height {
            break;
        }

        // This item is at least partially visible

        // Calculate how many lines to skip from the top of this item
        let visible_start = app.scroll_position.saturating_sub(current_y);

        // Calculate the visible portion of this item
        let visible_lines = item_height.saturating_sub(visible_start);

        // Calculate where to render this item in the viewport
        let adjusted_y = current_y.saturating_sub(app.scroll_position);

        // Calculate the actual render height (limited by available space)
        let render_height = visible_lines.min(visible_height.saturating_sub(adjusted_y));

        if render_height > 0 {
            // The widget should only be given the height it actually needs,
            // not all the remaining space in the viewport
            let widget_height = if visible_start > 0 {
                // Widget is partially scrolled, give it only the visible portion
                render_height as u16
            } else {
                // Widget starts at or below viewport top, give it its full height
                // but limited by remaining viewport space
                (item_height as u16).min(render_height as u16)
            };

            let item_area = Rect {
                x: inner.x,
                y: inner.y + adjusted_y as u16,
                width: inner.width,
                height: widget_height,
            };

            // Render the item with proper line skipping
            match item {
                RenderItem::Event(event) => {
                    event.render_with_skip(item_area, f.buffer_mut(), visible_start);
                }
                RenderItem::ToolExecution(exec) => {
                    let widget = ToolExecutionWidget { execution: exec };
                    widget.render_with_skip(item_area, f.buffer_mut(), visible_start);
                }
            }
        }

        current_y += item_height;
    }

    // No scrollbar or indicators - clean minimal UI
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let title = if app.waiting_for_confirmation {
        " Confirm Command (y/n) "
    } else if app.waiting_for_response {
        " Waiting for Assistant... "
    } else {
        " Input (Press Enter to send) "
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Create input paragraph with cursor
    let input_text =
        if app.input.is_empty() && !app.waiting_for_response && !app.waiting_for_confirmation {
            "Type your message here...".to_string()
        } else {
            app.input.clone()
        };

    let style =
        if app.input.is_empty() && !app.waiting_for_response && !app.waiting_for_confirmation {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default()
        };

    let paragraph = Paragraph::new(input_text).style(style);

    f.render_widget(paragraph, inner);

    // Show cursor position only when focused on input
    if !app.waiting_for_response && !app.waiting_for_confirmation {
        f.set_cursor_position((inner.x + app.input.len() as u16, inner.y));
    }
}
