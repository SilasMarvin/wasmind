use base64::{Engine, engine::general_purpose::STANDARD};
use crossbeam::channel::{Receiver, Sender, unbounded};
use genai::chat::{ChatMessage, ChatRequest, ChatStreamEvent, ContentPart, MessageContent};
use image::ImageFormat;
use snafu::ResultExt;
use std::io::Cursor;
use tracing::error;

use crate::{
    SResult, assistant,
    config::ParsedConfig,
    context::{clipboard::capture_clipboard, screen::capture_screen},
    tui,
};

/// All available events the worker can handle
#[derive(Debug)]
pub enum Event {
    UserTUIInput(String),
    Action(Action),
    ChatStreamEvent(ChatStreamEvent),
}

/// Actions the worker can perform and users can bind keys to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    CaptureWindow,
    CaptureClipboard,
    Assist,
    CancelAssist,
}

impl Action {
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "CaptureWindow" => Some(Action::CaptureWindow),
            "CaptureClipboard" => Some(Action::CaptureClipboard),
            "Assist" => Some(Action::Assist),
            _ => None,
        }
    }
}

pub fn execute_worker(tx: Sender<Event>, rx: Receiver<Event>, config: ParsedConfig) {
    if let Err(e) = do_execute_worker(tx, rx, config) {
        error!("Error executing worker: {e:?}");
        tui::display_error(&format!("Error executing worker: {e:?}"));
    }
}

pub fn do_execute_worker(
    tx: Sender<Event>,
    rx: Receiver<Event>,
    config: ParsedConfig,
) -> SResult<()> {
    let mut chat_request = ChatRequest::default().with_system(&config.model.system_prompt);
    let mut parts = vec![];

    let (assistant_tx, assistant_rx) = unbounded();
    let local_worker_tx = tx.clone();
    let local_config = config.clone();
    let _assistant_handle = std::thread::spawn(move || {
        assistant::execute_assistant(local_worker_tx, assistant_rx, local_config);
    });

    let mut waiting_for_assistant_response = false;

    tui::display_user_prompt();
    while let Ok(task) = rx.recv() {
        match task {
            Event::UserTUIInput(text) => {
                tui::display_user_prompt();
                parts.push(ContentPart::from_text(text));
                // This is kind of silly but rust ownership is being annoying
                tx.send(Event::Action(Action::Assist))
                    .whatever_context("Error sending assist event to worker from worker")?;
            }
            Event::Action(action) => match action {
                Action::CaptureWindow => {
                    let image = capture_screen()?;
                    let mut buffer = Cursor::new(Vec::new());
                    image.write_to(&mut buffer, ImageFormat::Png).unwrap();
                    let base64 = STANDARD.encode(buffer.into_inner());
                    parts.push(ContentPart::from_image_base64("image/png", base64.clone()));
                    tui::display_screenshot(&format!("Screenshot_FILLER",));
                }
                Action::CaptureClipboard => {
                    let text = capture_clipboard()?;
                    parts.push(ContentPart::from_text(text.clone()));
                    tui::display_clipboard_excerpt(&text);
                }
                Action::Assist => {
                    eprintln!("GOT ASSIST");
                    if waiting_for_assistant_response {
                        continue;
                    }
                    // Excute the assistant
                    chat_request = chat_request
                        .append_message(ChatMessage::user(MessageContent::Parts(parts)));
                    assistant_tx
                        .send(assistant::Task::Assist(chat_request.clone()))
                        .whatever_context("Error sending assist request to the assistant")?;
                    parts = vec![];
                    waiting_for_assistant_response = true;
                    tui::display_assistant_start();
                }
                Action::CancelAssist => {
                    if waiting_for_assistant_response {
                        assistant_tx
                            .send(assistant::Task::Cancel)
                            .whatever_context(
                                "Error sending cancel assist request to the assistant",
                            )?;
                        waiting_for_assistant_response = false;
                        tui::display_user_prompt();
                    }
                }
            },
            Event::ChatStreamEvent(event) => match event {
                ChatStreamEvent::Start => (),
                ChatStreamEvent::Chunk(stream_chunk) => {
                    tui::display_text(&stream_chunk.content);
                }
                ChatStreamEvent::ReasoningChunk(stream_chunk) => {
                    tui::display_text(&stream_chunk.content);
                }
                ChatStreamEvent::End(_) => {
                    waiting_for_assistant_response = false;
                    tui::display_done_marker();
                    tui::display_user_prompt();
                }
            },
        }
    }

    Ok(())
}
