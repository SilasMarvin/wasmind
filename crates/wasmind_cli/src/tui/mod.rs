use model::Model;
use ratatui::crossterm::{
    event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
    execute,
};
use std::{io::stdout, sync::Arc, thread};
use tokio::sync::broadcast::{Receiver, Sender};
use tuirealm::{PollStrategy, Update};
use wasmind::actors::MessageEnvelope;
use wasmind::context::WasmindContext;

pub mod components;
pub mod icons;
mod model;
mod throbber_in_title_ext;
mod utils;

use crate::config::ParsedTuiConfig;

pub struct Tui {
    tui_config: ParsedTuiConfig,
    rx: Receiver<MessageEnvelope>,
    initial_prompt: Option<String>,
    context: Arc<WasmindContext>,
}

impl Tui {
    pub fn new(
        tui_config: ParsedTuiConfig,
        tx: Sender<MessageEnvelope>,
        initial_prompt: Option<String>,
        context: Arc<WasmindContext>,
    ) -> Self {
        Self {
            tui_config,
            rx: tx.subscribe(),
            initial_prompt,
            context,
        }
    }

    pub fn run(self) {
        thread::spawn(|| start_model(self.tui_config, self.rx, self.initial_prompt, self.context));
    }
}

fn start_model(
    config: ParsedTuiConfig,
    rx: Receiver<MessageEnvelope>,
    initial_prompt: Option<String>,
    context: Arc<WasmindContext>,
) {
    let mut stdout = stdout();
    if let Err(e) = execute!(
        stdout,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    ) {
        tracing::error!(
            "Error enabling the Kitty Keyboard Protocol - some key bindings may not work as expected. See: https://sw.kovidgoyal.net/kitty/keyboard-protocol Error: {e:?}"
        );
    }

    let mut model = Model::new(config, rx, initial_prompt, context);
    // Enter alternate screen
    let _ = model.terminal.enter_alternate_screen();
    let _ = model.terminal.enable_raw_mode();
    // Main loop
    // NOTE: loop until quit; quit is set in update if AppClose is received from counter
    while !model.quit {
        match model.app.tick(PollStrategy::Once) {
            Err(err) => {
                tracing::error!("{err:?}");
            }
            Ok(messages) if !messages.is_empty() => {
                // NOTE: redraw if at least one msg has been processed
                for msg in messages {
                    model.update(Some(msg));
                }
            }
            _ => {}
        }
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
