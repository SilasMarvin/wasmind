use image::{DynamicImage, imageops::FilterType};
use snafu::{ResultExt, whatever};
use xcap::{Window, image::RgbaImage};

use crate::{SResult, XcapSnafu};

const MAX_SIZE: u32 = 1024;

pub fn capture_screen() -> SResult<RgbaImage> {
    let windows = Window::all().context(XcapSnafu)?;

    if let Some(largest_window) = windows
        .iter()
        .filter(|w| w.is_focused().unwrap_or(false) && !w.is_minimized().unwrap_or(true))
        .max_by_key(|w| w.width().unwrap_or(0) * w.height().unwrap_or(0))
    {
        let image = largest_window.capture_image().context(XcapSnafu)?;

        let (width, height) = (image.width(), image.height());

        let scale = if width > height {
            MAX_SIZE as f32 / width as f32
        } else {
            MAX_SIZE as f32 / height as f32
        };

        if scale < 1.0 {
            let new_width = (width as f32 * scale) as u32;
            let new_height = (height as f32 * scale) as u32;

            let resized_image = image::imageops::resize(
                &DynamicImage::ImageRgba8(image),
                new_width,
                new_height,
                FilterType::Lanczos3,
            );

            Ok(resized_image)
        } else {
            Ok(image)
        }
    } else {
        whatever!("No focused window found")
    }
}
