use crate::config::ParsedTuiConfig;
use crate::tui::global_throbber;
use crate::tui::model::TuiMessage;
use ratatui::layout::{Constraint, Direction, Layout};
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};
use wasmind::{actors::MessageEnvelope, utils::parse_common_message_as};
use wasmind_actor_utils::common_messages::assistant::AddMessage;

use super::chat::ChatAreaComponent;
use super::graph::GraphAreaComponent;
use super::splash::SplashComponent;
use super::toast::ToastComponent;

pub const DASHBOARD_SCOPE: &str = "DASHBD";

pub const SCOPE_ATTR: &str = "SCOPE_ATTR";

/// Actions the user can bind keys to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DashboardUserAction {
    Exit,
    InterruptAgent,
}

impl DashboardUserAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            DashboardUserAction::Exit => "Exit",
            DashboardUserAction::InterruptAgent => "InterruptAgent",
        }
    }
}

impl TryFrom<&str> for DashboardUserAction {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "Exit" => Ok(DashboardUserAction::Exit),
            "InterruptAgent" => Ok(DashboardUserAction::InterruptAgent),
            _ => Err(()),
        }
    }
}

#[derive(MockComponent)]
pub struct DashboardComponent {
    component: Dashboard,
    config: ParsedTuiConfig,
}

impl DashboardComponent {
    pub fn new(config: ParsedTuiConfig, initial_prompt: Option<String>) -> Self {
        Self {
            component: Dashboard {
                state: State::None,
                props: Props::default(),
                graph_area_component: GraphAreaComponent::new(config.clone()),
                show_splash: initial_prompt.is_none(),
                chat_area_component: ChatAreaComponent::new(config.clone(), initial_prompt),
                splash_component: SplashComponent::new(config.clone()),
                toast_component: ToastComponent::new(),
            },
            config,
        }
    }
}

struct Dashboard {
    props: Props,
    state: State,
    graph_area_component: GraphAreaComponent,
    chat_area_component: ChatAreaComponent,
    splash_component: SplashComponent,
    toast_component: ToastComponent,
    show_splash: bool,
}

impl MockComponent for Dashboard {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            if self.show_splash {
                self.splash_component.view(frame, area);
            } else {
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
                    .spacing(1)
                    .split(area);
                self.graph_area_component.view(frame, chunks[0]);
                self.chat_area_component.view(frame, chunks[1]);
            }

            // Render toasts as overlay on top of everything
            self.toast_component.view(frame, area);
        }
    }

    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.props.get(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        if Attribute::Custom(SCOPE_ATTR) == attr {
            self.chat_area_component.attr(attr, value);
        } else {
            self.props.set(attr, value);
        }
    }

    fn state(&self) -> State {
        self.state.clone()
    }

    fn perform(&mut self, _cmd: Cmd) -> CmdResult {
        unreachable!()
    }
}

impl Component<TuiMessage, MessageEnvelope> for DashboardComponent {
    fn on(&mut self, ev: Event<MessageEnvelope>) -> Option<TuiMessage> {
        // Handle tick events
        if let Event::Tick = &ev {
            global_throbber::tick();
            return Some(TuiMessage::Redraw);
        }

        // Handle keyboard events
        if let Event::Keyboard(key_event) = &ev
            && let Some(action) = self.config.dashboard.key_bindings.get(key_event)
        {
            match action {
                DashboardUserAction::Exit => return Some(TuiMessage::Exit),
                DashboardUserAction::InterruptAgent => return Some(TuiMessage::InterruptAgent),
            }
        }

        // Handle AddMessage special case - hide splash and route to graph + chat only
        if let Event::User(envelope) = &ev {
            if parse_common_message_as::<AddMessage>(envelope).is_some() {
                self.component.show_splash = false;

                let mut messages = Vec::new();
                messages.extend(self.component.graph_area_component.on(ev.clone()));
                messages.extend(self.component.chat_area_component.on(ev.clone()));

                return if messages.is_empty() {
                    Some(TuiMessage::Redraw)
                } else {
                    Some(TuiMessage::Batch(messages))
                };
            }
        }

        // Route events to appropriate components based on current state
        let mut messages = Vec::new();

        // Graph component always gets events
        messages.extend(self.component.graph_area_component.on(ev.clone()));

        // Route to main content components based on splash state
        if self.component.show_splash {
            messages.extend(self.component.splash_component.on(ev.clone()));
        } else {
            messages.extend(self.component.chat_area_component.on(ev.clone()));
        }

        // Toast component always gets events (handles UserNotification and Tick internally)
        messages.extend(self.component.toast_component.on(ev));

        Some(TuiMessage::Batch(messages))
    }
}
