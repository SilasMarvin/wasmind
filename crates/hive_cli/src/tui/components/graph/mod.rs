use crate::{
    config::ParsedTuiConfig,
    tui::{model::TuiMessage, utils::create_block_with_title},
};
use agent::{AgentComponent, AgentMetrics};
use hive::{actors::MessageEnvelope, scope::Scope, utils::parse_common_message_as};
use hive_actor_utils_common_messages::{
    actors::{AgentSpawned, Exit},
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

    /// Find the Y position of the selected node in the tree
    fn find_selected_y_position(&self, current_y: &mut u32) -> Option<u32> {
        if self.component.component.is_selected {
            return Some(*current_y + agent::WIDGET_HEIGHT as u32 / 2); // Center of the node
        }

        // Update y for this node
        *current_y += agent::WIDGET_HEIGHT as u32 + 1;

        // Check children
        for child in &self.spawned_agents {
            if let Some(selected_y) = child.find_selected_y_position(current_y) {
                return Some(selected_y);
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
                is_modified: false,
                height: 0,
                stats,
                scroll_offset: 0,
                viewport_height: 0,
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
    height: u32,
    is_modified: bool,
    stats: TotalStats,
    scroll_offset: u32,
    viewport_height: u16,
}

impl GraphArea {
    /// Centers the currently selected node in the viewport
    fn center_on_selected(&mut self) {
        // Don't center if we don't know the viewport size yet
        if self.viewport_height == 0 {
            return;
        }
        
        if let Some(ref root) = self.root_node {
            let mut y_position = 0;
            if let Some(selected_y) = root.find_selected_y_position(&mut y_position) {
                // Calculate scroll offset to center the selected node
                let center_offset = selected_y.saturating_sub(self.viewport_height as u32 / 2);

                // Ensure we don't scroll past the content bounds
                let max_offset = (self.height * (agent::WIDGET_HEIGHT as u32 + 1))
                    .saturating_sub(self.viewport_height as u32);
                self.scroll_offset = center_offset.min(max_offset);
            }
        }
    }
}

fn render_tree_node(
    frame: &mut Frame,
    area: Rect,
    node: &mut AgentNode,
    depth: u16,
    y_offset: &mut u32,
    viewport_start: u32,
    viewport_end: u32,
) -> bool {
    // Check if this node is within the viewport
    let node_start = *y_offset;
    let node_end = *y_offset + agent::WIDGET_HEIGHT as u32;
    let node_visible = node_end > viewport_start && node_start < viewport_end;

    // Safety check for horizontal bounds
    if area.x + (depth * LINE_INDENT) + agent::WIDGET_WIDTH >= frame.area().width {
        tracing::error!("The graph is too wide to show on the screen - this will be fixed soon!");
        return false;
    }

    // Position for the agent box (indented based on depth)
    let box_x = area.x + (depth * LINE_INDENT);

    // Render if the node intersects with viewport
    if node_visible {
        // The height the agent can render in
        let (start_y, agent_height) = if node_start < viewport_start {
            (
                0,
                agent::WIDGET_HEIGHT - (viewport_start - node_start) as u16,
            )
        } else if node_end > viewport_end {
            // node_start must be > viewport_start here
            (
                node_start - viewport_start,
                agent::WIDGET_HEIGHT - (node_end - viewport_end) as u16,
            )
        } else {
            (node_start - viewport_start, agent::WIDGET_HEIGHT)
        };

        // Only render if we have valid dimensions
        if agent_height > 0 && box_x + agent::WIDGET_WIDTH <= area.x + area.width {
            let agent_area = Rect::new(
                box_x,
                area.y + start_y as u16,
                agent::WIDGET_WIDTH,
                agent_height,
            );
            node.component.component.view_with_content_trim(
                frame,
                agent_area,
                node_start < viewport_start,
            );
        }
    }

    // Always update y_offset for layout calculation
    *y_offset += agent::WIDGET_HEIGHT as u32 + 1;

    // Track if the selected node was found and rendered
    let mut selected_rendered = node_visible && node.component.component.is_selected;

    // Render children
    let child_count = node.spawned_agents.len();
    if child_count > 0 {
        let line_x = area.x + ((depth + 1) * LINE_INDENT) - 3;

        // Calculate parent end position in viewport coordinates
        // let viewport_relative_y = node_start.saturating_sub(viewport_start) as u16;
        // let parent_end_viewport = area.y + viewport_relative_y + agent::WIDGET_HEIGHT;

        let parent_end_viewport =
            node_start as i32 - viewport_start as i32 + agent::WIDGET_HEIGHT as i32;
        let parent_end_viewport = if parent_end_viewport < 0 {
            0 + area.y
        } else {
            parent_end_viewport as u16 + area.y
        };

        for (index, child) in node.spawned_agents.iter_mut().enumerate() {
            let is_last_child = index == child_count - 1;
            let child_y_start = *y_offset;

            // Early exit if we're already past the viewport
            if child_y_start > viewport_end {
                return false;
            }

            // Check if child will be visible
            let child_visible = (child_y_start + agent::WIDGET_HEIGHT as u32) > viewport_start
                && child_y_start < viewport_end;

            // Draw connecting lines only if parent or child is visible
            if node_visible || child_visible {
                let child_elbow_y_absolute = child_y_start + agent::WIDGET_HEIGHT as u32 / 2;
                let child_elbow_y =
                    area.y + child_elbow_y_absolute.saturating_sub(viewport_start) as u16;

                // Draw vertical line from parent end down to child's elbow level
                if child_visible
                    && child_elbow_y >= area.y
                    && parent_end_viewport <= area.y + area.height
                {
                    let line_start = parent_end_viewport.max(area.y);
                    let line_end = child_elbow_y.min(area.y + area.height);

                    for y in line_start..line_end {
                        if y < area.y + area.height {
                            let line_widget = Paragraph::new(Span::raw("│"));
                            let line_area = Rect::new(line_x, y, 1, 1);
                            frame.render_widget(line_widget, line_area);
                        }
                    }
                }

                // Draw the elbow (└── or ├──) only if it's actually in the viewport
                if child_elbow_y_absolute >= viewport_start && child_elbow_y_absolute < viewport_end {
                    if child_elbow_y >= area.y && child_elbow_y < area.y + area.height {
                        let branch = if is_last_child {
                            "└── "
                        } else {
                            "├── "
                        };
                        let branch_widget = Paragraph::new(Span::raw(branch));
                        let branch_area = Rect::new(line_x, child_elbow_y, 4, 1);
                        frame.render_widget(branch_widget, branch_area);
                    }
                }
            }

            // Recursively render child
            let child_selected = render_tree_node(
                frame,
                area,
                child,
                depth + 1,
                y_offset,
                viewport_start,
                viewport_end,
            );
            selected_rendered = selected_rendered || child_selected;

            // Draw continuation line for non-last children
            if !is_last_child && (node_visible || child_visible) {
                let next_child_start = *y_offset;
                let current_elbow_y = area.y
                    + (child_y_start + agent::WIDGET_HEIGHT as u32 / 2)
                        .saturating_sub(viewport_start) as u16;
                let next_elbow_y = area.y
                    + (next_child_start + agent::WIDGET_HEIGHT as u32 / 2)
                        .saturating_sub(viewport_start) as u16;

                let line_start = (current_elbow_y + 1).max(area.y);
                let line_end = next_elbow_y.min(area.y + area.height);

                for y in line_start..line_end {
                    if y < area.y + area.height {
                        let line_widget = Paragraph::new(Span::raw("│"));
                        let line_area = Rect::new(line_x, y, 1, 1);
                        frame.render_widget(line_widget, line_area);
                    }
                }
            }
        }
    }

    selected_rendered
}

impl MockComponent for GraphArea {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            // Clear the entire area before rendering to prevent visual artifacts
            Clear.render(area, frame.buffer_mut());

            // Update viewport dimensions
            self.viewport_height = area.height;

            // Calculate viewport bounds
            let viewport_start = self.scroll_offset;
            let viewport_end = self.scroll_offset + area.height as u32;

            // Render the agent graph with viewport culling
            let mut y_offset = 0;
            if let Some(root) = &mut self.root_node {
                render_tree_node(
                    frame,
                    area,
                    root,
                    0,
                    &mut y_offset,
                    viewport_start,
                    viewport_end,
                );

                // Render the overall stats (always visible in top-right)
                let live_agents = root.count();
                let block = create_block_with_title(
                    "[ System Stats ]",
                    Borders::default(),
                    false,
                    Some(Padding::horizontal(1)),
                );
                let stats_paragraph = Paragraph::new(format!(
                    "Active Agents: {}\nAgents Spawned: {}\nCompletion Requests: {}\nTools Called: {}\nTokens Used: {}",
                    live_agents,
                    self.stats.agents_spawned,
                    self.stats.aggregated_agent_metrics.completion_requests_sent,
                    self.stats.aggregated_agent_metrics.tools_called,
                    self.stats.aggregated_agent_metrics.total_tokens_used
                )).block(block);
                let [mut stats_area] = Layout::horizontal([stats_paragraph.line_width() as u16])
                    .flex(Flex::End)
                    .areas(area);
                stats_area.height = stats_paragraph.line_count(stats_area.width) as u16;
                Clear.render(stats_area, frame.buffer_mut());
                frame.render_widget(stats_paragraph, stats_area);
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

                    if scope.is_some() {
                        // Center the newly selected node
                        self.component.center_on_selected();
                    }

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
                        {
                            let scope = &envelope.from_scope;
                            root.increment_metrics(scope, metrics);
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
                        {
                            let scope = &envelope.from_scope;
                            root.increment_metrics(scope, metrics);
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
                    // Get the agent scope and parent scope
                    let agent_scope = agent_spawned.agent_id.clone();
                    let parent_scope = agent_spawned.parent_agent.clone();

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
                        self.component.height += 1;
                    } else {
                        node.component.component.is_selected = true;
                        self.component.root_node = Some(node);
                        self.component.height = 1;
                        // Center on the newly created root node
                        self.component.center_on_selected();
                    }

                    Some(TuiMessage::Redraw)
                } else if let Some(agent_status_update) =
                    parse_common_message_as::<StatusUpdate>(&envelope)
                {
                    if let Some(root) = &mut self.component.root_node {
                        let agent_scope = &envelope.from_scope;
                        if matches!(agent_status_update.status, assistant::Status::Done { .. }) {
                            if let Some(scope) = root.remove(agent_scope) {
                                // Center on the newly selected node after removal
                                self.component.center_on_selected();
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
                }
                // Handle Exit messages - when an agent exits, remove it from the graph
                else if parse_common_message_as::<Exit>(&envelope).is_some() {
                    if let Some(root) = &mut self.component.root_node {
                        let agent_scope = &envelope.from_scope;
                        // Remove the agent that sent the Exit message
                        if let Some(scope) = root.remove(agent_scope) {
                            // Center on the newly selected node after removal
                            self.component.center_on_selected();
                            self.component.height -= 1;
                            Some(TuiMessage::Graph(GraphTuiMessage::SelectedAgent(
                                scope.to_string(),
                            )))
                        } else {
                            None
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
