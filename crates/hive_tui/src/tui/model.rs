use std::time::Duration;
use tokio::sync::broadcast::{Receiver, Sender};
use tuirealm::listener::{ListenerResult, Poll};
use tuirealm::terminal::{CrosstermTerminalAdapter, TerminalAdapter, TerminalBridge};
use tuirealm::{Application, AttrValue, Attribute, EventListenerCfg, ListenerError, Update};

use crate::config::ParsedTuiConfig;
use crate::tui::components::dashboard::{DASHBOARD_SCOPE, DashboardComponent, SCOPE_ATTR};
use hive::actors::MessageEnvelope;

use super::components::graph::GraphTuiMessage;

struct PollBroadcastWrapper {
    rx: Receiver<MessageEnvelope>,
}

impl Poll<MessageEnvelope> for PollBroadcastWrapper {
    fn poll(&mut self) -> ListenerResult<Option<tuirealm::Event<MessageEnvelope>>> {
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
    pub app: Application<String, TuiMessage, MessageEnvelope>,
    pub quit: bool,
    pub redraw: bool,
    pub terminal: TerminalBridge<T>,
    tx: Sender<MessageEnvelope>,
}

impl Model<CrosstermTerminalAdapter> {
    pub fn new(
        config: ParsedTuiConfig,
        tx: Sender<MessageEnvelope>,
        rx: Receiver<MessageEnvelope>,
        initial_prompt: Option<String>,
    ) -> Self {
        Self {
            app: Self::init_app(config, rx, initial_prompt),
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
                    self.app.view(&DASHBOARD_SCOPE.to_string(), f, f.area());
                })
                .is_ok()
        );
    }

    pub fn init_app(
        config: ParsedTuiConfig,
        rx: Receiver<MessageEnvelope>,
        initial_prompt: Option<String>,
    ) -> Application<String, TuiMessage, MessageEnvelope> {
        let mut app: Application<String, TuiMessage, MessageEnvelope> = Application::init(
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
                DASHBOARD_SCOPE.to_string(),
                Box::new(DashboardComponent::new(config, initial_prompt)),
                Vec::default()
            )
            .is_ok()
        );
        assert!(app.active(&DASHBOARD_SCOPE.to_string()).is_ok());
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
                    // For now, just set quit - we'll implement proper exit messaging later
                }
                TuiMessage::UpdatedUserTypedLLMMessage(_) => (),
                TuiMessage::SubmittedUserTypedLLMMessage(_message) => {
                    // For now, just ignore - we'll implement proper message sending later
                }
                TuiMessage::Redraw => (),
                TuiMessage::Graph(graph_message) => match graph_message {
                    GraphTuiMessage::SelectedAgent(scope) => {
                        assert!(
                            self.app
                                .attr(
                                    &DASHBOARD_SCOPE.to_string(),
                                    Attribute::Custom(SCOPE_ATTR),
                                    AttrValue::String(scope)
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
