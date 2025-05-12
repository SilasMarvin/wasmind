use snafu::{ResultExt, whatever};
use xcap::{Window, image::RgbaImage};

use crate::{SResult, XcapSnafu};

pub fn capture_screen() -> SResult<RgbaImage> {
    let windows = Window::all().context(XcapSnafu)?;

    if let Some(largest_window) = windows
        .iter()
        .filter(|w| w.is_focused().unwrap_or(false) && !w.is_minimized().unwrap_or(true))
        .max_by_key(|w| w.width().unwrap_or(0) * w.height().unwrap_or(0))
    {
        println!(
            "Capturing Window:\n id: {}\n title: {}\n app_name: {}\n monitor: {:?}\n position: {:?}\n size {:?}\n state {:?}\n",
            largest_window.id().context(XcapSnafu)?,
            largest_window.title().context(XcapSnafu)?,
            largest_window.app_name().context(XcapSnafu)?,
            largest_window
                .current_monitor()
                .context(XcapSnafu)?
                .name()
                .context(XcapSnafu)?,
            (
                largest_window.x().context(XcapSnafu)?,
                largest_window.y().context(XcapSnafu)?,
                largest_window.z().context(XcapSnafu)?
            ),
            (
                largest_window.width().context(XcapSnafu)?,
                largest_window.height().context(XcapSnafu)?
            ),
            (
                largest_window.is_minimized().context(XcapSnafu)?,
                largest_window.is_maximized().context(XcapSnafu)?,
                largest_window.is_focused().context(XcapSnafu)?
            )
        );

        largest_window.capture_image().context(XcapSnafu)
    } else {
        whatever!("No focused window found")
    }
}
