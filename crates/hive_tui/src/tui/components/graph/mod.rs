use crate::{
    config::ParsedTuiConfig,
    tui::{model::TuiMessage, utils::create_block_with_title},
};
use agent::{AgentComponent, AgentMetrics};
use hive::{actors::MessageEnvelope, scope::Scope, utils::parse_common_message_as};
use hive_actor_utils_common_messages::{
    actors::AgentSpawned,
    assistant::{self, Request as AssistantRequest, Status as AgentStatus, StatusUpdate},
    tools::ExecuteTool,
};
use ratatui::{
    layout::{Flex, Layout},
    text::Span,
    widgets::{Clear, Padding, Paragraph, Widget},
};
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State,
    command::{Cmd, CmdResult},
    props::Borders,
    ratatui::layout::Rect,
};

use super::scrollable::ScrollableComponentTrait;

mod agent;

const LINE_INDENT: u16 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphTuiMessage {
    SelectedAgent(String),
}

/// Actions the user can bind keys to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GraphUserAction {
    SelectDown,
    SelectUp,
}

impl GraphUserAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            GraphUserAction::SelectDown => "SelectDown",
            GraphUserAction::SelectUp => "SelectUp",
        }
    }
}

impl TryFrom<&str> for GraphUserAction {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "SelectDown" => Ok(GraphUserAction::SelectDown),
            "SelectUp" => Ok(GraphUserAction::SelectUp),
            _ => Err(()),
        }
    }
}

struct AgentNode {
    component: AgentComponent,
    spawned_agents: Vec<Box<AgentNode>>,
}

impl AgentNode {
    fn new(component: AgentComponent) -> Self {
        Self {
            component,
            spawned_agents: vec![],
        }
    }

    /// Returns the Scope of the newly selected node, or None if no next node available
    fn select_next(&mut self) -> Option<Scope> {
        if self.component.component.is_selected {
            if let Some(child) = self.spawned_agents.first_mut() {
                child.component.component.is_selected = true;
                self.component.component.is_selected = false;
                Some(child.scope().clone())
            } else {
                None
            }
        } else {
            let spawned_agent_len = self.spawned_agents.len();
            for (index, agent) in self.spawned_agents.iter_mut().enumerate() {
                // First check if this child contains the selected node
                if agent.contains_selected() {
                    // Try to select next within this child
                    if let Some(selected_scope) = agent.select_next() {
                        return Some(selected_scope);
                    } else {
                        // Child contains selected but couldn't go next, try next sibling
                        if spawned_agent_len > index + 1 {
                            agent.clear_selected();
                            self.spawned_agents[index + 1]
                                .component
                                .component
                                .is_selected = true;
                            return Some(self.spawned_agents[index + 1].scope().clone());
                        } else {
                            return None;
                        }
                    }
                }
            }
            None
        }
    }

    /// Returns the Scope of the newly selected node, or None if no previous node available
    fn select_previous(&mut self) -> Option<Scope> {
        // First, check if any child contains the selected node
        for (index, agent) in self.spawned_agents.iter_mut().enumerate() {
            if agent.contains_selected() {
                // Try to select previous within this child
                if let Some(selected_scope) = agent.select_previous() {
                    return Some(selected_scope);
                } else {
                    // Child contains selected but couldn't go previous
                    if index > 0 {
                        // Move to the previous sibling's last descendant
                        agent.clear_selected();
                        let prev_sibling = &mut self.spawned_agents[index - 1];
                        return Some(prev_sibling.select_last());
                    } else {
                        // This is the first child, so select the parent (self)
                        agent.clear_selected();
                        self.component.component.is_selected = true;
                        return Some(self.scope().clone());
                    }
                }
            }
        }

        // If no child contains the selected node, check if self is selected
        if self.component.component.is_selected {
            // Can't go previous from here
            None
        } else {
            // Selected node is not in this subtree
            None
        }
    }

    /// Helper function to check if this subtree contains the selected node
    fn contains_selected(&self) -> bool {
        if self.component.component.is_selected {
            return true;
        }
        self.spawned_agents
            .iter()
            .any(|child| child.contains_selected())
    }

    /// Helper function to select the last descendant in the subtree and return its Scope
    fn select_last(&mut self) -> Scope {
        if let Some(last_child) = self.spawned_agents.last_mut() {
            last_child.select_last()
        } else {
            self.component.component.is_selected = true;
            self.scope().clone()
        }
    }

    fn clear_selected(&mut self) {
        self.component.component.is_selected = false;
        for agent in &mut self.spawned_agents {
            agent.clear_selected();
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

    fn set_status(&mut self, scope: &Scope, status: &AgentStatus) -> bool {
        if self.scope() == scope {
            self.component.set_status(status.clone());
            return true;
        } else {
            self.spawned_agents
                .iter_mut()
                .any(|agent| agent.set_status(scope, status))
        }
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

    /// Removes the node with the given scope and selects the previous node.
    /// Returns the scope of the newly selected node, or None if no previous node exists.
    fn remove(&mut self, scope: &Scope) -> Option<Scope> {
        // Check if we need to remove self
        if self.scope() == scope {
            // Can't remove self from within the method
            // This case should be handled by the parent
            return None;
        }

        // Find and remove the child with the matching scope
        let mut child_index = None;
        for (index, child) in self.spawned_agents.iter().enumerate() {
            if child.scope() == scope {
                child_index = Some(index);
                break;
            }
        }

        if let Some(index) = child_index {
            // Remove the child
            let removed_child = self.spawned_agents.remove(index);

            // Determine what to select next
            if removed_child.component.component.is_selected {
                // The removed node was selected, so select previous
                if index > 0 {
                    // Select the last descendant of the previous sibling
                    let prev_sibling = &mut self.spawned_agents[index - 1];
                    return Some(prev_sibling.select_last());
                } else {
                    // No previous sibling, select parent (self)
                    self.component.component.is_selected = true;
                    return Some(self.scope().clone());
                }
            } else {
                // The removed node wasn't selected, so no selection change needed
                // Return the currently selected node's scope if any
                return self.find_selected_scope();
            }
        }

        // Try to remove from children
        for (index, child) in self.spawned_agents.iter_mut().enumerate() {
            if child.contains_scope(scope) {
                if let Some(new_selection) = child.remove(scope) {
                    return Some(new_selection);
                } else {
                    // Child couldn't handle the removal (trying to remove itself)
                    // Remove this child and handle selection
                    let removed_child = self.spawned_agents.remove(index);

                    if removed_child.contains_selected() {
                        // Need to select previous
                        if index > 0 {
                            let prev_sibling = &mut self.spawned_agents[index - 1];
                            return Some(prev_sibling.select_last());
                        } else {
                            self.component.component.is_selected = true;
                            return Some(self.scope().clone());
                        }
                    } else {
                        return self.find_selected_scope();
                    }
                }
            }
        }

        None
    }

    /// Helper function to check if this subtree contains a node with the given scope
    fn contains_scope(&self, scope: &Scope) -> bool {
        if self.scope() == scope {
            return true;
        }
        self.spawned_agents
            .iter()
            .any(|child| child.contains_scope(scope))
    }

    /// Helper function to find and return the scope of the currently selected node
    fn find_selected_scope(&self) -> Option<Scope> {
        if self.component.component.is_selected {
            return Some(self.scope().clone());
        }

        for child in &self.spawned_agents {
            if let Some(selected_scope) = child.find_selected_scope() {
                return Some(selected_scope);
            }
        }

        None
    }
}

#[derive(MockComponent)]
pub struct GraphAreaComponent {
    component: GraphArea,
    config: ParsedTuiConfig,
}

impl GraphAreaComponent {
    pub fn new(config: ParsedTuiConfig) -> Self {
        let mut stats = TotalStats::default();
        stats.agents_spawned += 1;
        Self {
            component: GraphArea {
                state: State::None,
                props: Props::default(),
                root_node: None,
                content_height: 0,
                is_modified: false,
                stats,
            },
            config,
        }
    }
}

#[derive(Default)]
struct TotalStats {
    agents_spawned: u64,
    aggregated_agent_metrics: AgentMetrics,
}

struct GraphArea {
    props: Props,
    state: State,
    root_node: Option<AgentNode>,
    content_height: u16,
    is_modified: bool,
    stats: TotalStats,
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
            // Render the agent graph
            let mut y_offset = 0;
            if let Some(root) = &mut self.root_node {
                render_tree_node(frame, area, root, 0, &mut y_offset);
                self.content_height = y_offset;

                // Render the overall stats
                let live_agents = root.count();
                let block = create_block_with_title(
                    "[ System Stats ]",
                    Borders::default(),
                    false,
                    Some(Padding::uniform(1)),
                );
                let stats_paragraph = Paragraph::new(format!(
                    "Active Agents: {}\nAgents Spawned: {}\nCompletion Requests: {}\nTools Called: {}\nTokens Used: {}",
                    live_agents,
                    self.stats.agents_spawned,
                    self.stats.aggregated_agent_metrics.completion_requests_sent,
                    self.stats.aggregated_agent_metrics.tools_called,
                    self.stats.aggregated_agent_metrics.total_tokens_used
                )).block(block);
                let [mut area] = Layout::horizontal([stats_paragraph.line_width() as u16])
                    .flex(Flex::End)
                    .areas(area);
                area.height = stats_paragraph.line_count(area.width) as u16;
                Clear.render(area, frame.buffer_mut());
                frame.render_widget(stats_paragraph, area);
            }

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

impl ScrollableComponentTrait<TuiMessage, MessageEnvelope> for GraphAreaComponent {
    fn is_modified(&self) -> bool {
        self.component.is_modified
    }

    fn get_content_height(&self, _area: Rect) -> u16 {
        self.component.content_height
    }
}

impl Component<TuiMessage, MessageEnvelope> for GraphAreaComponent {
    fn on(&mut self, ev: Event<MessageEnvelope>) -> Option<TuiMessage> {
        let msg = match ev {
            Event::Tick => {
                self.component.is_modified = true;
                Some(TuiMessage::Redraw)
            }
            Event::Keyboard(key_event) => {
                if let Some(action) = self.config.graph.key_bindings.get(&key_event) {
                    let scope = match action {
                        GraphUserAction::SelectDown => self
                            .component
                            .root_node
                            .as_mut()
                            .map(|root| root.select_next())
                            .flatten(),
                        GraphUserAction::SelectUp => self
                            .component
                            .root_node
                            .as_mut()
                            .map(|root| root.select_previous())
                            .flatten(),
                    };

                    scope.and_then(|scope| {
                        Some(TuiMessage::Graph(GraphTuiMessage::SelectedAgent(
                            scope.to_string(),
                        )))
                    })
                } else {
                    None
                }
            }
            Event::User(envelope) => {
                // Handle AssistantRequest messages
                if let Some(_) = parse_common_message_as::<AssistantRequest>(&envelope) {
                    if let Some(root) = &mut self.component.root_node {
                        let metrics = AgentMetrics::with_completion_request();
                        self.component.stats.aggregated_agent_metrics += metrics;
                        if let Ok(scope) = envelope.from_scope.parse::<Scope>() {
                            root.increment_metrics(&scope, metrics);
                        }
                        Some(TuiMessage::Redraw)
                    } else {
                        None
                    }
                }
                // Handle AssistantToolCall messages
                else if let Some(_) = parse_common_message_as::<ExecuteTool>(&envelope) {
                    if let Some(root) = &mut self.component.root_node {
                        let metrics = AgentMetrics::with_tool_call();
                        self.component.stats.aggregated_agent_metrics += metrics;
                        if let Ok(scope) = envelope.from_scope.parse::<Scope>() {
                            root.increment_metrics(&scope, metrics);
                        }
                        Some(TuiMessage::Redraw)
                    } else {
                        None
                    }
                }
                // Handle AgentSpawned messages
                else if let Some(agent_spawned) =
                    parse_common_message_as::<AgentSpawned>(&envelope)
                {
                    // Parse the agent scope and parent scope
                    if let Ok(agent_scope) = agent_spawned.agent_id.parse::<Scope>() {
                        let parent_scope = agent_spawned
                            .parent_agent
                            .and_then(|p| p.parse::<Scope>().ok());

                        // Create new agent component
                        let agent_component = AgentComponent::new(
                            agent_scope,
                            agent_spawned.name,
                            agent_spawned.actors,
                            false,
                        );
                        let mut node = AgentNode::new(agent_component);

                        if let Some(root) = &mut self.component.root_node
                            && let Some(parent_scope) = parent_scope
                        {
                            // Insert into the tree at the parent scope
                            let _ = root.insert(&parent_scope, node);
                            self.component.stats.agents_spawned += 1;
                        } else {
                            node.component.component.is_selected = true;
                            self.component.root_node = Some(node);
                        }

                        Some(TuiMessage::Redraw)
                    } else {
                        tracing::error!("Failed to parse agent scope: {}", agent_spawned.agent_id);
                        None
                    }
                } else if let Some(agent_status_update) =
                    parse_common_message_as::<StatusUpdate>(&envelope)
                {
                    if let Some(root) = &mut self.component.root_node
                        && let Ok(agent_scope) = envelope.from_scope.parse::<Scope>()
                    {
                        if matches!(agent_status_update.status, assistant::Status::Done { .. }) {
                            if let Some(scope) = root.remove(&agent_scope) {
                                Some(TuiMessage::Graph(GraphTuiMessage::SelectedAgent(
                                    scope.to_string(),
                                )))
                            } else {
                                None
                            }
                        } else {
                            root.set_status(&agent_scope, &agent_status_update.status);
                            Some(TuiMessage::Redraw)
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        };

        if msg.is_some() {
            self.component.is_modified = true;
        }

        msg
    }
}
