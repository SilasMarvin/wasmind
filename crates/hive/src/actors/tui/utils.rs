use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    widgets::{Block, Padding, block::Title},
};
use throbber_widgets_tui::{Throbber, ThrobberState};
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

// Center both horizontally and vertically. See: https://ratatui.rs/recipes/layout/center-a-widget/
pub fn center(area: Rect, horizontal: Constraint, vertical: Constraint) -> Rect {
    let [area] = Layout::horizontal([horizontal])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([vertical]).flex(Flex::Center).areas(area);
    area
}

// Center horizontally: See https://ratatui.rs/recipes/layout/center-a-widget/
pub fn center_horizontal(area: Rect, width: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    area
}
