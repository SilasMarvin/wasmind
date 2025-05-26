use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

use super::app::App;
use super::widgets::EventWidget;

/// Main UI drawing function
pub fn draw(f: &mut Frame, app: &App) {
    // Create main layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // Chat history
            Constraint::Length(3), // Input box
        ])
        .split(f.area());

    // Draw chat history
    draw_chat_history(f, app, chunks[0]);

    // Draw input box
    draw_input(f, app, chunks[1]);
}

fn draw_chat_history(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Copilot Assistant ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::White));
    
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Calculate the layout for each event
    let mut current_y = 0;
    let visible_height = inner.height as usize;
    
    // Render events from oldest to newest
    for (_idx, event) in app.events.iter().enumerate() {
        let event_height = event.height() as usize;
        
        // Skip events that are scrolled off the top
        if current_y + event_height < app.scroll_offset {
            current_y += event_height;
            continue;
        }
        
        // Stop rendering if we've filled the visible area
        if current_y >= app.scroll_offset + visible_height {
            break;
        }
        
        // Calculate the actual render position
        let render_y = (current_y - app.scroll_offset) as u16;
        let render_height = std::cmp::min(
            event_height as u16,
            inner.height.saturating_sub(render_y)
        );
        
        if render_height > 0 {
            let event_area = Rect {
                x: inner.x,
                y: inner.y + render_y,
                width: inner.width,
                height: render_height,
            };
            
            event.render(event_area, f.buffer_mut());
        }
        
        current_y += event_height;
    }

    // Draw scrollbar if needed
    if app.events.len() > 0 {
        let total_height: usize = app.events.iter().map(|e| e.height() as usize).sum();
        if total_height > visible_height {
            let mut scrollbar_state = ScrollbarState::default()
                .content_length(total_height)
                .position(app.scroll_offset)
                .viewport_content_length(visible_height);
                
            let scrollbar = Scrollbar::default()
                .orientation(ScrollbarOrientation::VerticalRight)
                .style(Style::default().fg(Color::Gray));
                
            f.render_stateful_widget(scrollbar, inner, &mut scrollbar_state);
        }
    }
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
        Style::default().fg(Color::White)
    };

    let paragraph = Paragraph::new(input_text)
        .style(style);
    
    f.render_widget(paragraph, inner);

    // Show cursor position
    if !app.waiting_for_response {
        f.set_cursor_position((
            inner.x + app.input.len() as u16,
            inner.y,
        ));
    }
}