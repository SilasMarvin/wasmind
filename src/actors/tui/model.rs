use std::time::Duration;

use tokio::sync::broadcast::{Receiver, Sender};
use tuirealm::listener::{ListenerResult, Poll};
use tuirealm::ratatui::layout::{Constraint, Direction, Layout};
use tuirealm::terminal::{CrosstermTerminalAdapter, TerminalAdapter, TerminalBridge};
use tuirealm::{Application, EventListenerCfg, ListenerError, Update};

use crate::actors::tui::components::chat::{CHAT_SCOPE, ChatAreaComponent};
use crate::actors::tui::components::graph::GRAPH_SCOPE;
use crate::actors::tui::components::llm_textarea::LLMTextAreaComponent;
use crate::actors::{ActorMessage, AgentMessageType, Message, UserContext};
use crate::hive::MAIN_MANAGER_SCOPE;
use crate::scope::Scope;

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
    ActorMessage(ActorMessage),
    UpdatedUserTypedLLMMessage(String),
    SubmittedUserTypedLLMMessage(String),
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
    pub fn new(tx: Sender<ActorMessage>) -> Self {
        Self {
            app: Self::init_app(tx.subscribe()),
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
                    let chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .margin(1)
                        .constraints(
                            [Constraint::Percentage(50), Constraint::Percentage(50)].as_ref(),
                        )
                        .split(f.area());
                    // self.app.view(&GRAPH_SCOPE, f, chunks[0]);
                    self.app.view(&CHAT_SCOPE, f, chunks[1]);
                })
                .is_ok()
        );
    }

    pub fn init_app(rx: Receiver<ActorMessage>) -> Application<Scope, TuiMessage, ActorMessage> {
        let mut app: Application<Scope, TuiMessage, ActorMessage> = Application::init(
            EventListenerCfg::default()
                .crossterm_input_listener(Duration::from_millis(20), 3)
                .add_port(
                    Box::new(PollBroadcastWrapper { rx }),
                    Duration::from_millis(20),
                    1024,
                )
                .tick_interval(Duration::from_secs(1))
                .poll_timeout(Duration::from_millis(10)),
        );

        assert!(
            app.mount(
                CHAT_SCOPE.clone(),
                Box::new(ChatAreaComponent::new()),
                Vec::default()
            )
            .is_ok()
        );
        assert!(app.active(&CHAT_SCOPE).is_ok());

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
                TuiMessage::ActorMessage(actor_message) => (),
                TuiMessage::UpdatedUserTypedLLMMessage(_) => (),
                TuiMessage::SubmittedUserTypedLLMMessage(message) => {
                    let _ = self.tx.send(ActorMessage {
                        scope: MAIN_MANAGER_SCOPE.clone(),
                        message: Message::UserContext(UserContext::UserTUIInput(message)),
                    });
                }
            }
        }

        None
    }
}
