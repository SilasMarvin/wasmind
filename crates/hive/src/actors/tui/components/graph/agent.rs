use crate::{
    actors::{
        ActorMessage, AgentStatus, AgentType,
        tui::{
            icons::{MAIN_MANAGER_ICON, SUB_MANGER_ICON, WORKER_ICON},
            model::TuiMessage,
            utils,
        },
    },
    scope::Scope,
};
use ratatui::widgets::{Padding, Paragraph, Widget, Wrap};
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State,
    command::{Cmd, CmdResult},
    props::Borders,
    props::Color,
    ratatui::layout::Rect,
};

pub const WIDGET_WIDTH: u16 = 50;
pub const WIDGET_HEIGHT: u16 = 9;

fn get_icon_for_agent_type(agent_type: AgentType) -> &'static str {
    match agent_type {
        AgentType::MainManager => MAIN_MANAGER_ICON,
        AgentType::SubManager => SUB_MANGER_ICON,
        AgentType::Worker => WORKER_ICON,
    }
}

#[derive(Default, Copy, Clone)]
pub struct AgentMetrics {
    completion_requests_sent: u64,
    tools_called: u64,
    total_tokens_used: u64,
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

impl std::ops::Add<AgentMetrics> for AgentMetrics {
    type Output = AgentMetrics;

    fn add(self, rhs: AgentMetrics) -> Self::Output {
        AgentMetrics {
            completion_requests_sent: self.completion_requests_sent + rhs.completion_requests_sent,
            tools_called: self.tools_called + rhs.tools_called,
            total_tokens_used: self.total_tokens_used + rhs.total_tokens_used,
        }
    }
}

#[derive(MockComponent)]
pub struct AgentComponent {
    pub component: Agent,
}

impl AgentComponent {
    pub fn new(
        id: Scope,
        agent_type: AgentType,
        role: String,
        task: Option<String>,
        is_selected: bool,
    ) -> Self {
        Self {
            component: Agent {
                id,
                is_selected,
                agent_type,
                role,
                metrics: AgentMetrics::default(),
                state: State::None,
                props: Props::default(),
                task,
                status: None,
                context_size: 0,
            },
        }
    }

    pub fn set_status(&mut self, status: AgentStatus) {
        self.component.status = Some(status);
    }

    pub fn increment_metrics(&mut self, metrics: AgentMetrics) {
        self.component.metrics = self.component.metrics.clone() + metrics;
    }
}

pub struct Agent {
    pub id: Scope,
    pub is_selected: bool,
    metrics: AgentMetrics,
    props: Props,
    state: State,
    pub agent_type: AgentType,
    role: String,
    task: Option<String>,
    context_size: u64,
    status: Option<AgentStatus>,
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
            let title = format!(
                "[ {} {} ]",
                get_icon_for_agent_type(self.agent_type),
                self.agent_type
            );
            let div =
                utils::create_block_with_title(title, borders, false, Some(Padding::uniform(1)));
            frame.render_widget(div, area);

            // Render the Role
            let role_paragraph_chunk = Rect::new(area.x + 2, area.y + 2, area.width - 4, 1);
            let role_paragraph = Paragraph::new(format!("Role: {}", self.role));
            role_paragraph.render(role_paragraph_chunk, frame.buffer_mut());

            // Render the task
            let render_paragraph_chunk = Rect::new(area.x + 2, area.y + 3, area.width - 4, 3);
            let task = if self.agent_type == AgentType::MainManager && self.task.is_none() {
                "Task: (dynamically set by user)".to_string()
            } else if let Some(task) = &self.task {
                format!("Task: {task}")
            } else {
                "Task: Uknown (this is a bug please report it)".to_string()
            };
            let render_paragraph = Paragraph::new(task).wrap(Wrap { trim: true });
            render_paragraph.render(render_paragraph_chunk, frame.buffer_mut());

            // Context
            let paragraph_chunk = Rect::new(area.x + 2, area.y + 6, area.width, 2);
            let paragraph = Paragraph::new(format!("Context\n{}", self.context_size));
            paragraph.render(paragraph_chunk, frame.buffer_mut());

            // Requests Made
            let paragraph_chunk = Rect::new(area.x + 12, area.y + 6, area.width, 2);
            let paragraph = Paragraph::new(format!(
                "Requests\n{}",
                self.metrics.completion_requests_sent
            ));
            paragraph.render(paragraph_chunk, frame.buffer_mut());

            // Tool Calls
            let paragraph_chunk = Rect::new(area.x + 23, area.y + 6, area.width, 2);
            let paragraph = Paragraph::new(format!("Tool Calls\n{}", self.metrics.tools_called));
            paragraph.render(paragraph_chunk, frame.buffer_mut());

            // Tokens Used
            let paragraph_chunk = Rect::new(area.x + 36, area.y + 6, area.width, 2);
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

impl Component<TuiMessage, ActorMessage> for AgentComponent {
    fn on(&mut self, ev: Event<ActorMessage>) -> Option<TuiMessage> {
        None
    }
}
