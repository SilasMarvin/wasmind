use crate::{
    actors::{ActorMessage, AgentType, tui::model::TuiMessage},
    hive::{MAIN_MANAGER_ROLE, MAIN_MANAGER_SCOPE},
    scope::Scope,
};
use agent::AgentComponent;
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};

mod agent;

struct AgentNode {
    is_selected: bool,
    component: AgentComponent,
    spawned_agents: Vec<Box<AgentNode>>,
}

impl AgentNode {
    fn new(component: AgentComponent) -> Self {
        Self {
            component,
            spawned_agents: vec![],
            is_selected: false,
        }
    }

    fn scope(&self) -> &Scope {
        &self.component.component.id
    }

    fn insert(&mut self, parent_scope: &Scope, node: AgentNode) -> Result<(), AgentNode> {
        if self.scope() == parent_scope {
            self.spawned_agents.push(Box::new(node));
            return Ok(());
        }

        let mut node_to_insert = node;

        for child in &mut self.spawned_agents {
            match child.insert(parent_scope, node_to_insert) {
                Ok(()) => {
                    return Ok(());
                }
                Err(returned_node) => {
                    node_to_insert = returned_node;
                }
            }
        }

        Err(node_to_insert)
    }
}

#[derive(MockComponent)]
pub struct GraphAreaComponent {
    component: GraphArea,
}

impl GraphAreaComponent {
    pub fn new() -> Self {
        Self {
            component: GraphArea {
                state: State::None,
                props: Props::default(),
                root_node: AgentNode::new(AgentComponent::new(
                    MAIN_MANAGER_SCOPE,
                    AgentType::MainManager,
                    MAIN_MANAGER_ROLE.to_string(),
                    None,
                    true,
                )),
            },
        }
    }
}

struct GraphArea {
    props: Props,
    state: State,
    root_node: AgentNode,
}

impl MockComponent for GraphArea {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            let area = Rect::new(area.x, area.y, agent::WIDGET_WIDTH, agent::WIDGET_HEIGHT);
            self.root_node.component.view(frame, area);
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

impl Component<TuiMessage, ActorMessage> for GraphAreaComponent {
    fn on(&mut self, ev: Event<ActorMessage>) -> Option<TuiMessage> {
        match ev {
            Event::Keyboard(key_event) => None,
            Event::Mouse(mouse_event) => None,
            Event::User(actor_message) => match actor_message.message {
                crate::actors::Message::AssistantRequest(assistant_request) => None,
                crate::actors::Message::ToolCallUpdate(tool_call_update) => None,
                crate::actors::Message::Agent(agent_message) => match agent_message.message {
                    crate::actors::AgentMessageType::AgentSpawned {
                        agent_type,
                        role,
                        task_description,
                        ..
                    } => {
                        let agent_component = AgentComponent::new(
                            agent_message.agent_id,
                            agent_type,
                            role,
                            Some(task_description),
                            false,
                        );
                        let node = AgentNode::new(agent_component);
                        let _ = self.component.root_node.insert(&actor_message.scope, node);

                        Some(TuiMessage::Redraw)
                    }
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        }
    }
}
