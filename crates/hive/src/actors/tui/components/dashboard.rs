use crate::actors::litellm_manager::LiteLLMManager;
use crate::actors::tui::utils::center;
use crate::actors::{Actor, Message};
use crate::actors::{ActorMessage, tui::model::TuiMessage};
use crate::config::ParsedTuiConfig;
use crate::scope::Scope;
use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Widget, Wrap};
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};

use super::chat::ChatAreaComponent;
use super::graph::GraphAreaComponent;
use super::scrollable::ScrollableComponent;

pub const DASHBOARD_SCOPE: Scope =
    Scope::from_uuid(uuid::uuid!("00000000-0000-0000-0000-d68b0e6c4cf1"));

pub const SCOPE_ATTR: &'static str = "SCOPE_ATTR";

/// Actions the user can bind keys to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DashboardUserAction {
    Exit,
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
    pub fn new(config: ParsedTuiConfig) -> Self {
        Self {
            component: Dashboard {
                state: State::None,
                props: Props::default(),
                graph_area_component: ScrollableComponent::new(
                    Box::new(GraphAreaComponent::new(config.clone())),
                    false,
                ),
                chat_area_component: ChatAreaComponent::new(config.clone()),
                litellm_is_ready: false,
            },
            config,
        }
    }
}

struct Dashboard {
    props: Props,
    state: State,
    graph_area_component: ScrollableComponent,
    chat_area_component: ChatAreaComponent,
    litellm_is_ready: bool,
}

impl MockComponent for Dashboard {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
                .split(area);

            if self.litellm_is_ready {
                self.graph_area_component.view(frame, chunks[0]);
            } else {
                let block = Block::new()
                    .borders(Borders::ALL)
                    .padding(Padding::uniform(1));
                let paragraph = Paragraph::new(
                    "Waiting for LiteLLM docker container health check.\n\nThis should only take a few seconds...",
                ).alignment(Alignment::Center).wrap(Wrap { trim: true }).block(block);

                let width = paragraph.line_width();
                let height = paragraph.line_count(area.width);
                let area = center(
                    chunks[0],
                    Constraint::Length(width as u16),
                    Constraint::Length(height as u16),
                );
                paragraph.render(area, frame.buffer_mut());
            }
            self.chat_area_component.view(frame, chunks[1]);
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

impl Component<TuiMessage, ActorMessage> for DashboardComponent {
    fn on(&mut self, ev: Event<ActorMessage>) -> Option<TuiMessage> {
        if let Event::Keyboard(key_event) = &ev {
            if let Some(action) = self.config.dashboard.key_bindings.get(&key_event) {
                match action {
                    DashboardUserAction::Exit => {
                        return Some(TuiMessage::Exit);
                    }
                }
            }
        }

        if let Event::User(ActorMessage {
            message: Message::ActorReady { actor_id },
            ..
        }) = &ev
        {
            if actor_id == LiteLLMManager::ACTOR_ID {
                self.component.litellm_is_ready = true;
                return Some(TuiMessage::Redraw);
            }
        }

        match (
            self.component.graph_area_component.on(ev.clone()),
            self.component.chat_area_component.on(ev),
        ) {
            (None, None) => None,
            (None, Some(msg)) => Some(msg),
            (Some(msg), None) => Some(msg),
            (Some(msg1), Some(msg2)) => Some(TuiMessage::Batch(vec![msg1, msg2])),
        }
    }
}
