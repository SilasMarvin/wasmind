use crate::config::ParsedTuiConfig;
use crate::tui::model::TuiMessage;
use hive::{actors::MessageEnvelope, utils::parse_common_message_as};
use hive_actor_utils::common_messages::assistant::AddMessage;
use ratatui::layout::{Constraint, Direction, Layout};
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};

use super::chat::ChatAreaComponent;
use super::graph::GraphAreaComponent;
use super::splash::SplashComponent;

pub const DASHBOARD_SCOPE: &str = "DASHBD";

pub const SCOPE_ATTR: &str = "SCOPE_ATTR";

/// Actions the user can bind keys to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DashboardUserAction {
    Exit,
}

impl DashboardUserAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            DashboardUserAction::Exit => "Exit",
        }
    }
}

impl TryFrom<&str> for DashboardUserAction {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "Exit" => Ok(DashboardUserAction::Exit),
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
        if let Event::Tick = &ev {
            return Some(TuiMessage::Redraw);
        }

        if let Event::Keyboard(key_event) = &ev
            && let Some(action) = self.config.dashboard.key_bindings.get(key_event)
        {
            match action {
                DashboardUserAction::Exit => {
                    return Some(TuiMessage::Exit);
                }
            }
        }

        if let Event::User(envelope) = &ev {
            if parse_common_message_as::<AddMessage>(envelope).is_some() {
                self.component.show_splash = false;

                return match (
                    self.component.graph_area_component.on(ev.clone()),
                    self.component.chat_area_component.on(ev),
                ) {
                    (None, None) => Some(TuiMessage::Redraw),
                    (None, Some(msg)) => Some(msg),
                    (Some(msg), None) => Some(msg),
                    (Some(msg1), Some(msg2)) => Some(TuiMessage::Batch(vec![msg1, msg2])),
                };
            }
        }

        let mut conditional_msg_set = match (self.component.show_splash, &ev) {
            (_, Event::User(_)) => {
                vec![
                    self.component.chat_area_component.on(ev.clone()),
                    self.component.splash_component.on(ev.clone()),
                ]
            }
            (false, _) => {
                vec![self.component.chat_area_component.on(ev.clone())]
            }
            (true, _) => {
                vec![self.component.splash_component.on(ev.clone())]
            }
        };

        let graph_area_component_msg = self.component.graph_area_component.on(ev);
        conditional_msg_set.extend([graph_area_component_msg]);

        Some(TuiMessage::Batch(
            conditional_msg_set.into_iter().flatten().collect(),
        ))
    }
}
