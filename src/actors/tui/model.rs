use std::time::Duration;

use tokio::sync::broadcast::Receiver;
use tuirealm::listener::{ListenerResult, Poll};
use tuirealm::ratatui::layout::{Constraint, Direction, Layout};
use tuirealm::terminal::{CrosstermTerminalAdapter, TerminalAdapter, TerminalBridge};
use tuirealm::{Application, EventListenerCfg, ListenerError, Update};

use crate::actors::ActorMessage;
use crate::actors::tui::components::llm_textarea::LLMTextAreaComponent;
use crate::hive::ROOT_AGENT_SCOPE;
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
}

pub struct Model<T>
where
    T: TerminalAdapter,
{
    /// Application
    pub app: Application<Scope, TuiMessage, ActorMessage>,
    /// Indicates that the application must quit
    pub quit: bool,
    /// Tells whether to redraw interface
    pub redraw: bool,
    /// Used to draw to terminal
    pub terminal: TerminalBridge<T>,
}

impl Model<CrosstermTerminalAdapter> {
    pub fn new(rx: Receiver<ActorMessage>) -> Self {
        Self {
            app: Self::init_app(rx),
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
                        .direction(Direction::Vertical)
                        .margin(1)
                        .constraints(
                            [
                                Constraint::Length(3), // Clock
                                Constraint::Length(3), // Letter Counter
                                Constraint::Length(3), // Digit Counter
                                Constraint::Length(1), // Label
                            ]
                            .as_ref(),
                        )
                        .split(f.area());
                    self.app.view(&ROOT_AGENT_SCOPE, f, chunks[0]);
                    // self.app.view(&Id::Clock, f, chunks[0]);
                    // self.app.view(&Id::LetterCounter, f, chunks[1]);
                    // self.app.view(&Id::DigitCounter, f, chunks[2]);
                    // self.app.view(&Id::Label, f, chunks[3]);
                })
                .is_ok()
        );
    }

    pub fn init_app(rx: Receiver<ActorMessage>) -> Application<Scope, TuiMessage, ActorMessage> {
        // Setup application
        // NOTE: NoUserEvent is a shorthand to tell tui-realm we're not going to use any custom user event
        // NOTE: the event listener is configured to use the default crossterm input listener and to raise a Tick event each second
        // which we will use to update the clock

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
                ROOT_AGENT_SCOPE.clone(),
                Box::new(LLMTextAreaComponent::new()),
                Vec::default()
            )
            .is_ok()
        );
        assert!(app.active(&ROOT_AGENT_SCOPE).is_ok());

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

            None

            // Match message
            // match msg {
            //     Msg::AppClose => {
            //         self.quit = true; // Terminate
            //         None
            //     }
            //     Msg::Clock => None,
            //     Msg::DigitCounterBlur => {
            //         // Give focus to letter counter
            //         assert!(self.app.active(&Id::LetterCounter).is_ok());
            //         None
            //     }
            //     Msg::DigitCounterChanged(v) => {
            //         // Update label
            //         assert!(
            //             self.app
            //                 .attr(
            //                     &Id::Label,
            //                     Attribute::Text,
            //                     AttrValue::String(format!("DigitCounter has now value: {v}"))
            //                 )
            //                 .is_ok()
            //         );
            //         None
            //     }
            //     Msg::LetterCounterBlur => {
            //         // Give focus to digit counter
            //         assert!(self.app.active(&Id::DigitCounter).is_ok());
            //         None
            //     }
            //     Msg::LetterCounterChanged(v) => {
            //         // Update label
            //         assert!(
            //             self.app
            //                 .attr(
            //                     &Id::Label,
            //                     Attribute::Text,
            //                     AttrValue::String(format!("LetterCounter has now value: {v}"))
            //                 )
            //                 .is_ok()
            //         );
            //         None
            //     }
            // }
        } else {
            None
        }
    }
}
