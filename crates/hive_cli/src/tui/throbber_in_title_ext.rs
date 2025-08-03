use ratatui::prelude::*;
use throbber_widgets_tui::{Throbber, ThrobberState};

/// Extension trait for rendering widgets with an animated throbber in the title.
///
/// This trait allows any ratatui widget to be rendered with a throbber animation
/// after the title text.
pub trait ThrobberInTitleExt: Sized {
    /// Renders the widget with a throbber after the title.
    ///
    /// # Arguments
    /// * `f` - The frame to render to
    /// * `area` - The area to render the widget in
    /// * `title_len` - The length of the title text (in characters)
    /// * `throbber` - The configured throbber widget to render
    /// * `throbber_state` - The state for the throbber animation
    fn render_with_throbber(
        self,
        f: &mut Frame,
        area: Rect,
        title_len: usize,
        throbber: Throbber,
        throbber_state: &mut ThrobberState,
    );
}

// Implement for any widget that implements the Widget trait
impl<W: Widget> ThrobberInTitleExt for W {
    fn render_with_throbber(
        self,
        f: &mut Frame,
        area: Rect,
        title_len: usize,
        throbber: Throbber,
        throbber_state: &mut ThrobberState,
    ) {
        // First render the widget itself
        f.render_widget(self, area);

        // Calculate throbber position (after the title)
        let throbber_x = area.x + title_len as u16 + 1;
        let throbber_y = area.y;

        // Make sure we don't overflow the area
        if throbber_x < area.right().saturating_sub(1) {
            let throbber_area = Rect::new(throbber_x, throbber_y, 1, 1);
            f.render_stateful_widget(throbber, throbber_area, throbber_state);
        }
    }
}
