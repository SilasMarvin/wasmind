use crate::actors::{ActorMessage, tui::model::TuiMessage};
use ratatui::buffer::Buffer;
use std::u16;
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};

pub trait ScrollableComponentTrait<Msg, UserEvent>: Component<Msg, UserEvent>
where
    Msg: PartialEq,
    UserEvent: Eq + PartialEq + Clone + PartialOrd,
{
    fn is_modified(&self) -> bool;
    fn get_content_height(&self, area: Rect) -> u16;
}

#[derive(MockComponent)]
pub struct ScrollableComponent {
    component: Scrollable,
}

impl ScrollableComponent {
    pub fn new(
        child: Box<dyn ScrollableComponentTrait<TuiMessage, ActorMessage>>,
        auto_scroll: bool,
    ) -> Self {
        Self {
            component: Scrollable {
                props: Props::default(),
                child,
                scroll_offset: 0,
                content_height: 0,
                cached_buffer: None,
                last_render_area: None,
                auto_scroll,
            },
        }
    }
}

struct Scrollable {
    props: Props,
    child: Box<dyn ScrollableComponentTrait<TuiMessage, ActorMessage>>,
    scroll_offset: u16,
    content_height: u16,
    cached_buffer: Option<Buffer>,
    last_render_area: Option<Rect>,
    auto_scroll: bool,
}

impl MockComponent for Scrollable {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        // Check if visible
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            // Check if we need to re-render (child is modified or area changed)
            let should_rerender = self.child.is_modified()
                || self.last_render_area != Some(area)
                || self.cached_buffer.is_none();

            if should_rerender {
                // Create a large temporary buffer to render the full content
                let max_height = u16::MAX / 2; // Use half of u16::MAX for safety
                let temp_area = Rect::new(0, 0, area.width, max_height);
                let temp_buffer = Buffer::empty(temp_area);

                // Swap the frame's buffer with our temporary buffer
                let original_buffer = std::mem::replace(frame.buffer_mut(), temp_buffer);

                // Render the child component with "infinite" height
                self.child.view(frame, temp_area);

                // Get back the rendered buffer and restore the original
                let rendered_buffer = std::mem::replace(frame.buffer_mut(), original_buffer);

                // Get the actual content height from the child component
                let new_content_height = self.child.get_content_height(temp_area);

                // Check if user was at bottom before content changed
                let was_at_bottom = if let Some(old_area) = self.last_render_area {
                    let old_max_offset = self.content_height.saturating_sub(old_area.height);
                    self.scroll_offset >= old_max_offset
                } else {
                    true // If no previous area, assume at bottom
                };

                // Update content height
                self.content_height = new_content_height;

                // Auto-scroll to bottom if user was previously at bottom and content grew
                if self.auto_scroll && was_at_bottom && new_content_height > 0 {
                    let new_max_offset = self.content_height.saturating_sub(area.height);
                    self.scroll_offset = new_max_offset;
                }

                self.cached_buffer = Some(rendered_buffer);
                self.last_render_area = Some(area);
            }

            // Copy the visible portion from cached buffer to the frame
            if let Some(ref cached_buffer) = self.cached_buffer {
                let frame_buffer = frame.buffer_mut();

                // Calculate visible range
                let visible_height = area
                    .height
                    .min(self.content_height.saturating_sub(self.scroll_offset));

                // Copy visible content from cached buffer to frame buffer
                for y in 0..visible_height {
                    let source_y = self.scroll_offset + y;
                    let dest_y = area.y + y;

                    for x in 0..area.width {
                        let source_idx = (source_y as usize) * (area.width as usize) + (x as usize);
                        let dest_x = area.x + x;

                        if let Some(cell) = cached_buffer.content().get(source_idx) {
                            if let Some(dest_cell) = frame_buffer.cell_mut((dest_x, dest_y)) {
                                *dest_cell = cell.clone();
                            }
                        }
                    }
                }
            }
        }
    }

    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.child.query(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.child.attr(attr, value);
    }

    fn state(&self) -> State {
        self.child.state().clone()
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        self.child.perform(cmd)
    }
}

impl Component<TuiMessage, ActorMessage> for ScrollableComponent {
    fn on(&mut self, ev: Event<ActorMessage>) -> Option<TuiMessage> {
        match ev.clone() {
            Event::Mouse(mouse_event) => match mouse_event.kind {
                tuirealm::event::MouseEventKind::ScrollDown => {
                    // Scroll down (increase offset)
                    let scroll_speed = 3; // Lines to scroll per event
                    let max_offset = self.component.content_height.saturating_sub(
                        self.component
                            .last_render_area
                            .map(|a| a.height)
                            .unwrap_or(0),
                    );

                    self.component.scroll_offset = self
                        .component
                        .scroll_offset
                        .saturating_add(scroll_speed)
                        .min(max_offset);

                    Some(TuiMessage::Redraw)
                }
                tuirealm::event::MouseEventKind::ScrollUp => {
                    // Scroll up (decrease offset)
                    let scroll_speed = 3; // Lines to scroll per event
                    self.component.scroll_offset =
                        self.component.scroll_offset.saturating_sub(scroll_speed);

                    Some(TuiMessage::Redraw)
                }
                _ => self.component.child.on(ev),
            },
            Event::Keyboard(key_event) => {
                match key_event.code {
                    tuirealm::event::Key::Up if key_event.modifiers.is_empty() => {
                        // Scroll up by one line
                        self.component.scroll_offset =
                            self.component.scroll_offset.saturating_sub(1);
                        Some(TuiMessage::Redraw)
                    }
                    tuirealm::event::Key::Down if key_event.modifiers.is_empty() => {
                        // Scroll down by one line
                        let max_offset = self.component.content_height.saturating_sub(
                            self.component
                                .last_render_area
                                .map(|a| a.height)
                                .unwrap_or(0),
                        );

                        self.component.scroll_offset = self
                            .component
                            .scroll_offset
                            .saturating_add(1)
                            .min(max_offset);

                        Some(TuiMessage::Redraw)
                    }
                    tuirealm::event::Key::PageDown => {
                        // Scroll down by page
                        let page_size = self
                            .component
                            .last_render_area
                            .map(|a| a.height)
                            .unwrap_or(10);
                        let max_offset = self.component.content_height.saturating_sub(page_size);

                        self.component.scroll_offset = self
                            .component
                            .scroll_offset
                            .saturating_add(page_size)
                            .min(max_offset);

                        Some(TuiMessage::Redraw)
                    }
                    tuirealm::event::Key::PageUp => {
                        // Scroll up by page
                        let page_size = self
                            .component
                            .last_render_area
                            .map(|a| a.height)
                            .unwrap_or(10);
                        self.component.scroll_offset =
                            self.component.scroll_offset.saturating_sub(page_size);

                        Some(TuiMessage::Redraw)
                    }
                    tuirealm::event::Key::CtrlHome => {
                        // Jump to top
                        self.component.scroll_offset = 0;
                        Some(TuiMessage::Redraw)
                    }
                    tuirealm::event::Key::CtrlEnd => {
                        // Jump to bottom
                        let max_offset = self.component.content_height.saturating_sub(
                            self.component
                                .last_render_area
                                .map(|a| a.height)
                                .unwrap_or(0),
                        );
                        self.component.scroll_offset = max_offset;
                        Some(TuiMessage::Redraw)
                    }
                    _ => self.component.child.on(ev),
                }
            }
            _ => self.component.child.on(ev),
        }
    }
}