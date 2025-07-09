use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::Block,
};
use tuirealm::props::Borders;

pub fn get_block<'a>(props: Borders, focus: bool) -> Block<'a> {
    Block::default()
        .borders(props.sides)
        .border_style(if focus {
            props.style()
        } else {
            Style::default().fg(Color::Reset).bg(Color::Reset)
        })
        .border_type(props.modifiers)
}

pub fn offset_y(rect: Rect, offset: u16) -> Rect {
    Rect {
        y: rect.y + offset,
        ..rect
    }
}
