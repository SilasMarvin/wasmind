use base64::{Engine, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageFormat, imageops::FilterType};
use std::io::Cursor;
use tokio::sync::broadcast;
use tracing::{error, info};
use xcap::{Window, image::RgbaImage};

use crate::{
    actors::{Action, Actor, Message},
    config::ParsedConfig,
};

/// Context actor that handles screen and clipboard capture
pub struct Context {
    tx: broadcast::Sender<Message>,
    #[allow(dead_code)] // TODO: Use for screenshot size, quality settings
    config: ParsedConfig,
}

impl Context {
    const MAX_SIZE: u32 = 1024;

    fn capture_screen() -> Result<RgbaImage, String> {
        let windows = Window::all().map_err(|e| format!("Failed to get windows: {}", e))?;

        if let Some(largest_window) = windows
            .iter()
            .filter(|w| w.is_focused().unwrap_or(false) && !w.is_minimized().unwrap_or(true))
            .max_by_key(|w| w.width().unwrap_or(0) * w.height().unwrap_or(0))
        {
            let image = largest_window
                .capture_image()
                .map_err(|e| format!("Failed to capture image: {}", e))?;

            let (width, height) = (image.width(), image.height());

            let scale = if width > height {
                Self::MAX_SIZE as f32 / width as f32
            } else {
                Self::MAX_SIZE as f32 / height as f32
            };

            if scale < 1.0 {
                let new_width = (width as f32 * scale) as u32;
                let new_height = (height as f32 * scale) as u32;
                let dynamic_image = DynamicImage::ImageRgba8(image);
                let resized = dynamic_image.resize(new_width, new_height, FilterType::Lanczos3);
                Ok(resized.to_rgba8())
            } else {
                Ok(image)
            }
        } else {
            Err("No focused window found".to_string())
        }
    }

    fn capture_clipboard() -> Result<String, String> {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| format!("Failed to access clipboard: {}", e))?;
        let text = clipboard
            .get_text()
            .map_err(|e| format!("Failed to get clipboard text: {}", e))?;
        Ok(text)
    }
    async fn handle_capture_window(&mut self) {
        info!("Capturing window");
        match Self::capture_screen() {
            Ok(image) => {
                let mut buffer = Cursor::new(Vec::new());
                if let Err(e) = image.write_to(&mut buffer, ImageFormat::Png) {
                    error!("Failed to encode screenshot: {}", e);
                    let _ = self.tx.send(Message::ScreenshotCaptured(Err(format!(
                        "Failed to encode screenshot: {}",
                        e
                    ))));
                    return;
                }
                let base64 = STANDARD.encode(buffer.into_inner());
                let _ = self.tx.send(Message::ScreenshotCaptured(Ok(base64)));
            }
            Err(e) => {
                error!("Failed to capture screen: {}", e);
                let _ = self.tx.send(Message::ScreenshotCaptured(Err(format!(
                    "Failed to capture screen: {}",
                    e
                ))));
            }
        }
    }

    async fn handle_capture_clipboard(&mut self) {
        info!("Capturing clipboard");
        match Self::capture_clipboard() {
            Ok(text) => {
                let _ = self.tx.send(Message::ClipboardCaptured(Ok(text)));
            }
            Err(e) => {
                error!("Failed to capture clipboard: {}", e);
                let _ = self.tx.send(Message::ClipboardCaptured(Err(format!(
                    "Failed to capture clipboard: {}",
                    e
                ))));
            }
        }
    }
}

#[async_trait::async_trait]
impl Actor for Context {
    const ACTOR_ID: &'static str = "context";

    fn new(config: ParsedConfig, tx: broadcast::Sender<Message>) -> Self {
        Self { tx, config }
    }

    fn get_rx(&self) -> broadcast::Receiver<Message> {
        self.tx.subscribe()
    }

    fn get_tx(&self) -> broadcast::Sender<Message> {
        self.tx.clone()
    }

    async fn handle_message(&mut self, message: Message) {
        match message {
            Message::Action(Action::CaptureWindow) => {
                self.handle_capture_window().await;
            }
            Message::Action(Action::CaptureClipboard) => {
                self.handle_capture_clipboard().await;
            }
            _ => {}
        }
    }
}
