use ratatui::{
    layout::Rect,
    widgets::{Block, Padding, block::Title},
};
use tuirealm::props::Borders;

pub fn create_block<'a>(borders: Borders, _focus: bool, padding: Option<Padding>) -> Block<'a> {
    Block::default()
        .borders(borders.sides)
        .border_style(borders.style())
        .border_type(borders.modifiers)
        .padding(padding.unwrap_or(Padding::ZERO))
}

pub fn create_block_with_title<'a, T: Into<Title<'a>>>(
    title: T,
    borders: Borders,
    focus: bool,
    padding: Option<Padding>,
) -> Block<'a> {
    create_block(borders, focus, padding).title(title)
}

pub fn offset_y(rect: Rect, offset: u16) -> Rect {
    Rect {
        y: rect.y + offset,
        ..rect
    }
}
