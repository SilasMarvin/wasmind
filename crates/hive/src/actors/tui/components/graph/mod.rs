use crate::{
    actors::{ActorMessage, AgentType, tui::model::TuiMessage},
    config::ParsedTuiConfig,
    hive::{MAIN_MANAGER_ROLE, MAIN_MANAGER_SCOPE},
    scope::Scope,
};
use agent::{AgentComponent, AgentMetrics};
use ratatui::{text::Span, widgets::Paragraph};
use serde::Deserialize;
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};

use super::scrollable::ScrollableComponentTrait;

mod agent;

const LINE_INDENT: u16 = 5;

/// Actions the user can bind keys to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum GraphUserAction {
    SelectDown,
    SelectUp,
}

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

    fn increment_metrics(&mut self, scope: &Scope, metrics: AgentMetrics) -> bool {
        if self.scope() == scope {
            self.component.increment_metrics(metrics);
            return true;
        } else {
            self.spawned_agents
                .iter_mut()
                .any(|child| child.increment_metrics(scope, metrics))
        }
    }

    fn count(&self) -> u32 {
        return 1 + self
            .spawned_agents
            .iter()
            .fold(0, |acc, spa| acc + spa.count());
    }

    fn scope(&self) -> &Scope {
        &self.component.component.id
    }

    fn insert(&mut self, parent_scope: &Scope, node: AgentNode) -> Result<(), AgentNode> {
        if self.scope() == parent_scope {
            self.spawned_agents.push(Box::new(node));
            // Sort by agent type (MainManager < SubManager < Worker)
            self.spawned_agents
                .sort_by_key(|child| child.component.component.agent_type);
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
    config: ParsedTuiConfig,
}

impl GraphAreaComponent {
    pub fn new(config: ParsedTuiConfig) -> Self {
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
                content_height: 0,
                is_modified: false,
            },
            config,
        }
    }
}

struct GraphArea {
    props: Props,
    state: State,
    root_node: AgentNode,
    content_height: u16,
    is_modified: bool,
}

fn render_tree_node(
    frame: &mut Frame,
    area: Rect,
    node: &mut AgentNode,
    depth: u16,
    y_offset: &mut u16,
) {
    // SAFTEY CHECKS:
    // The Y is basically infinite as this is in Scrollable
    // The X is not so we check here.
    // TODO: Fix this
    if area.x + (depth * LINE_INDENT) + agent::WIDGET_WIDTH >= frame.area().width {
        tracing::error!(
            "The graph is to wide to show on the screen sorry :( - this will be fixed soon!"
        );
        return;
    }

    // Position for the agent box (indented based on depth)
    let box_x = area.x + (depth * LINE_INDENT);
    let box_y = area.y + *y_offset;

    // Render the agent component
    let agent_area = Rect::new(box_x, box_y, agent::WIDGET_WIDTH, agent::WIDGET_HEIGHT);
    node.component.view(frame, agent_area);

    // Update y_offset for next element
    *y_offset += agent::WIDGET_HEIGHT + 1;

    // Render children
    let child_count = node.spawned_agents.len();
    if child_count > 0 {
        // Draw vertical line from parent box to children
        let line_x = area.x + ((depth + 1) * LINE_INDENT) - 3;
        let parent_end = box_y + agent::WIDGET_HEIGHT;

        for (index, child) in node.spawned_agents.iter_mut().enumerate() {
            let is_last_child = index == child_count - 1;
            let child_y_start = *y_offset;

            // Draw vertical line from parent to this child's elbow
            let child_elbow_y = area.y + child_y_start + agent::WIDGET_HEIGHT / 2;

            // Draw vertical line from parent end down to child's elbow level
            for y in parent_end..=child_elbow_y {
                let line_widget = Paragraph::new(Span::raw("│"));
                let line_area = Rect::new(line_x, y, 1, 1);
                frame.render_widget(line_widget, line_area);
            }

            // Draw the elbow (└── or ├──) at the child's vertical center
            let branch = if is_last_child {
                "└── "
            } else {
                "├── "
            };
            let branch_widget = Paragraph::new(Span::raw(branch));
            let branch_area = Rect::new(line_x, child_elbow_y, 4, 1);
            frame.render_widget(branch_widget, branch_area);

            // Recursively render child
            render_tree_node(frame, area, child, depth + 1, y_offset);

            // For non-last children, continue vertical line below the elbow
            if !is_last_child {
                let next_child_start = *y_offset;
                for y in (child_elbow_y + 1)..(area.y + next_child_start + agent::WIDGET_HEIGHT / 2)
                {
                    let line_widget = Paragraph::new(Span::raw("│"));
                    let line_area = Rect::new(line_x, y, 1, 1);
                    frame.render_widget(line_widget, line_area);
                }
            }
        }
    }
}

impl MockComponent for GraphArea {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            let mut y_offset = 0;
            render_tree_node(frame, area, &mut self.root_node, 0, &mut y_offset);
            self.content_height = y_offset;
            self.is_modified = false;
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

impl ScrollableComponentTrait<TuiMessage, ActorMessage> for GraphAreaComponent {
    fn is_modified(&self) -> bool {
        self.component.is_modified
    }

    fn get_content_height(&self, _area: Rect) -> u16 {
        self.component.content_height
    }
}

impl Component<TuiMessage, ActorMessage> for GraphAreaComponent {
    fn on(&mut self, ev: Event<ActorMessage>) -> Option<TuiMessage> {
        let msg = match ev {
            Event::Keyboard(key_event) => None,
            Event::Mouse(mouse_event) => None,
            Event::User(actor_message) => match actor_message.message {
                crate::actors::Message::AssistantRequest(_) => {
                    let metrics = AgentMetrics::with_completion_request();
                    self.component
                        .root_node
                        .increment_metrics(&actor_message.scope, metrics);
                    Some(TuiMessage::Redraw)
                }
                crate::actors::Message::AssistantToolCall(_) => {
                    let metrics = AgentMetrics::with_tool_call();
                    self.component
                        .root_node
                        .increment_metrics(&actor_message.scope, metrics);
                    Some(TuiMessage::Redraw)
                }
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
        };

        if msg.is_some() {
            self.component.is_modified = true;
        }

        msg
    }
}
