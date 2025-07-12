use crate::actors::{ActorMessage, tui::model::TuiMessage};
use crate::scope::Scope;
use ratatui::layout::{Constraint, Direction, Layout};
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};

use super::chat::ChatAreaComponent;
use super::graph::GraphAreaComponent;
use super::scrollable::ScrollableComponent;

pub const DASHBOARD_SCOPE: Scope =
    Scope::from_uuid(uuid::uuid!("00000000-0000-0000-0000-d68b0e6c4cf1"));

#[derive(MockComponent)]
pub struct DashboardComponent {
    component: Dashboard,
}

impl DashboardComponent {
    pub fn new() -> Self {
        Self {
            component: Dashboard {
                state: State::None,
                props: Props::default(),
                graph_area_component: ScrollableComponent::new(
                    Box::new(GraphAreaComponent::new()),
                    false,
                ),
                chat_area_component: ChatAreaComponent::new(),
            },
        }
    }
}

struct Dashboard {
    props: Props,
    state: State,
    graph_area_component: ScrollableComponent,
    chat_area_component: ChatAreaComponent,
}

impl MockComponent for Dashboard {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
                .split(area);
            self.graph_area_component.view(frame, chunks[0]);
            self.chat_area_component.view(frame, chunks[1]);
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

    fn perform(&mut self, _cmd: Cmd) -> CmdResult {
        unreachable!()
    }
}

impl Component<TuiMessage, ActorMessage> for DashboardComponent {
    fn on(&mut self, ev: Event<ActorMessage>) -> Option<TuiMessage> {
        // TODO: Control which one is active
        match (
            self.component.graph_area_component.on(ev.clone()),
            self.component.chat_area_component.on(ev),
        ) {
            (None, None) => None,
            (None, Some(msg)) => Some(msg),
            (Some(msg), None) => Some(msg),
            (Some(msg1), Some(msg2)) => Some(TuiMessage::Batch(vec![msg1, msg2])),
        }
    }
}
