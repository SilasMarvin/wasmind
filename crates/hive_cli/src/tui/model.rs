use hive::actors::MessageEnvelope;
use hive::context::HiveContext;
use hive::scope::Scope;
use hive_actor_utils::STARTING_SCOPE;
use hive_actor_utils::common_messages::actors::Exit;
use hive_actor_utils::common_messages::assistant::AddMessage;
use hive_actor_utils::llm_client_types::ChatMessage;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::Receiver;
use tuirealm::listener::{ListenerResult, Poll};
use tuirealm::terminal::{CrosstermTerminalAdapter, TerminalAdapter, TerminalBridge};
use tuirealm::{Application, AttrValue, Attribute, EventListenerCfg, ListenerError, Update};

use crate::config::ParsedTuiConfig;
use crate::tui::components::dashboard::{DASHBOARD_SCOPE, DashboardComponent, SCOPE_ATTR};

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
    context: Arc<HiveContext>,
    active_scope: Scope,
}

impl Model<CrosstermTerminalAdapter> {
    pub fn new(
        config: ParsedTuiConfig,
        rx: Receiver<MessageEnvelope>,
        initial_prompt: Option<String>,
        context: Arc<HiveContext>,
    ) -> Self {
        Self {
            app: Self::init_app(config, rx, initial_prompt),
            quit: false,
            redraw: true,
            terminal: TerminalBridge::init_crossterm().expect("Cannot initialize terminal"),
            context,
            active_scope: STARTING_SCOPE.to_string(),
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
                    4096,
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
            match msg {
                TuiMessage::Batch(batch) => {
                    for msg in batch {
                        self.update(Some(msg));
                    }
                }
                TuiMessage::Exit => {
                    // Broadcast exit message
                    if let Err(e) = self.context.broadcast_common_message(Exit) {
                        tracing::error!("Failed to broadcast exit message: {}", e);
                    }
                    self.quit = true;
                }
                TuiMessage::UpdatedUserTypedLLMMessage(_) => {
                    self.redraw = true;
                }
                TuiMessage::SubmittedUserTypedLLMMessage(message) => {
                    // Broadcast AddMessage from the user
                    let add_message = AddMessage {
                        agent: self.active_scope.to_string(),
                        message: ChatMessage::user(&message),
                    };
                    if let Err(e) = self.context.broadcast_common_message(add_message) {
                        tracing::error!("Failed to broadcast AddMessage: {}", e);
                    }
                    self.redraw = true;
                }
                TuiMessage::Redraw => {
                    self.redraw = true;
                }
                TuiMessage::Graph(graph_message) => match graph_message {
                    GraphTuiMessage::SelectedAgent(scope) => {
                        self.active_scope = Scope::from(scope.as_str());
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
