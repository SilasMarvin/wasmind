use base64::{Engine, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageFormat, imageops::FilterType};
use std::io::Cursor;
use tokio::sync::broadcast;
use tracing::error;
use xcap::{Window, image::RgbaImage};

use crate::{
    actors::{Actor, ActorContext, Message, UserContext},
    config::ParsedConfig,
    scope::Scope,
};

use super::ActorMessage;

/// Context actor that handles screen and clipboard capture
#[derive(hive_macros::ActorContext)]
pub struct Context {
    tx: broadcast::Sender<ActorMessage>,
    #[allow(dead_code)] // TODO: Use for screenshot size, quality settings
    config: ParsedConfig,
    scope: Scope,
}

impl Context {
    const MAX_SIZE: u32 = 1024;

    pub fn new(config: ParsedConfig, tx: broadcast::Sender<ActorMessage>, scope: Scope) -> Self {
        Self { tx, config, scope }
    }

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

    async fn handle_capture_window(&mut self) {
        match Self::capture_screen() {
            Ok(image) => {
                let mut buffer = Cursor::new(Vec::new());
                if let Err(e) = image.write_to(&mut buffer, ImageFormat::Png) {
                    error!("Failed to encode screenshot: {}", e);
                    self.broadcast(Message::UserContext(UserContext::ScreenshotCaptured(Err(
                        format!("Failed to encode screenshot: {}", e),
                    ))));
                    return;
                }
                let base64 = STANDARD.encode(buffer.into_inner());
                self.broadcast(Message::UserContext(UserContext::ScreenshotCaptured(Ok(
                    base64,
                ))));
            }
            Err(e) => {
                error!("Failed to capture screen: {}", e);
                self.broadcast(Message::UserContext(UserContext::ScreenshotCaptured(Err(
                    format!("Failed to capture screen: {}", e),
                ))));
            }
        }
    }

    async fn handle_capture_clipboard(&mut self) {
        match Self::capture_clipboard() {
            Ok(text) => {
                self.broadcast(Message::UserContext(UserContext::ClipboardCaptured(Ok(
                    text,
                ))));
            }
            Err(e) => {
                error!("Failed to capture clipboard: {}", e);
                self.broadcast(Message::UserContext(UserContext::ClipboardCaptured(Err(
                    format!("Failed to capture clipboard: {}", e),
                ))));
            }
        }
    }
}

#[async_trait::async_trait]
impl Actor for Context {
    const ACTOR_ID: &'static str = "context";

    async fn handle_message(&mut self, _message: ActorMessage) {
        todo!()
    }
}
