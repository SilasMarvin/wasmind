use crate::actors::{ActorMessage, tui::model::TuiMessage};
use ratatui::buffer::Buffer;
use std::u16;
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State, StateValue,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};

pub trait ScrollableComponentTrait<Msg, UserEvent>: Component<Msg, UserEvent>
where
    Msg: PartialEq,
    UserEvent: Eq + PartialEq + Clone + PartialOrd,
{
    fn is_modified(&self) -> bool;
}

#[derive(MockComponent)]
pub struct ScrollableComponent {
    component: Scrollable,
}

impl ScrollableComponent {
    pub fn new(child: Box<dyn ScrollableComponentTrait<TuiMessage, ActorMessage>>) -> Self {
        Self {
            component: Scrollable {
                props: Props::default(),
                state: State::One(StateValue::String("".to_string())),
                child,
            },
        }
    }
}

struct Scrollable {
    props: Props,
    state: State,
    child: Box<dyn ScrollableComponentTrait<TuiMessage, ActorMessage>>,
}

impl MockComponent for Scrollable {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        // Check if visible
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            let old_buffer = frame.buffer_mut();
            let temp_buffer = Buffer::empty(Rect::new(1, 1, area.width, u16::MAX));

            self.child.view(frame, area);
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

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        self.child.perform(cmd)
    }
}

impl Component<TuiMessage, ActorMessage> for ScrollableComponent {
    fn on(&mut self, ev: Event<ActorMessage>) -> Option<TuiMessage> {
        match ev {
            Event::Mouse(mouse_event) => match mouse_event.kind {
                tuirealm::event::MouseEventKind::ScrollDown => {
                    // todo!()
                }
                tuirealm::event::MouseEventKind::ScrollUp => {
                    // todo!()
                }
                _ => (),
            },
            _ => (),
        }
        self.component.child.on(ev)
    }
}
