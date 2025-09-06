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
use wasmind::{actors::MessageEnvelope, scope::Scope, utils::parse_common_message_as};
use wasmind_actor_utils::common_messages::{
    actors::{AgentSpawned, Exit},
    assistant::{
        Request as AssistantRequest, Response as AssistantResponse, Status as AgentStatus,
        StatusUpdate,
    },
    tools::ExecuteTool,
};

use crate::{
    config::ParsedTuiConfig,
    tui::{model::TuiMessage, utils::create_block_with_title},
};

mod agent;

use agent::{AgentComponent, AgentMetrics};

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
    spawned_agents: Vec<AgentNode>,
}

impl AgentNode {
    /// Creates a new AgentNode.
    fn new(component: AgentComponent) -> Self {
        Self {
            component,
            spawned_agents: vec![],
        }
    }

    // --- Private Helper Functions for State and Identification ---

    fn scope(&self) -> &Scope {
        &self.component.component.id
    }
    fn is_selected(&self) -> bool {
        self.component.component.is_selected
    }
    fn select(&mut self) {
        self.component.component.is_selected = true;
    }
    fn unselect(&mut self) {
        self.component.component.is_selected = false;
    }

    // --- Core Tree Traversal Helpers ---

    /// Recursively finds a mutable reference to a node with the given scope.
    fn find_mut(&mut self, scope: &Scope) -> Option<&mut AgentNode> {
        if self.scope() == scope {
            return Some(self);
        }
        self.spawned_agents
            .iter_mut()
            .find_map(|child| child.find_mut(scope))
    }

    /// Recursively finds an immutable reference to the currently selected node.
    fn find_selected(&self) -> Option<&AgentNode> {
        if self.is_selected() {
            return Some(self);
        }
        self.spawned_agents
            .iter()
            .find_map(|child| child.find_selected())
    }

    /// Builds a flattened, depth-first list of immutable references to node Scopes.
    /// This is the core of the safe navigation strategy.
    fn get_scopes_in_order<'a>(&'a self, scopes: &mut Vec<&'a Scope>) {
        scopes.push(self.scope());
        for child in &self.spawned_agents {
            child.get_scopes_in_order(scopes);
        }
    }

    // --- Public API ---

    /// Finds the currently selected agent and returns its scope and actors.
    fn find_selected_agent_info(&self) -> Option<(Scope, Vec<String>)> {
        self.find_selected().map(|node| {
            (
                node.scope().clone(),
                node.component.component.actors.clone(),
            )
        })
    }

    /// Selects the next node in a depth-first traversal order.
    fn select_next(&mut self) -> Option<Scope> {
        // 1. Find the scope of the currently selected node.
        let current_scope = self.find_selected()?.scope().clone();

        // 2. Get a safe, ordered list of all scope references.
        let mut scopes = Vec::new();
        self.get_scopes_in_order(&mut scopes);

        // 3. Find the position of the current node and determine the next scope.
        let current_pos = scopes.iter().position(|s| **s == current_scope)?;
        let next_scope_ref = scopes.get(current_pos + 1)?;
        let next_scope = (*next_scope_ref).clone();

        // 4. Perform the state change with two separate, non-conflicting traversals.
        if let Some(current_node) = self.find_mut(&current_scope) {
            current_node.unselect();
        }
        if let Some(next_node) = self.find_mut(&next_scope) {
            next_node.select();
            Some(next_node.scope().clone())
        } else {
            None // Should be unreachable if the scopes list is correct
        }
    }

    /// Selects the previous node in a depth-first traversal order.
    fn select_previous(&mut self) -> Option<Scope> {
        let current_scope = self.find_selected()?.scope().clone();

        let prev_scope = {
            let mut scopes = Vec::new();
            self.get_scopes_in_order(&mut scopes);

            let current_pos = scopes.iter().position(|s| **s == current_scope)?;
            if current_pos == 0 {
                return None;
            } // Already at the first node
            scopes.get(current_pos - 1)?.to_string()
        };

        if let Some(current_node) = self.find_mut(&current_scope) {
            current_node.unselect();
        }
        if let Some(prev_node) = self.find_mut(&prev_scope) {
            prev_node.select();
            Some(prev_node.scope().clone())
        } else {
            None
        }
    }

    /// Inserts a new node under the given parent. Returns `true` on success.
    #[allow(clippy::result_large_err)]
    fn insert(&mut self, parent_scope: &Scope, node: AgentNode) -> Result<(), AgentNode> {
        if let Some(parent_node) = self.find_mut(parent_scope) {
            parent_node.spawned_agents.push(node);
            Ok(())
        } else {
            Err(node)
        }
    }

    /// Removes the node with the given scope and selects the previous node.
    /// Returns the scope of the newly selected node if the removed node was selected.
    fn remove(&mut self, scope_to_remove: &Scope) -> Option<Scope> {
        if self.scope() == scope_to_remove {
            return None;
        } // Cannot remove the root

        // 1. Get the ordered list of scope references BEFORE mutation.
        let mut scopes = Vec::new();
        self.get_scopes_in_order(&mut scopes);

        // 2. Find the position of the node to remove.
        let removal_pos = scopes.iter().position(|s| *s == scope_to_remove)?;

        // 3. Determine which scope to select after removal (the previous one).
        // Must clone here because the original reference will be invalidated by the removal.
        let new_scope_to_select = if removal_pos > 0 {
            scopes[removal_pos - 1].clone()
        } else {
            self.scope().clone()
        };

        // 4. Perform the actual removal via a recursive helper.
        let removed_node = self.remove_recursive(scope_to_remove)?;

        // 5. If the removed node was the selected one, update the selection.
        if removed_node.is_selected() {
            if let Some(newly_selected_node) = self.find_mut(&new_scope_to_select) {
                newly_selected_node.select();
                return Some(new_scope_to_select);
            }
        }

        None
    }

    /// Recursive helper to find and remove a child node. Returns the removed node.
    fn remove_recursive(&mut self, scope: &Scope) -> Option<AgentNode> {
        if let Some(index) = self.spawned_agents.iter().position(|c| c.scope() == scope) {
            return Some(self.spawned_agents.remove(index));
        }
        self.spawned_agents
            .iter_mut()
            .find_map(|child| child.remove_recursive(scope))
    }

    /// Sets the status for the agent with the given scope. Returns `true` on success.
    fn set_status(&mut self, scope: &Scope, status: AgentStatus) -> bool {
        if let Some(node) = self.find_mut(scope) {
            node.component.set_status(status);
            true
        } else {
            false
        }
    }

    /// Increments metrics for the agent with the given scope. Returns `true` on success.
    fn increment_metrics(&mut self, scope: &Scope, metrics: AgentMetrics) -> bool {
        if let Some(node) = self.find_mut(scope) {
            node.component.increment_metrics(metrics);
            true
        } else {
            false
        }
    }

    /// Counts the total number of nodes in this subtree.
    fn count(&self) -> u32 {
        1 + self.spawned_agents.iter().map(|c| c.count()).sum::<u32>()
    }

    /// Finds the Y position of the selected node in the tree for rendering.
    fn find_selected_y_position(&self) -> Option<u32> {
        let mut scopes = Vec::new();
        self.get_scopes_in_order(&mut scopes);

        let selected_scope = self.find_selected()?.scope();

        let position = scopes.iter().position(|s| *s == selected_scope)?;

        let center_offset = agent::WIDGET_HEIGHT / 2;

        Some(
            (position as u32 * agent::WIDGET_HEIGHT as u32)
                + center_offset as u32
                + position as u32,
        )
    }
}

pub struct GraphAreaComponent {
    component: GraphArea,
    config: ParsedTuiConfig,
    nodes_to_insert: Vec<(Scope, AgentNode)>,
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
                height: 0,
                stats,
                scroll_offset: 0,
                viewport_height: 0,
            },
            config,
            nodes_to_insert: vec![],
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
            if let Some(selected_y) = root.find_selected_y_position() {
                // Calculate scroll offset to center the selected node
                let center_offset = selected_y.saturating_sub(self.viewport_height as u32 / 2);

                // Ensure we don't scroll past the content bounds
                let max_offset = (self.height * (agent::WIDGET_HEIGHT as u32 + 1))
                    .saturating_sub(self.viewport_height as u32);
                self.scroll_offset = center_offset.min(max_offset);
            }
        }
    }

    /// Formats key bindings for display in the keybindings box
    fn format_keybindings(&self, config: &ParsedTuiConfig) -> String {
        use crate::utils::key_event_to_string;

        let mut bindings = Vec::new();

        for (key_event, action) in &config.graph.key_bindings {
            // Format the key combination to be user-friendly
            let key_str = key_event_to_string(key_event);

            // Format the action to be user-friendly
            let action_str = match action {
                GraphUserAction::SelectUp => "Select Up",
                GraphUserAction::SelectDown => "Select Down",
            };

            bindings.push(format!("{key_str}: {action_str}"));
        }

        if bindings.is_empty() {
            "No key bindings configured".to_string()
        } else {
            bindings.join("\n")
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
            area.y
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
                if child_elbow_y_absolute >= viewport_start
                    && child_elbow_y_absolute < viewport_end
                    && child_elbow_y >= area.y
                    && child_elbow_y < area.y + area.height
                {
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

            // Use full area for tree rendering
            let tree_area = area;

            // Update viewport dimensions for the tree area
            self.viewport_height = tree_area.height;

            // Calculate viewport bounds
            let viewport_start = self.scroll_offset;
            let viewport_end = self.scroll_offset + tree_area.height as u32;

            // Render the agent graph with viewport culling in the tree area
            let mut y_offset = 0;
            if let Some(root) = &mut self.root_node {
                render_tree_node(
                    frame,
                    tree_area,
                    root,
                    0,
                    &mut y_offset,
                    viewport_start,
                    viewport_end,
                );

                // Render the overall stats (always visible in top-right of tree area)
                let live_agents = root.count();

                // Get selected agent's actors
                let selected_section =
                    if let Some((scope, actors)) = root.find_selected_agent_info() {
                        let mut section = format!("-----\nSelected: {scope}");
                        if actors.is_empty() {
                            section.push_str("\nActors: none");
                        } else {
                            section.push_str("\nActors:");
                            for actor in &actors {
                                section.push_str(&format!("\n  {actor}"));
                            }
                        }
                        section
                    } else {
                        "-----\nNo agent selected".to_string()
                    };

                let block = create_block_with_title(
                    "[ System Stats ]",
                    Borders::default(),
                    false,
                    Some(Padding::horizontal(1)),
                );
                let stats_paragraph = Paragraph::new(format!(
                    "Active Agents: {}\nAgents Spawned: {}\nCompletion Requests: {}\nTools Called: {}\nTokens Used: {}\n  Prompt: {}\n  Completion: {}\n{}", 
                    live_agents,
                    self.stats.agents_spawned,
                    self.stats.aggregated_agent_metrics.completion_requests_sent,
                    self.stats.aggregated_agent_metrics.tools_called,
                    self.stats.aggregated_agent_metrics.total_tokens_used,
                    self.stats.aggregated_agent_metrics.prompt_tokens_used,
                    self.stats.aggregated_agent_metrics.completion_tokens_used,
                    selected_section
                )).block(block);
                let [mut stats_area] = Layout::horizontal([stats_paragraph.line_width() as u16])
                    .flex(Flex::End)
                    .areas(tree_area);
                stats_area.height = stats_paragraph.line_count(stats_area.width) as u16;
                Clear.render(stats_area, frame.buffer_mut());
                frame.render_widget(stats_paragraph, stats_area);
            }
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

impl MockComponent for GraphAreaComponent {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        // First render the main graph view
        self.component.view(frame, area);

        // Then render the keybindings box at bottom-right if we have a root node
        if self.component.root_node.is_some() {
            let keybindings_content = self.component.format_keybindings(&self.config);
            let block = create_block_with_title(
                "[ Key Bindings ]",
                Borders::default(),
                false,
                Some(Padding::horizontal(1)),
            );
            let keybindings_paragraph = Paragraph::new(keybindings_content).block(block);

            // Position at top-right, under the system stats
            let content_width = keybindings_paragraph.line_width() as u16;
            let content_height = keybindings_paragraph.line_count(content_width) as u16;

            // Calculate the stats box height to position keybindings underneath
            let stats_height = if let Some(ref root) = self.component.root_node {
                // This mirrors the stats calculation from GraphArea::view
                let live_agents = root.count();

                // Get selected agent's actors (same logic as in GraphArea::view)
                let selected_section =
                    if let Some((scope, actors)) = root.find_selected_agent_info() {
                        let mut section = format!("-----\nSelected: {scope}");
                        if actors.is_empty() {
                            section.push_str("\nActors: none");
                        } else {
                            section.push_str("\nActors:");
                            for actor in &actors {
                                section.push_str(&format!("\n  {actor}"));
                            }
                        }
                        section
                    } else {
                        "-----\nNo agent selected".to_string()
                    };

                let stats_content = format!(
                    "Active Agents: {}\nAgents Spawned: {}\nCompletion Requests: {}\nTools Called: {}\nTokens Used: {}\n  Prompt: {}\n  Completion: {}\n{}",
                    live_agents,
                    self.component.stats.agents_spawned,
                    self.component
                        .stats
                        .aggregated_agent_metrics
                        .completion_requests_sent,
                    self.component.stats.aggregated_agent_metrics.tools_called,
                    self.component
                        .stats
                        .aggregated_agent_metrics
                        .total_tokens_used,
                    self.component
                        .stats
                        .aggregated_agent_metrics
                        .prompt_tokens_used,
                    self.component
                        .stats
                        .aggregated_agent_metrics
                        .completion_tokens_used,
                    selected_section
                );
                let stats_block = create_block_with_title(
                    "[ System Stats ]",
                    Borders::default(),
                    false,
                    Some(Padding::horizontal(1)),
                );
                let stats_paragraph = Paragraph::new(stats_content).block(stats_block);
                stats_paragraph.line_count(stats_paragraph.line_width() as u16) as u16
            } else {
                0
            };

            // Create layout for top-right positioning, under stats
            let [keybindings_area] = Layout::horizontal([content_width])
                .flex(Flex::End)
                .areas(area);

            // Position vertically under the stats box with a small gap
            let keybindings_area = Rect {
                x: keybindings_area.x,
                y: keybindings_area.y + stats_height + 1, // +1 for gap
                width: content_width,
                height: content_height,
            };
            Clear.render(keybindings_area, frame.buffer_mut());
            frame.render_widget(keybindings_paragraph, keybindings_area);
        }
    }

    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.component.query(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.component.attr(attr, value);
    }

    fn state(&self) -> State {
        self.component.state()
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        self.component.perform(cmd)
    }
}

impl Component<TuiMessage, MessageEnvelope> for GraphAreaComponent {
    fn on(&mut self, ev: Event<MessageEnvelope>) -> Option<TuiMessage> {
        match ev {
            Event::Keyboard(key_event) => {
                if let Some(action) = self.config.graph.key_bindings.get(&key_event) {
                    let scope = match action {
                        GraphUserAction::SelectDown => self
                            .component
                            .root_node
                            .as_mut()
                            .and_then(|root| root.select_next()),
                        GraphUserAction::SelectUp => self
                            .component
                            .root_node
                            .as_mut()
                            .and_then(|root| root.select_previous()),
                    };

                    if scope.is_some() {
                        // Center the newly selected node
                        self.component.center_on_selected();
                    }

                    scope.map(|scope| {
                        TuiMessage::Graph(GraphTuiMessage::SelectedAgent(scope.to_string()))
                    })
                } else {
                    None
                }
            }
            Event::User(envelope) => {
                // Handle AssistantRequest messages
                if parse_common_message_as::<AssistantRequest>(&envelope).is_some() {
                    if let Some(root) = &mut self.component.root_node {
                        let metrics = AgentMetrics::with_completion_request();
                        self.component.stats.aggregated_agent_metrics += metrics;
                        {
                            let scope = &envelope.from_scope;
                            root.increment_metrics(scope, metrics);
                        }
                        None
                    } else {
                        None
                    }
                }
                // Handle AssistantToolCall messages
                else if parse_common_message_as::<ExecuteTool>(&envelope).is_some() {
                    if let Some(root) = &mut self.component.root_node {
                        let metrics = AgentMetrics::with_tool_call();
                        self.component.stats.aggregated_agent_metrics += metrics;
                        {
                            let scope = &envelope.from_scope;
                            root.increment_metrics(scope, metrics);
                        }
                        None
                    } else {
                        None
                    }
                }
                // Handle AssistantResponse messages
                else if let Some(response) =
                    parse_common_message_as::<AssistantResponse>(&envelope)
                {
                    if let Some(root) = &mut self.component.root_node {
                        let total_tokens = response.usage.total_tokens as u64;
                        let prompt_tokens = response.usage.prompt_tokens as u64;
                        let completion_tokens = response.usage.completion_tokens as u64;
                        let metrics = AgentMetrics::with_tokens(
                            total_tokens,
                            prompt_tokens,
                            completion_tokens,
                        );
                        self.component.stats.aggregated_agent_metrics += metrics;
                        {
                            let scope = &envelope.from_scope;
                            root.increment_metrics(scope, metrics);
                        }
                        None
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

                    // It is possible a node can be sent with a parent scope that does not exist
                    // yet. This can happen when the constructor for an agent spawns another agent.
                    // In this situation we store the node in `nodes_to_insert` and try to insert
                    // again later.
                    match (&mut self.component.root_node, parent_scope) {
                        (None, None) => {
                            node.component.component.is_selected = true;
                            self.component.root_node = Some(node);
                            self.component.height = 1;
                            self.component.center_on_selected();
                        }
                        (None, Some(parent_scope)) => {
                            self.nodes_to_insert.push((parent_scope, node));
                        }
                        (Some(_), None) => {
                            tracing::warn!("Trying to insert a second root node -- ignored");
                        }
                        (Some(root), Some(parent_scope)) => {
                            if let Err(node) = root.insert(&parent_scope, node) {
                                self.nodes_to_insert.push((parent_scope, node));
                            } else {
                                self.component.stats.agents_spawned += 1;
                                self.component.height += 1;
                            }
                        }
                    }

                    if let Some(root) = &mut self.component.root_node {
                        self.nodes_to_insert = self
                            .nodes_to_insert
                            .drain(..)
                            .filter_map(|(parent_scope, node)| {
                                if let Err(node) = root.insert(&parent_scope, node) {
                                    Some((parent_scope, node))
                                } else {
                                    self.component.stats.agents_spawned += 1;
                                    self.component.height += 1;
                                    None
                                }
                            })
                            .collect();
                    }

                    None
                } else if let Some(agent_status_update) =
                    parse_common_message_as::<StatusUpdate>(&envelope)
                {
                    if let Some(root) = &mut self.component.root_node {
                        let agent_scope = &envelope.from_scope;
                        root.set_status(agent_scope, agent_status_update.status);
                        None
                    } else {
                        None
                    }
                }
                // Handle Exit messages - when an agent exits, remove it from the graph
                else if parse_common_message_as::<Exit>(&envelope).is_some() {
                    if let Some(root) = &mut self.component.root_node {
                        // Remove the agent that sent the Exit message
                        if let Some(scope) = root.remove(&envelope.from_scope) {
                            self.component.center_on_selected();
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
        }
    }
}
