use crate::{
    actors::{ActorMessage, AgentType, tui::model::TuiMessage},
    scope::Scope,
};
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};

#[derive(Default)]
struct AgentStats {
    requests_sent: u64,
    tools_called: u64,
}

#[derive(MockComponent)]
pub struct AgentComponent {
    pub component: Agent,
}

impl AgentComponent {
    pub fn new(id: Scope, agent_type: AgentType, role: impl ToString) -> Self {
        Self {
            component: Agent {
                id,
                agent_type,
                role: role.to_string(),
                stats: AgentStats::default(),
                state: State::None,
                props: Props::default(),
            },
        }
    }
}

pub struct Agent {
    pub id: Scope,
    stats: AgentStats,
    props: Props,
    state: State,
    agent_type: AgentType,
    role: String,
}

impl MockComponent for Agent {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {}
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

impl Component<TuiMessage, ActorMessage> for AgentComponent {
    fn on(&mut self, ev: Event<ActorMessage>) -> Option<TuiMessage> {
        None
    }
}
