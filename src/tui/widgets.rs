use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

use super::events::TuiEvent;

// Helper function to skip lines from text and render with appropriate borders
fn render_text_with_skip(
    area: Rect,
    buf: &mut Buffer,
    text: &str,
    skip_lines: usize,
    title: &str,
    border_style: Style,
    text_style: Style,
) {
    let lines: Vec<&str> = text.lines().collect();

    // Calculate total widget height including borders
    let total_height = lines.len() + 2; // +2 for top and bottom borders

    // Determine border type based on skip_lines and available space
    let borders = if skip_lines == 0 {
        if area.height >= total_height as u16 {
            Borders::ALL // Complete message
        } else {
            Borders::TOP | Borders::LEFT | Borders::RIGHT // Bottom cut off
        }
    } else {
        if skip_lines + area.height as usize >= total_height {
            Borders::BOTTOM | Borders::LEFT | Borders::RIGHT // Top cut off, bottom visible
        } else {
            Borders::LEFT | Borders::RIGHT // Both top and bottom cut off
        }
    };

    let block = if skip_lines == 0 {
        Block::default()
            .title(title)
            .borders(borders)
            .border_style(border_style)
    } else {
        Block::default().borders(borders).border_style(border_style)
    };

    let inner = block.inner(area);
    block.render(area, buf);

    // Calculate how many content lines to skip
    // In your giggle example, visible_start is how many lines to skip from the TEXT content
    // skip_lines here represents lines to skip from the ENTIRE widget including borders
    // So we need to convert widget line skipping to content line skipping

    let content_skip = if skip_lines <= 1 {
        // skip_lines 0 = show full widget, skip_lines 1 = skip just the top border
        0
    } else {
        // skip_lines > 1 means we're skipping border + some content
        skip_lines - 1 // Subtract 1 for the top border
    };

    // Skip content lines and take what fits
    let visible_lines: Vec<&str> = lines
        .iter()
        .skip(content_skip)
        .take(inner.height as usize)
        .copied()
        .collect();

    if !visible_lines.is_empty() {
        let paragraph = Paragraph::new(visible_lines.join("\n"))
            .style(text_style)
            .wrap(Wrap { trim: false });
        paragraph.render(inner, buf);
    }
}

/// Trait for rendering TUI events as widgets
pub trait EventWidget {
    fn render_with_skip(&self, area: Rect, buf: &mut Buffer, skip_lines: usize);
    fn height(&self, width: u16) -> u16;
}

impl EventWidget for TuiEvent {
    fn render_with_skip(&self, area: Rect, buf: &mut Buffer, skip_lines: usize) {
        match self {
            TuiEvent::UserInput { text, timestamp } => {
                UserInputWidget { text, timestamp }.render_with_skip(area, buf, skip_lines);
            }
            TuiEvent::UserMicrophoneInput { text, timestamp } => {
                UserMicrophoneWidget { text, timestamp }.render_with_skip(area, buf, skip_lines);
            }
            TuiEvent::AssistantResponse {
                text, timestamp, ..
            } => {
                AssistantResponseWidget { text, timestamp }.render_with_skip(area, buf, skip_lines);
            }
            TuiEvent::Screenshot { name, timestamp } => {
                ScreenshotWidget { name, timestamp }.render_with_skip(area, buf, skip_lines);
            }
            TuiEvent::ClipboardCapture {
                excerpt, timestamp, ..
            } => {
                ClipboardWidget { excerpt, timestamp }.render_with_skip(area, buf, skip_lines);
            }
            TuiEvent::FunctionCall {
                name,
                args,
                timestamp,
            } => {
                FunctionCallWidget {
                    name,
                    args,
                    timestamp,
                }
                .render_with_skip(area, buf, skip_lines);
            }
            TuiEvent::FunctionResult {
                name,
                result,
                timestamp,
            } => {
                FunctionResultWidget {
                    name,
                    result,
                    timestamp,
                }
                .render_with_skip(area, buf, skip_lines);
            }
            TuiEvent::CommandPrompt {
                command,
                args,
                timestamp,
            } => {
                CommandPromptWidget {
                    command,
                    args,
                    timestamp,
                }
                .render_with_skip(area, buf, skip_lines);
            }
            TuiEvent::CommandResult {
                command,
                stdout,
                stderr,
                exit_code,
                timestamp,
            } => {
                CommandResultWidget {
                    command,
                    stdout,
                    stderr,
                    exit_code,
                    timestamp,
                }
                .render_with_skip(area, buf, skip_lines);
            }
            TuiEvent::Error { message, timestamp } => {
                ErrorWidget { message, timestamp }.render_with_skip(area, buf, skip_lines);
            }
            TuiEvent::SystemMessage { message, timestamp } => {
                SystemWidget { message, timestamp }.render_with_skip(area, buf, skip_lines);
            }
            TuiEvent::SetWaitingForResponse { .. } | TuiEvent::SetWaitingForConfirmation { .. } => {
                // These are state changes, not rendered
            }
        }
    }

    fn height(&self, width: u16) -> u16 {
        // Account for borders and padding
        let inner_width = width.saturating_sub(2) as usize;
        if inner_width == 0 {
            return 3; // Minimum height with borders
        }

        match self {
            TuiEvent::UserInput { text, .. }
            | TuiEvent::UserMicrophoneInput { text, .. }
            | TuiEvent::AssistantResponse { text, .. } => {
                // Calculate wrapped lines
                let mut total_lines = 0;
                for line in text.lines() {
                    if line.is_empty() {
                        total_lines += 1;
                    } else {
                        // Simple wrap calculation - divide line length by width
                        total_lines += (line.len() + inner_width - 1) / inner_width;
                    }
                }
                total_lines.max(1) as u16 + 2 // +2 for borders
            }
            TuiEvent::CommandPrompt { command, args, .. } => {
                // Calculate height for command prompt with confirmation text
                let prompt_text = format!(
                    "🔸 $ {} {}\nAllow execution? (y/n)",
                    command,
                    args.join(" ")
                );
                let mut total_lines = 0;
                for line in prompt_text.lines() {
                    if line.is_empty() {
                        total_lines += 1;
                    } else {
                        total_lines += (line.len() + inner_width - 1) / inner_width;
                    }
                }
                total_lines.max(1) as u16 + 2 // +2 for borders
            }
            TuiEvent::CommandResult { stdout, stderr, .. } => {
                let mut total_lines = 1; // Exit code line

                if !stdout.is_empty() {
                    total_lines += 1; // STDOUT: header
                    let stdout_lines: Vec<&str> = stdout.lines().collect();
                    let displayed_lines = stdout_lines.len().min(4);
                    for line in stdout_lines.iter().take(displayed_lines) {
                        if line.is_empty() {
                            total_lines += 1;
                        } else {
                            total_lines += (line.len() + inner_width - 1) / inner_width;
                        }
                    }
                    if stdout_lines.len() > 4 {
                        total_lines += 1; // For the "... (X more lines)" message
                    }
                }

                if !stderr.is_empty() {
                    total_lines += 1; // STDERR: header
                    let stderr_lines: Vec<&str> = stderr.lines().collect();
                    let displayed_lines = stderr_lines.len().min(4);
                    for line in stderr_lines.iter().take(displayed_lines) {
                        if line.is_empty() {
                            total_lines += 1;
                        } else {
                            total_lines += (line.len() + inner_width - 1) / inner_width;
                        }
                    }
                    if stderr_lines.len() > 4 {
                        total_lines += 1; // For the "... (X more lines)" message
                    }
                }

                total_lines as u16 + 2 // +2 for borders
            }
            TuiEvent::SystemMessage { message, .. } => {
                // Calculate wrapped lines for system messages
                let mut total_lines = 0;
                for line in message.lines() {
                    if line.is_empty() {
                        total_lines += 1;
                    } else {
                        // Simple wrap calculation - divide line length by width
                        total_lines += (line.len() + inner_width - 1) / inner_width;
                    }
                }
                total_lines.max(1) as u16 + 2 // +2 for borders
            }
            TuiEvent::SetWaitingForResponse { .. } | TuiEvent::SetWaitingForConfirmation { .. } => {
                0
            }
            _ => 3, // Default height for simple widgets
        }
    }
}

// Individual widget implementations

struct UserInputWidget<'a> {
    text: &'a str,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> UserInputWidget<'a> {
    fn render_with_skip(&self, area: Rect, buf: &mut Buffer, skip_lines: usize) {
        render_text_with_skip(
            area,
            buf,
            self.text,
            skip_lines,
            &format!("User [{}]", self.timestamp.format("%H:%M:%S")),
            Style::default().fg(Color::Blue),
            Style::default(),
        );
    }
}

impl<'a> Widget for UserInputWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_with_skip(area, buf, 0);
    }
}

struct UserMicrophoneWidget<'a> {
    text: &'a str,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> UserMicrophoneWidget<'a> {
    fn render_with_skip(&self, area: Rect, buf: &mut Buffer, skip_lines: usize) {
        render_text_with_skip(
            area,
            buf,
            self.text,
            skip_lines,
            &format!("User (Microphone) [{}]", self.timestamp.format("%H:%M:%S")),
            Style::default().fg(Color::Cyan),
            Style::default(),
        );
    }
}

impl<'a> Widget for UserMicrophoneWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_with_skip(area, buf, 0);
    }
}

struct AssistantResponseWidget<'a> {
    text: &'a str,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> AssistantResponseWidget<'a> {
    fn render_with_skip(&self, area: Rect, buf: &mut Buffer, skip_lines: usize) {
        render_text_with_skip(
            area,
            buf,
            self.text,
            skip_lines,
            &format!("Assistant [{}]", self.timestamp.format("%H:%M:%S")),
            Style::default().fg(Color::Green),
            Style::default(),
        );
    }
}

impl<'a> Widget for AssistantResponseWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_with_skip(area, buf, 0);
    }
}

struct ScreenshotWidget<'a> {
    name: &'a str,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> ScreenshotWidget<'a> {
    fn render_with_skip(&self, area: Rect, buf: &mut Buffer, skip_lines: usize) {
        render_text_with_skip(
            area,
            buf,
            &format!("📸 {}", self.name),
            skip_lines,
            &format!("Screenshot [{}]", self.timestamp.format("%H:%M:%S")),
            Style::default().fg(Color::Yellow),
            Style::default().fg(Color::Yellow),
        );
    }
}

impl<'a> Widget for ScreenshotWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_with_skip(area, buf, 0);
    }
}

struct ClipboardWidget<'a> {
    excerpt: &'a str,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> ClipboardWidget<'a> {
    fn render_with_skip(&self, area: Rect, buf: &mut Buffer, skip_lines: usize) {
        render_text_with_skip(
            area,
            buf,
            &format!("📋 {}...", self.excerpt),
            skip_lines,
            &format!("Clipboard [{}]", self.timestamp.format("%H:%M:%S")),
            Style::default().fg(Color::Magenta),
            Style::default().fg(Color::Magenta),
        );
    }
}

impl<'a> Widget for ClipboardWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_with_skip(area, buf, 0);
    }
}

struct FunctionCallWidget<'a> {
    name: &'a str,
    args: &'a Option<String>,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> FunctionCallWidget<'a> {
    fn render_with_skip(&self, area: Rect, buf: &mut Buffer, skip_lines: usize) {
        let text = if let Some(args) = self.args {
            format!("⚡ {}({})", self.name, args)
        } else {
            format!("⚡ {}()", self.name)
        };

        render_text_with_skip(
            area,
            buf,
            &text,
            skip_lines,
            &format!("Function Call [{}]", self.timestamp.format("%H:%M:%S")),
            Style::default().fg(Color::LightBlue),
            Style::default().fg(Color::LightBlue),
        );
    }
}

impl<'a> Widget for FunctionCallWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_with_skip(area, buf, 0);
    }
}

struct FunctionResultWidget<'a> {
    name: &'a str,
    result: &'a str,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> FunctionResultWidget<'a> {
    fn render_with_skip(&self, area: Rect, buf: &mut Buffer, skip_lines: usize) {
        render_text_with_skip(
            area,
            buf,
            &format!("✓ {}: {}", self.name, self.result),
            skip_lines,
            &format!("Function Result [{}]", self.timestamp.format("%H:%M:%S")),
            Style::default().fg(Color::DarkGray),
            Style::default().fg(Color::Gray),
        );
    }
}

impl<'a> Widget for FunctionResultWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_with_skip(area, buf, 0);
    }
}

struct CommandPromptWidget<'a> {
    command: &'a str,
    args: &'a [String],
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> CommandPromptWidget<'a> {
    fn render_with_skip(&self, area: Rect, buf: &mut Buffer, skip_lines: usize) {
        render_text_with_skip(
            area,
            buf,
            &format!(
                "🔸 $ {} {}\nAllow execution? (y/n)",
                self.command,
                self.args.join(" ")
            ),
            skip_lines,
            &format!("Command Prompt [{}]", self.timestamp.format("%H:%M:%S")),
            Style::default().fg(Color::Red),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        );
    }
}

impl<'a> Widget for CommandPromptWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_with_skip(area, buf, 0);
    }
}

struct CommandResultWidget<'a> {
    command: &'a str,
    stdout: &'a str,
    stderr: &'a str,
    exit_code: &'a i32,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> CommandResultWidget<'a> {
    fn render_with_skip(&self, area: Rect, buf: &mut Buffer, skip_lines: usize) {
        // Build all lines first
        let mut all_lines = vec![];
        all_lines.push(format!("Exit code: {}", self.exit_code));

        if !self.stdout.is_empty() {
            all_lines.push("STDOUT:".to_string());
            let stdout_lines: Vec<&str> = self.stdout.lines().collect();
            if stdout_lines.len() <= 4 {
                for line in stdout_lines {
                    all_lines.push(line.to_string());
                }
            } else {
                // Show first 4 lines and add "..."
                for line in stdout_lines.iter().take(4) {
                    all_lines.push(line.to_string());
                }
                all_lines.push(format!("... ({} more lines)", stdout_lines.len() - 4));
            }
        }

        if !self.stderr.is_empty() {
            all_lines.push("STDERR:".to_string());
            let stderr_lines: Vec<&str> = self.stderr.lines().collect();
            if stderr_lines.len() <= 4 {
                for line in stderr_lines {
                    all_lines.push(line.to_string());
                }
            } else {
                // Show first 4 lines and add "..."
                for line in stderr_lines.iter().take(4) {
                    all_lines.push(line.to_string());
                }
                all_lines.push(format!("... ({} more lines)", stderr_lines.len() - 4));
            }
        }

        // Calculate total widget height including borders
        let total_height = all_lines.len() + 2; // +2 for top and bottom borders

        // Determine border type based on skip_lines and available space
        let borders = if skip_lines == 0 {
            if area.height >= total_height as u16 {
                Borders::ALL // Complete message
            } else {
                Borders::TOP | Borders::LEFT | Borders::RIGHT // Bottom cut off
            }
        } else {
            if skip_lines + area.height as usize >= total_height {
                Borders::BOTTOM | Borders::LEFT | Borders::RIGHT // Top cut off, bottom visible
            } else {
                Borders::LEFT | Borders::RIGHT // Both top and bottom cut off
            }
        };

        let block = if skip_lines == 0 {
            Block::default()
                .title(format!(
                    "Command Result [{}]",
                    self.timestamp.format("%H:%M:%S")
                ))
                .borders(borders)
                .border_style(Style::default().fg(Color::Gray))
        } else {
            Block::default()
                .borders(borders)
                .border_style(Style::default().fg(Color::Gray))
        };

        let inner = block.inner(area);
        block.render(area, buf);

        // Calculate how many content lines to skip
        let content_skip = if skip_lines <= 1 { 0 } else { skip_lines - 1 };

        // Skip content lines and take what fits, then rebuild styled lines
        let visible_lines: Vec<String> = all_lines
            .iter()
            .skip(content_skip)
            .take(inner.height as usize)
            .cloned()
            .collect();

        if !visible_lines.is_empty() {
            let mut styled_lines = vec![];
            for (i, line) in visible_lines.iter().enumerate() {
                let actual_line_index = content_skip + i;
                if actual_line_index == 0 {
                    // Exit code line
                    styled_lines.push(Line::from(vec![
                        Span::raw("Exit code: "),
                        Span::styled(
                            self.exit_code.to_string(),
                            if *self.exit_code == 0 {
                                Style::default().fg(Color::Green)
                            } else {
                                Style::default().fg(Color::Red)
                            },
                        ),
                    ]));
                } else if line == "STDOUT:" {
                    styled_lines.push(Line::from(Span::styled(
                        line,
                        Style::default().fg(Color::Green),
                    )));
                } else if line == "STDERR:" {
                    styled_lines.push(Line::from(Span::styled(
                        line,
                        Style::default().fg(Color::Red),
                    )));
                } else {
                    styled_lines.push(Line::from(Span::raw(line)));
                }
            }

            let paragraph = Paragraph::new(styled_lines).wrap(Wrap { trim: true });
            paragraph.render(inner, buf);
        }
    }
}

impl<'a> Widget for CommandResultWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_with_skip(area, buf, 0);
    }
}

struct ErrorWidget<'a> {
    message: &'a str,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> ErrorWidget<'a> {
    fn render_with_skip(&self, area: Rect, buf: &mut Buffer, skip_lines: usize) {
        render_text_with_skip(
            area,
            buf,
            &format!("❌ {}", self.message),
            skip_lines,
            &format!("Error [{}]", self.timestamp.format("%H:%M:%S")),
            Style::default().fg(Color::Red),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        );
    }
}

impl<'a> Widget for ErrorWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_with_skip(area, buf, 0);
    }
}

struct SystemWidget<'a> {
    message: &'a str,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> SystemWidget<'a> {
    fn render_with_skip(&self, area: Rect, buf: &mut Buffer, skip_lines: usize) {
        render_text_with_skip(
            area,
            buf,
            self.message,
            skip_lines,
            &format!("System [{}]", self.timestamp.format("%H:%M:%S")),
            Style::default().fg(Color::DarkGray),
            Style::default().fg(Color::DarkGray),
        );
    }
}

impl<'a> Widget for SystemWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_with_skip(area, buf, 0);
    }
}
