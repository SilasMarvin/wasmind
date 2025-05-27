use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::app::App;
use super::widgets::EventWidget;

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
    
    // Update the visible height in the app if it changed
    if inner.height != app.visible_height {
        // We can't modify app here, but we'll handle this in the main loop
    }
    
    // Calculate total content height
    let _total_height: usize = app.events.iter().map(|e| e.height(inner.width) as usize).sum();
    
    // Render events from oldest to newest
    let mut current_y = 0;
    
    for event in app.events.iter() {
        let event_height = event.height(inner.width) as usize;
        
        // Skip events that are completely above the visible area
        if current_y + event_height <= app.scroll_position {
            current_y += event_height;
            continue;
        }
        
        // Skip events that are completely below the visible area
        if current_y >= app.scroll_position + visible_height {
            break;
        }
        
        // This event is at least partially visible
        
        // Calculate how many lines to skip from the top of this event
        let visible_start = app.scroll_position.saturating_sub(current_y);
        
        // Calculate the visible portion of this event
        let visible_lines = event_height.saturating_sub(visible_start);
        
        // Calculate where to render this event in the viewport
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
                (event_height as u16).min(render_height as u16)
            };
            
            let event_area = Rect {
                x: inner.x,
                y: inner.y + adjusted_y as u16,
                width: inner.width,
                height: widget_height,
            };
            
            // Render the event with proper line skipping
            event.render_with_skip(event_area, f.buffer_mut(), visible_start);
        }
        
        current_y += event_height;
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
    let input_text = if app.input.is_empty() && !app.waiting_for_response && !app.waiting_for_confirmation {
        "Type your message here...".to_string()
    } else {
        app.input.clone()
    };

    let style = if app.input.is_empty() && !app.waiting_for_response && !app.waiting_for_confirmation {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
    };

    let paragraph = Paragraph::new(input_text)
        .style(style);
    
    f.render_widget(paragraph, inner);

    // Show cursor position only when focused on input
    if !app.waiting_for_response && !app.waiting_for_confirmation {
        f.set_cursor_position((
            inner.x + app.input.len() as u16,
            inner.y,
        ));
    }
}