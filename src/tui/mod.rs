pub mod app;
pub mod events;
pub mod ui;
pub mod widgets;

use std::io;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crossbeam::channel::{Receiver, Sender};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event as CrosstermEvent, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use snafu::ResultExt;

use self::app::App;
use self::events::TuiEvent;
use crate::{config::ParsedConfig, worker};

/// TUI Error types
#[derive(Debug, snafu::Snafu)]
pub enum TuiError {
    #[snafu(display("Failed to setup terminal"))]
    TerminalSetup { source: io::Error },

    #[snafu(display("Failed to restore terminal"))]
    TerminalRestore { source: io::Error },

    #[snafu(display("Failed to draw frame"))]
    DrawFrame { source: io::Error },

    #[snafu(display("Failed to handle event"))]
    EventHandle { source: io::Error },
}

type TuiResult<T> = Result<T, TuiError>;

/// Tasks the TUI can receive
#[derive(Debug, Clone)]
pub enum Task {
    AddEvent(TuiEvent),
    UpdateInput(String),
    ClearInput,
    Exit,
}

/// Main TUI execution function
pub fn execute_tui(
    worker_tx: Sender<worker::Event>,
    tui_rx: Receiver<Task>,
    config: ParsedConfig,
) -> TuiResult<()> {
    // Setup terminal
    enable_raw_mode().context(TerminalSetupSnafu)?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).context(TerminalSetupSnafu)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context(TerminalSetupSnafu)?;

    // Create app state
    let app = Arc::new(Mutex::new(App::new()));

    // Run the app
    let res = run_app(&mut terminal, app.clone(), worker_tx, tui_rx, config);

    // Restore terminal
    disable_raw_mode().context(TerminalRestoreSnafu)?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .context(TerminalRestoreSnafu)?;
    terminal.show_cursor().context(TerminalRestoreSnafu)?;

    res
}

/// Main application loop
fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: Arc<Mutex<App>>,
    worker_tx: Sender<worker::Event>,
    tui_rx: Receiver<Task>,
    _config: ParsedConfig,
) -> TuiResult<()> {
    // Spawn input handler thread
    let app_clone = app.clone();
    let worker_tx_clone = worker_tx.clone();
    thread::spawn(move || {
        handle_input(app_clone, worker_tx_clone);
    });

    loop {
        // Draw UI
        terminal
            .draw(|f| {
                let app = app.lock().unwrap();
                ui::draw(f, &*app);
            })
            .context(DrawFrameSnafu)?;

        // Handle TUI tasks with timeout
        match tui_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(task) => {
                let mut app = app.lock().unwrap();
                match task {
                    Task::AddEvent(event) => {
                        app.add_event(event);
                    }
                    Task::UpdateInput(input) => {
                        app.set_input(input);
                    }
                    Task::ClearInput => {
                        app.clear_input();
                    }
                    Task::Exit => {
                        break;
                    }
                }
            }
            Err(_) => {
                // Timeout is fine, just continue
            }
        }
    }

    Ok(())
}

/// Handle keyboard input
fn handle_input(app: Arc<Mutex<App>>, worker_tx: Sender<worker::Event>) {
    loop {
        if event::poll(Duration::from_millis(100)).unwrap() {
            if let Ok(CrosstermEvent::Key(key)) = event::read() {
                let mut app = app.lock().unwrap();
                match key.code {
                    KeyCode::Enter => {
                        let input = app.get_input().to_string();
                        if !input.is_empty() {
                            app.clear_input();
                            let _ = worker_tx.send(worker::Event::UserTUIInput(input));
                        }
                    }
                    KeyCode::Char(c) => {
                        app.push_char(c);
                    }
                    KeyCode::Backspace => {
                        app.pop_char();
                    }
                    KeyCode::Esc => {
                        // Could send cancel event here
                    }
                    _ => {}
                }
            }
        }
    }
}
