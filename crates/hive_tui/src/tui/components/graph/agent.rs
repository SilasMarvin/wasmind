use crate::tui::{model::TuiMessage, throbber_in_title_ext::ThrobberInTitleExt, utils};
use hive::{actors::MessageEnvelope, scope::Scope};
use hive_actor_utils_common_messages::assistant::Status as AgentStatus;
use ratatui::widgets::{Padding, Paragraph, Widget, Wrap};
use throbber_widgets_tui::{
    BLACK_CIRCLE, OGHAM_C, Throbber, ThrobberState, VERTICAL_BLOCK, symbols::throbber,
};
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State,
    command::{Cmd, CmdResult},
    props::Borders,
    props::Color,
    ratatui::layout::Rect,
};

pub const WIDGET_WIDTH: u16 = 50;
pub const WIDGET_HEIGHT: u16 = 8;

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
                throbber_state: ThrobberState::default(),
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
            hive_actor_utils_common_messages::assistant::WaitReason::WaitingForUserInput => "Waiting on user",
            hive_actor_utils_common_messages::assistant::WaitReason::WaitingForSystemInput { .. } => "Waiting on system ⌘",
            hive_actor_utils_common_messages::assistant::WaitReason::WaitingForAgentCoordination { .. } => "Waiting on coordination ⌘",
            hive_actor_utils_common_messages::assistant::WaitReason::WaitingForTools { .. } => "Calling tool ⌘",
            hive_actor_utils_common_messages::assistant::WaitReason::WaitingForAllActorsReady => "Waiting on actors ⌘",
            hive_actor_utils_common_messages::assistant::WaitReason::WaitingForLiteLLM => "Waiting on LiteLLM ⌘",
        },
        AgentStatus::Done {..} => "Done",
    }
}

fn get_throbber_for_agent_status(status: &AgentStatus) -> Option<throbber::Set> {
    match status {
        AgentStatus::Processing { .. } => Some(BLACK_CIRCLE),
        AgentStatus::Wait { reason } => match reason {
            hive_actor_utils_common_messages::assistant::WaitReason::WaitingForUserInput => None,
            hive_actor_utils_common_messages::assistant::WaitReason::WaitingForSystemInput { .. } => Some(OGHAM_C),
            hive_actor_utils_common_messages::assistant::WaitReason::WaitingForAgentCoordination { .. } => Some(OGHAM_C),
            hive_actor_utils_common_messages::assistant::WaitReason::WaitingForTools { .. } => Some(VERTICAL_BLOCK),
            hive_actor_utils_common_messages::assistant::WaitReason::WaitingForAllActorsReady => Some(OGHAM_C),
            hive_actor_utils_common_messages::assistant::WaitReason::WaitingForLiteLLM => Some(OGHAM_C),
        },
        AgentStatus::Done {..} => None,
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
    throbber_state: ThrobberState,
}

impl MockComponent for Agent {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            assert!(area.area() == WIDGET_WIDTH as u32 * WIDGET_HEIGHT as u32);

            let borders = if self.is_selected {
                Borders::default().color(Color::Green)
            } else {
                Borders::default()
            };
            let title = if let Some(status) = &self.status {
                format!("[ {} | {} ]", self.name, format_agent_status(status))
            } else {
                format!("[ {} ]", self.name)
            };
            let maybe_loc = title.chars().position(|c| c == '⌘');
            let div = utils::create_block_with_title(title, borders, false, None);

            if let Some(loc) = maybe_loc
                && let Some(status) = &self.status
                && let Some(throbber_set) = get_throbber_for_agent_status(&status)
            {
                let throbber = Throbber::default().throbber_set(throbber_set);
                self.throbber_state.calc_next();
                div.render_with_throbber(frame, area, loc, throbber, &mut self.throbber_state);
            } else {
                frame.render_widget_ref(div, area);
            }

            // Render the Actors list
            let actors_paragraph_chunk = Rect::new(area.x + 2, area.y + 1, area.width - 4, 2);
            let actors_text = if self.actors.is_empty() {
                "Actors: []".to_string()
            } else {
                format!("Actors: [{}]", self.actors.join(", "))
            };
            let actors_paragraph = Paragraph::new(actors_text).wrap(Wrap { trim: true });
            actors_paragraph.render(actors_paragraph_chunk, frame.buffer_mut());

            // Context
            let paragraph_chunk = Rect::new(area.x + 2, area.y + 5, area.width, 2);
            let paragraph = Paragraph::new(format!("Context\n{}", self.context_size));
            paragraph.render(paragraph_chunk, frame.buffer_mut());

            // Requests Made
            let paragraph_chunk = Rect::new(area.x + 12, area.y + 5, area.width, 2);
            let paragraph = Paragraph::new(format!(
                "Requests\n{}",
                self.metrics.completion_requests_sent
            ));
            paragraph.render(paragraph_chunk, frame.buffer_mut());

            // Tool Calls
            let paragraph_chunk = Rect::new(area.x + 23, area.y + 5, area.width, 2);
            let paragraph = Paragraph::new(format!("Tool Calls\n{}", self.metrics.tools_called));
            paragraph.render(paragraph_chunk, frame.buffer_mut());

            // Tokens Used
            let paragraph_chunk = Rect::new(area.x + 36, area.y + 5, area.width, 2);
            let paragraph =
                Paragraph::new(format!("Tokens Used\n{}", self.metrics.total_tokens_used));
            paragraph.render(paragraph_chunk, frame.buffer_mut());
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

impl Component<TuiMessage, MessageEnvelope> for AgentComponent {
    fn on(&mut self, _ev: Event<MessageEnvelope>) -> Option<TuiMessage> {
        None
    }
}
