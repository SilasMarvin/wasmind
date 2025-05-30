pub mod app;
pub mod events;
pub mod ui;
pub mod widgets;

use std::io;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crossterm::event::KeyModifiers;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event as CrosstermEvent, KeyCode,
        MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use snafu::ResultExt;
use tokio::sync::broadcast;
use tracing::{error, info};

use crate::{
    actors::{Action, Actor, Message},
    config::ParsedConfig,
    key_bindings::KeyBindingManager,
};

use self::app::App;
use self::events::TuiEvent;

#[derive(Clone, Copy)]
enum RedrawMessage {
    Redraw,
    End,
}

/// TUI Error types
#[derive(Debug, snafu::Snafu)]
pub enum TuiError {
    #[snafu(display("Failed to setup terminal"))]
    TerminalSetup { source: io::Error },

    #[snafu(display("Failed to restore terminal"))]
    TerminalRestore { source: io::Error },

    #[snafu(display("Failed to draw frame"))]
    DrawFrame { source: io::Error },
}

type TuiResult<T> = Result<T, TuiError>;

/// TUI actor that handles the terminal user interface
#[derive(Clone)]
pub struct TuiActor {
    tx: broadcast::Sender<Message>,
    config: ParsedConfig,
    app: Arc<Mutex<App>>,
    key_bindings: Arc<Mutex<KeyBindingManager>>,
    redraw_tx: crossbeam::channel::Sender<RedrawMessage>,
}

impl TuiActor {
    fn trigger_redraw(&self, stop: bool) {
        if stop {
            let _ = self.redraw_tx.send(RedrawMessage::End);
        } else {
            let _ = self.redraw_tx.send(RedrawMessage::Redraw);
        }
    }

    fn run_terminal(
        self,
        redraw_tx: crossbeam::channel::Sender<RedrawMessage>,
        redraw_rx: crossbeam::channel::Receiver<RedrawMessage>,
    ) -> TuiResult<()> {
        // Setup terminal
        enable_raw_mode().context(TerminalSetupSnafu)?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture).context(TerminalSetupSnafu)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).context(TerminalSetupSnafu)?;

        // Spawn input handler thread
        let app_clone = self.app.clone();
        let tx_clone = self.tx.clone();

        let redraw_tx_clone = redraw_tx.clone();
        thread::spawn(move || {
            handle_input(app_clone, tx_clone, redraw_tx_clone);
        });

        // Initial draw
        terminal
            .draw(|f| {
                let mut app = self.app.lock().unwrap();
                let chat_height = f.area().height.saturating_sub(4);
                let chat_width = f.area().width;
                if chat_height != app.visible_height || chat_width != app.visible_width {
                    app.set_visible_dimensions(chat_width, chat_height);
                }
                ui::draw(f, &*app);
            })
            .context(DrawFrameSnafu)?;

        // Main render loop
        while let Ok(r) = redraw_rx.recv() {
            match r {
                RedrawMessage::Redraw => {
                    terminal
                        .draw(|f| {
                            let mut app = self.app.lock().unwrap();
                            let chat_height = f.area().height.saturating_sub(4);
                            let chat_width = f.area().width;
                            if chat_height != app.visible_height || chat_width != app.visible_width
                            {
                                app.set_visible_dimensions(chat_width, chat_height);
                            }
                            ui::draw(f, &*app);
                        })
                        .context(DrawFrameSnafu)?;
                }
                RedrawMessage::End => break,
            }
        }

        // Restore terminal
        disable_raw_mode().context(TerminalRestoreSnafu)?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .context(TerminalRestoreSnafu)?;
        terminal.show_cursor().context(TerminalRestoreSnafu)?;

        Ok(())
    }
}

/// Handle keyboard and mouse input
fn handle_input(
    app: Arc<Mutex<App>>,
    tx: broadcast::Sender<Message>,
    redraw_tx: crossbeam::channel::Sender<RedrawMessage>,
) {
    loop {
        if event::poll(Duration::from_millis(16)).unwrap() {
            // ~60fps
            match event::read() {
                Ok(CrosstermEvent::Key(key)) => {
                    let mut app = app.lock().unwrap();
                    match key.code {
                        KeyCode::Enter => {
                            let input = app.get_input().to_string();
                            if !input.is_empty() {
                                app.clear_input();
                                let _ = tx.send(Message::UserTUIInput(input));
                            }
                        }
                        KeyCode::Char(c) => {
                            if c == 'c' && key.modifiers.contains(KeyModifiers::CONTROL) {
                                let _ = tx.send(Message::Action(Action::Exit));
                                let _ = redraw_tx.send(RedrawMessage::End);
                                break;
                            }

                            // Check if we're waiting for confirmation
                            if app.waiting_for_confirmation {
                                if c == 'y' || c == 'Y' {
                                    let _ = tx.send(Message::UserTUIInput("y".to_string()));
                                } else if c == 'n' || c == 'N' {
                                    let _ = tx.send(Message::UserTUIInput("n".to_string()));
                                }
                                // Don't add the character to input when waiting for confirmation
                            } else {
                                app.push_char(c);
                                drop(app);
                                let _ = redraw_tx.send(RedrawMessage::Redraw);
                            }
                        }
                        KeyCode::Backspace => {
                            app.pop_char();
                            drop(app);
                            let _ = redraw_tx.send(RedrawMessage::Redraw);
                        }
                        KeyCode::Up => {
                            app.scroll_up(1);
                            drop(app);
                            let _ = redraw_tx.send(RedrawMessage::Redraw);
                        }
                        KeyCode::Down => {
                            app.scroll_down(1);
                            drop(app);
                            let _ = redraw_tx.send(RedrawMessage::Redraw);
                        }
                        KeyCode::Home => {
                            app.scroll_to_top();
                            drop(app);
                            let _ = redraw_tx.send(RedrawMessage::Redraw);
                        }
                        KeyCode::End => {
                            app.scroll_to_bottom();
                            drop(app);
                            let _ = redraw_tx.send(RedrawMessage::Redraw);
                        }
                        _ => {}
                    }
                }
                Ok(CrosstermEvent::Mouse(mouse)) => match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        let mut app = app.lock().unwrap();
                        app.scroll_up(1);
                        drop(app);
                        let _ = redraw_tx.send(RedrawMessage::Redraw);
                    }
                    MouseEventKind::ScrollDown => {
                        let mut app = app.lock().unwrap();
                        app.scroll_down(1);
                        drop(app);
                        let _ = redraw_tx.send(RedrawMessage::Redraw);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

#[async_trait::async_trait]
impl Actor for TuiActor {
    fn new(config: ParsedConfig, tx: broadcast::Sender<Message>) -> Self {
        let key_bindings = KeyBindingManager::from(&config.keys);

        let (redraw_tx, redraw_rx) = crossbeam::channel::unbounded();

        let s = Self {
            tx,
            config,
            app: Arc::new(Mutex::new(App::new())),
            key_bindings: Arc::new(Mutex::new(key_bindings)),
            redraw_tx: redraw_tx.clone(),
        };

        let s_clone = s.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = s_clone.run_terminal(redraw_tx, redraw_rx) {
                error!("TUI error: {}", e);
            }
        });

        s
    }

    fn get_rx(&self) -> broadcast::Receiver<Message> {
        self.tx.subscribe()
    }

    async fn handle_message(&mut self, message: Message) {
        info!("TUI GOT MESSAGE: {:?}", message);

        // Process messages that affect TUI state
        match message {
            Message::AssistantResponse(content) => {
                let mut app = self.app.lock().unwrap();
                match content {
                    genai::chat::MessageContent::Text(text) => {
                        app.add_event(TuiEvent::assistant_response(text, false));
                        app.waiting_for_response = false;
                    }
                    genai::chat::MessageContent::Parts(parts) => {
                        for part in parts {
                            if let genai::chat::ContentPart::Text(text) = part {
                                app.add_event(TuiEvent::assistant_response(text, false));
                            }
                        }
                        app.waiting_for_response = false;
                    }
                    genai::chat::MessageContent::ToolCalls(_) => {
                        // Tool calls are handled separately
                        app.waiting_for_response = true;
                    }
                    _ => {}
                }
                drop(app);
                self.trigger_redraw(false);
            }
            Message::ToolCallUpdate(update) => {
                let mut app = self.app.lock().unwrap();
                app.track_tool_update(update);
                drop(app);
                self.trigger_redraw(false);
            }
            Message::MicrophoneTranscription(text) => {
                let mut app = self.app.lock().unwrap();
                app.add_event(TuiEvent::user_microphone(text));
                app.add_event(TuiEvent::microphone_stopped());
                drop(app);
                self.trigger_redraw(false);
            }
            Message::Action(Action::CaptureWindow) => {
                let mut app = self.app.lock().unwrap();
                app.add_event(TuiEvent::screenshot("Screenshot captured".to_string()));
                drop(app);
                self.trigger_redraw(false);
            }
            Message::Action(Action::CaptureClipboard) => {
                // Context actor will send the actual clipboard event
            }
            Message::Action(Action::ToggleRecordMicrophone) => {
                let mut app = self.app.lock().unwrap();
                app.add_event(TuiEvent::microphone_started());
                drop(app);
                self.trigger_redraw(false);
            }
            Message::Action(Action::Assist) => {
                let mut app = self.app.lock().unwrap();
                app.waiting_for_response = true;
                drop(app);
                self.trigger_redraw(false);
            }
            Message::Action(Action::Cancel) => {
                let mut app = self.app.lock().unwrap();
                app.waiting_for_response = false;
                app.add_event(TuiEvent::system("Cancelled assistant response".to_string()));
                drop(app);
                self.trigger_redraw(false);
            }
            Message::UserTUIInput(text) => {
                // Only display the input in TUI if it's not a confirmation response
                let mut app = self.app.lock().unwrap();
                if !app.waiting_for_confirmation {
                    app.add_event(TuiEvent::user_input(text));
                    app.clear_input();
                }
                drop(app);
                self.trigger_redraw(false);
            }
            Message::ScreenshotCaptured(result) => {
                let mut app = self.app.lock().unwrap();
                match result {
                    Ok(_base64) => {
                        app.add_event(TuiEvent::screenshot("Screenshot captured".to_string()));
                        // The assistant actor will handle the actual base64 image
                    }
                    Err(error) => {
                        app.add_event(TuiEvent::error(error));
                    }
                }
                drop(app);
                self.trigger_redraw(false);
            }
            Message::ClipboardCaptured(result) => {
                let mut app = self.app.lock().unwrap();
                match result {
                    Ok(text) => {
                        app.add_event(TuiEvent::clipboard(text.clone()));
                        // Also send as user input for the assistant
                        drop(app);
                        let _ = self.tx.send(Message::UserTUIInput(text));
                        self.trigger_redraw(false);
                    }
                    Err(error) => {
                        app.add_event(TuiEvent::error(error));
                        drop(app);
                        self.trigger_redraw(false);
                    }
                }
            }
            Message::PlanUpdated(plan) => {
                let mut app = self.app.lock().unwrap();
                app.add_event(TuiEvent::task_plan_updated(plan));
                drop(app);
                self.trigger_redraw(false);
            }
            _ => {}
        }
    }
}
