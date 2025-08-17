use ratatui::prelude::*;
use throbber_widgets_tui::Throbber;

use crate::tui::global_throbber;

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
    fn render_with_throbber(self, f: &mut Frame, area: Rect, title_len: usize, throbber: Throbber);

    /// Renders the widget with a throbber after the title to a buffer.
    ///
    /// # Arguments
    /// * `buf` - The buffer to render to
    /// * `area` - The area to render the widget in
    /// * `title_len` - The length of the title text (in characters)
    /// * `throbber` - The configured throbber widget to render
    fn render_buf_with_throbber(
        &self,
        buf: &mut Buffer,
        area: Rect,
        title_len: usize,
        throbber: Throbber,
    ) where
        Self: Clone;
}

// Implement for any widget that implements the Widget trait
impl<W: Widget> ThrobberInTitleExt for W {
    fn render_with_throbber(self, f: &mut Frame, area: Rect, title_len: usize, throbber: Throbber) {
        // First render the widget itself
        f.render_widget(self, area);

        // Calculate throbber position (after the title)
        let throbber_x = area.x + title_len as u16 + 1;
        let throbber_y = area.y;

        // Make sure we don't overflow the area
        if throbber_x < area.right().saturating_sub(1) {
            let throbber_area = Rect::new(throbber_x, throbber_y, 1, 1);
            let mut throbber_state = global_throbber::get_current_state();
            f.render_stateful_widget(throbber, throbber_area, &mut throbber_state);
        }
    }

    fn render_buf_with_throbber(
        &self,
        buf: &mut Buffer,
        area: Rect,
        title_len: usize,
        throbber: Throbber,
    ) where
        Self: Clone,
    {
        // First render the widget itself to the buffer
        self.clone().render(area, buf);

        // Calculate throbber position (after the title)
        let throbber_x = area.x + title_len as u16 + 1;
        let throbber_y = area.y;

        // Make sure we don't overflow the area
        if throbber_x < area.right().saturating_sub(1) {
            let throbber_area = Rect::new(throbber_x, throbber_y, 1, 1);
            let mut throbber_state = global_throbber::get_current_state();
            StatefulWidget::render(throbber, throbber_area, buf, &mut throbber_state);
        }
    }
}
