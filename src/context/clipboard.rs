use snafu::ResultExt;

use crate::{ClipboardSnafu, SResult};

pub fn capture_clipboard() -> SResult<String> {
    let mut clipboard = arboard::Clipboard::new().context(ClipboardSnafu)?;
    let text = clipboard.get_text().context(ClipboardSnafu)?;
    Ok(text)
}
