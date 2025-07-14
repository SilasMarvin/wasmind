use model::Model;
use ratatui::crossterm::{
    event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
    execute,
};
use std::{io::stdout, thread};
use tokio::sync::broadcast::Sender;
use tuirealm::{PollStrategy, Update};

pub mod components;
pub mod icons;
mod model;
mod throbber_in_title_ext;
mod utils;

use crate::{
    config::{ParsedConfig, ParsedTuiConfig},
    scope::Scope,
};

use super::{Actor, ActorContext, ActorMessage};

#[derive(hive_macros::ActorContext)]
pub struct TuiActor {
    #[allow(dead_code)]
    config: ParsedConfig,
    tx: Sender<ActorMessage>,
    scope: Scope,
}

impl TuiActor {
    pub fn new(
        config: ParsedConfig,
        tx: Sender<ActorMessage>,
        scope: Scope,
        initial_prompt: Option<String>,
    ) -> Self {
        let local_tx = tx.clone();
        let tui_config = config.tui.clone();
        thread::spawn(|| start_model(tui_config, local_tx, initial_prompt));

        Self {
            config,
            tx,
            scope,
        }
    }
}

#[async_trait::async_trait]
impl Actor for TuiActor {
    const ACTOR_ID: &'static str = "tui";

    async fn handle_message(&mut self, _message: ActorMessage) {}
}

fn start_model(config: ParsedTuiConfig, tx: Sender<ActorMessage>, initial_prompt: Option<String>) {
    let mut stdout = stdout();
    if let Err(e) = execute!(
        stdout,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    ) {
        tracing::error!(
            "Error enabling the Kitty Keyboard Protocol - some key bindings may not work as expected. See: https://sw.kovidgoyal.net/kitty/keyboard-protocol Error: {e:?}"
        );
    }

    // Setup model
    let mut model = Model::new(config, tx, initial_prompt);
    // Enter alternate screen
    let _ = model.terminal.enter_alternate_screen();
    let _ = model.terminal.enable_raw_mode();
    // Main loop
    // NOTE: loop until quit; quit is set in update if AppClose is received from counter
    while !model.quit {
        // Tick
        match model.app.tick(PollStrategy::Once) {
            Err(err) => {
                tracing::error!("{err:?}");
            }
            Ok(messages) if !messages.is_empty() => {
                // NOTE: redraw if at least one msg has been processed
                model.redraw = true;
                for msg in messages {
                    let mut msg = Some(msg);
                    while msg.is_some() {
                        msg = model.update(msg);
                    }
                }
            }
            _ => {}
        }
        // Redraw
        if model.redraw {
            model.view();
            model.redraw = false;
        }
    }
    // Terminate terminal
    let _ = model.terminal.leave_alternate_screen();
    let _ = model.terminal.disable_raw_mode();
    let _ = model.terminal.clear_screen();

    execute!(stdout, PopKeyboardEnhancementFlags).ok();
}
