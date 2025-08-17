use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
};
use throbber_widgets_tui::{BLACK_CIRCLE, OGHAM_C, Throbber, VERTICAL_BLOCK, symbols::throbber};
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State,
    command::{Cmd, CmdResult},
    props::BorderSides,
    props::Borders,
    props::Color,
    ratatui::layout::Rect,
};
use wasmind::{actors::MessageEnvelope, scope::Scope};
use wasmind_actor_utils::common_messages::assistant::{Status as AgentStatus, WaitReason};

use crate::tui::{model::TuiMessage, throbber_in_title_ext::ThrobberInTitleExt, utils};

pub const WIDGET_WIDTH: u16 = 50;
pub const WIDGET_HEIGHT: u16 = 6;

#[derive(Default, Copy, Clone)]
pub struct AgentMetrics {
    pub completion_requests_sent: u64,
    pub tools_called: u64,
    pub total_tokens_used: u64,
}

impl AgentMetrics {
    pub fn with_tool_call() -> Self {
        Self {
            tools_called: 1,
            ..Default::default()
        }
    }

    pub fn with_completion_request() -> Self {
        Self {
            completion_requests_sent: 1,
            ..Default::default()
        }
    }

    pub fn with_tokens(tokens: u64) -> Self {
        Self {
            total_tokens_used: tokens,
            ..Default::default()
        }
    }
}

impl std::ops::AddAssign<AgentMetrics> for AgentMetrics {
    fn add_assign(&mut self, rhs: AgentMetrics) {
        self.completion_requests_sent =
            self.completion_requests_sent + rhs.completion_requests_sent;
        self.tools_called = self.tools_called + rhs.tools_called;
        self.total_tokens_used = self.total_tokens_used + rhs.total_tokens_used;
    }
}

#[derive(MockComponent)]
pub struct AgentComponent {
    pub component: Agent,
}

impl AgentComponent {
    pub fn new(id: Scope, name: String, actors: Vec<String>, is_selected: bool) -> Self {
        Self {
            component: Agent {
                id,
                is_selected,
                name,
                actors,
                metrics: AgentMetrics::default(),
                state: State::None,
                props: Props::default(),
                status: None,
                context_size: 0,
            },
        }
    }

    pub fn set_status(&mut self, status: AgentStatus) {
        self.component.status = Some(status);
    }

    pub fn increment_metrics(&mut self, metrics: AgentMetrics) {
        self.component.metrics += metrics;
    }
}

fn format_agent_status(status: &AgentStatus) -> &'static str {
    match status {
        AgentStatus::Processing { .. } => "Processing ⌘",
        AgentStatus::Wait { reason } => match reason {
            WaitReason::WaitingForUserInput => "Waiting on user",
            WaitReason::WaitingForSystemInput { .. } => "Waiting on system ⌘",
            WaitReason::WaitingForAgentCoordination { .. } => "Waiting on coordination ⌘",
            WaitReason::WaitingForTools { .. } => "Calling tool ⌘",
            WaitReason::WaitingForAllActorsReady => "Waiting on actors ⌘",
            WaitReason::WaitingForLiteLLM => "Waiting on LiteLLM ⌘",
            WaitReason::CompactingConversation => "Compacting Conversation ⌘",
        },
        AgentStatus::Done { .. } => "Done",
    }
}

fn get_throbber_for_agent_status(status: &AgentStatus) -> Option<throbber::Set> {
    match status {
        AgentStatus::Processing { .. } => Some(BLACK_CIRCLE),
        AgentStatus::Wait { reason } => match reason {
            WaitReason::WaitingForUserInput => None,
            WaitReason::WaitingForSystemInput { .. } => Some(OGHAM_C),
            WaitReason::WaitingForAgentCoordination { .. } => Some(OGHAM_C),
            WaitReason::WaitingForTools { .. } => Some(VERTICAL_BLOCK),
            WaitReason::WaitingForAllActorsReady => Some(OGHAM_C),
            WaitReason::WaitingForLiteLLM => Some(OGHAM_C),
            WaitReason::CompactingConversation => Some(VERTICAL_BLOCK),
        },
        AgentStatus::Done { .. } => None,
    }
}

pub struct Agent {
    pub id: Scope,
    pub is_selected: bool,
    pub name: String,
    pub actors: Vec<String>,
    metrics: AgentMetrics,
    props: Props,
    state: State,
    context_size: u64,
    status: Option<AgentStatus>,
}

impl Agent {
    pub fn view_with_content_trim(&mut self, frame: &mut Frame, area: Rect, trim_top: bool) {
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) != AttrValue::Flag(true) {
            return;
        }

        // Render border (always render if we have any visible area)
        if area.height > 0 {
            let mut borders = if self.is_selected {
                Borders::default()
                    .color(Color::Green)
                    .modifiers(tuirealm::props::BorderType::Thick)
            } else {
                Borders::default()
            };

            if trim_top {
                borders.sides.remove(BorderSides::TOP);
            } else if area.height < WIDGET_HEIGHT {
                borders.sides.remove(BorderSides::BOTTOM);
            }

            let title = if !trim_top {
                if let Some(status) = &self.status {
                    format!("[ {} | {} ]", self.id, format_agent_status(status))
                } else {
                    format!("[ {} ]", self.id)
                }
            } else {
                String::new() // No title when top is clipped
            };

            let maybe_loc = title.chars().position(|c| c == '⌘');
            let div = utils::create_block_with_title(title, borders, false, None);

            if !trim_top
                && let Some(loc) = maybe_loc
                && let Some(status) = &self.status
                && let Some(throbber_set) = get_throbber_for_agent_status(status)
            {
                let throbber = Throbber::default().throbber_set(throbber_set);
                div.render_with_throbber(frame, area, loc, throbber);
            } else {
                frame.render_widget_ref(div, area);
            }
        }

        // Render content inside the box
        if area.height > 1 {
            // We need at least 3 lines to show content top or bottom border / conent
            let content_area = Rect::new(
                area.x + 2,
                area.y + if trim_top { 0 } else { 1 },
                area.width.saturating_sub(4),
                area.height
                    .saturating_sub(if trim_top || area.height < WIDGET_HEIGHT {
                        1
                    } else {
                        2
                    }),
            );

            // Generate all content lines
            let mut content_lines = Vec::new();

            // Line 1: Name (truncated with ellipsis if needed)
            let name_str = format!("Name: {}", self.name);
            let name_line = if name_str.len() > content_area.width as usize {
                format!(
                    "{}...",
                    &name_str[..content_area.width.saturating_sub(3) as usize]
                )
            } else {
                name_str
            };
            content_lines.push(Line::from(name_line));

            // Line 2: Actor count
            content_lines.push(Line::from(format!("Actors: {} active", self.actors.len())));

            // Line 3: Separator
            let separator_width = content_area.width.min(45) as usize;
            content_lines.push(Line::from("─".repeat(separator_width)));

            // Line 4: All metrics in 4-column layout
            content_lines.push(Line::from(vec![
                Span::raw(format!("Ctx:{:5} ", self.context_size)),
                Span::raw(format!("Reqs:{:5} ", self.metrics.completion_requests_sent)),
                Span::raw(format!("Tools:{:5} ", self.metrics.tools_called)),
                Span::raw(format!("Tok:{:7}", self.metrics.total_tokens_used)),
            ]));

            // Calculate which lines to show based on trim_top and available height
            let skip_lines = if trim_top {
                // When top is trimmed, we need to skip some content lines
                // The border takes 1 line, so WIDGET_HEIGHT - area.height - 1 gives us content lines to skip
                (WIDGET_HEIGHT.saturating_sub(area.height).saturating_sub(1)) as usize
            } else {
                0
            };

            // Take only the visible lines
            let visible_lines: Vec<Line> = content_lines
                .into_iter()
                .skip(skip_lines)
                .take(content_area.height as usize)
                .collect();

            if !visible_lines.is_empty() {
                let content_paragraph = Paragraph::new(visible_lines);
                frame.render_widget(content_paragraph, content_area);
            }
        }
    }
}

impl MockComponent for Agent {
    fn view(&mut self, _frame: &mut Frame, _area: Rect) {
        panic!("Use view_with_content_offset instead of view() for agent rendering");
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

impl Component<TuiMessage, MessageEnvelope> for AgentComponent {
    fn on(&mut self, _ev: Event<MessageEnvelope>) -> Option<TuiMessage> {
        None
    }
}
