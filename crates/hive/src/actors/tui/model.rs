use std::time::Duration;
use tokio::sync::broadcast::{Receiver, Sender};
use tuirealm::listener::{ListenerResult, Poll};
use tuirealm::terminal::{CrosstermTerminalAdapter, TerminalAdapter, TerminalBridge};
use tuirealm::{Application, AttrValue, Attribute, EventListenerCfg, ListenerError, Update};

use crate::actors::tui::components::dashboard::{DASHBOARD_SCOPE, DashboardComponent, SCOPE_ATTR};
use crate::actors::{ActorMessage, Message, UserContext};
use crate::config::ParsedTuiConfig;
use crate::hive::MAIN_MANAGER_SCOPE;
use crate::scope::Scope;

use super::components::graph::GraphTuiMessage;

struct PollBroadcastWrapper {
    rx: Receiver<ActorMessage>,
}

impl Poll<ActorMessage> for PollBroadcastWrapper {
    fn poll(&mut self) -> ListenerResult<Option<tuirealm::Event<ActorMessage>>> {
        match self.rx.try_recv() {
            Ok(msg) => Ok(Some(tuirealm::Event::User(msg))),
            Err(e) => match e {
                tokio::sync::broadcast::error::TryRecvError::Empty => Ok(None),
                e => {
                    tracing::error!("{e:?}");
                    Err(ListenerError::PollFailed)
                }
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiMessage {
    Batch(Vec<TuiMessage>),
    Redraw,
    Exit,
    UpdatedUserTypedLLMMessage(String),
    SubmittedUserTypedLLMMessage(String),
    Graph(GraphTuiMessage),
}

pub struct Model<T>
where
    T: TerminalAdapter,
{
    pub app: Application<Scope, TuiMessage, ActorMessage>,
    pub quit: bool,
    pub redraw: bool,
    pub terminal: TerminalBridge<T>,
    tx: Sender<ActorMessage>,
}

impl Model<CrosstermTerminalAdapter> {
    pub fn new(config: ParsedTuiConfig, tx: Sender<ActorMessage>) -> Self {
        Self {
            app: Self::init_app(config, tx.subscribe()),
            tx,
            quit: false,
            redraw: true,
            terminal: TerminalBridge::init_crossterm().expect("Cannot initialize terminal"),
        }
    }
}

impl<T> Model<T>
where
    T: TerminalAdapter,
{
    pub fn view(&mut self) {
        assert!(
            self.terminal
                .draw(|f| {
                    self.app.view(&DASHBOARD_SCOPE, f, f.area());
                })
                .is_ok()
        );
    }

    pub fn init_app(
        config: ParsedTuiConfig,
        rx: Receiver<ActorMessage>,
    ) -> Application<Scope, TuiMessage, ActorMessage> {
        let mut app: Application<Scope, TuiMessage, ActorMessage> = Application::init(
            EventListenerCfg::default()
                .crossterm_input_listener(Duration::from_millis(5), 1)
                .add_port(
                    Box::new(PollBroadcastWrapper { rx }),
                    Duration::from_millis(20),
                    1024,
                )
                .tick_interval(Duration::from_millis(350))
                .poll_timeout(Duration::from_millis(20)),
        );

        assert!(
            app.mount(
                DASHBOARD_SCOPE.clone(),
                Box::new(DashboardComponent::new(config)),
                Vec::default()
            )
            .is_ok()
        );
        assert!(app.active(&DASHBOARD_SCOPE).is_ok());
        app
    }
}

impl<T> Update<TuiMessage> for Model<T>
where
    T: TerminalAdapter,
{
    fn update(&mut self, msg: Option<TuiMessage>) -> Option<TuiMessage> {
        if let Some(msg) = msg {
            // Set redraw
            self.redraw = true;

            match msg {
                TuiMessage::Batch(batch) => {
                    for msg in batch {
                        self.update(Some(msg));
                    }
                }
                TuiMessage::Exit => {
                    self.quit = true;
                    let _ = self.tx.send(ActorMessage {
                        scope: MAIN_MANAGER_SCOPE.clone(),
                        message: Message::Exit,
                    });
                }
                TuiMessage::UpdatedUserTypedLLMMessage(_) => (),
                TuiMessage::SubmittedUserTypedLLMMessage(message) => {
                    let _ = self.tx.send(ActorMessage {
                        scope: MAIN_MANAGER_SCOPE.clone(),
                        message: Message::UserContext(UserContext::UserTUIInput(message)),
                    });
                }
                TuiMessage::Redraw => (),
                TuiMessage::Graph(graph_message) => match graph_message {
                    GraphTuiMessage::SelectedAgent(scope) => {
                        assert!(
                            self.app
                                .attr(
                                    &DASHBOARD_SCOPE,
                                    Attribute::Custom(SCOPE_ATTR),
                                    AttrValue::String(scope.to_string())
                                )
                                .is_ok()
                        );
                    }
                },
            }
        }

        None
    }
}
