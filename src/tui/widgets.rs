use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

use super::events::TuiEvent;

/// Trait for rendering TUI events as widgets
pub trait EventWidget {
    fn render(&self, area: Rect, buf: &mut Buffer);
    fn height(&self) -> u16;
}

impl EventWidget for TuiEvent {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        match self {
            TuiEvent::UserInput { text, timestamp } => {
                UserInputWidget { text, timestamp }.render(area, buf);
            }
            TuiEvent::UserMicrophoneInput { text, timestamp } => {
                UserMicrophoneWidget { text, timestamp }.render(area, buf);
            }
            TuiEvent::AssistantResponse { text, timestamp, .. } => {
                AssistantResponseWidget { text, timestamp }.render(area, buf);
            }
            TuiEvent::Screenshot { name, timestamp } => {
                ScreenshotWidget { name, timestamp }.render(area, buf);
            }
            TuiEvent::ClipboardCapture { excerpt, timestamp, .. } => {
                ClipboardWidget { excerpt, timestamp }.render(area, buf);
            }
            TuiEvent::FunctionCall { name, args, timestamp } => {
                FunctionCallWidget { name, args, timestamp }.render(area, buf);
            }
            TuiEvent::FunctionResult { name, result, timestamp } => {
                FunctionResultWidget { name, result, timestamp }.render(area, buf);
            }
            TuiEvent::CommandPrompt { command, args, timestamp } => {
                CommandPromptWidget { command, args, timestamp }.render(area, buf);
            }
            TuiEvent::CommandResult { command, stdout, stderr, exit_code, timestamp } => {
                CommandResultWidget { command, stdout, stderr, exit_code, timestamp }.render(area, buf);
            }
            TuiEvent::Error { message, timestamp } => {
                ErrorWidget { message, timestamp }.render(area, buf);
            }
            TuiEvent::SystemMessage { message, timestamp } => {
                SystemWidget { message, timestamp }.render(area, buf);
            }
            TuiEvent::SetWaitingForResponse { .. } | TuiEvent::SetWaitingForConfirmation { .. } => {
                // These are state changes, not rendered
            }
        }
    }

    fn height(&self) -> u16 {
        match self {
            TuiEvent::UserInput { text, .. } => text.lines().count() as u16 + 2,
            TuiEvent::UserMicrophoneInput { text, .. } => text.lines().count() as u16 + 2,
            TuiEvent::AssistantResponse { text, .. } => text.lines().count() as u16 + 2,
            TuiEvent::CommandResult { stdout, stderr, .. } => {
                let stdout_lines = if stdout.is_empty() { 0 } else { stdout.lines().count() + 1 };
                let stderr_lines = if stderr.is_empty() { 0 } else { stderr.lines().count() + 1 };
                (stdout_lines + stderr_lines + 3) as u16
            }
            TuiEvent::SetWaitingForResponse { .. } | TuiEvent::SetWaitingForConfirmation { .. } => 0,
            _ => 3, // Default height for simple widgets
        }
    }
}

// Individual widget implementations

struct UserInputWidget<'a> {
    text: &'a str,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> Widget for UserInputWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(format!("User [{}]", self.timestamp.format("%H:%M:%S")))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Blue));

        let inner = block.inner(area);
        block.render(area, buf);

        let paragraph = Paragraph::new(self.text)
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: true });
        paragraph.render(inner, buf);
    }
}

struct UserMicrophoneWidget<'a> {
    text: &'a str,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> Widget for UserMicrophoneWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(format!("User (Microphone) [{}]", self.timestamp.format("%H:%M:%S")))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(area);
        block.render(area, buf);

        let paragraph = Paragraph::new(self.text)
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: true });
        paragraph.render(inner, buf);
    }
}

struct AssistantResponseWidget<'a> {
    text: &'a str,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> Widget for AssistantResponseWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(format!("Assistant [{}]", self.timestamp.format("%H:%M:%S")))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Green));

        let inner = block.inner(area);
        block.render(area, buf);

        let paragraph = Paragraph::new(self.text)
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: true });
        paragraph.render(inner, buf);
    }
}

struct ScreenshotWidget<'a> {
    name: &'a str,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> Widget for ScreenshotWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(format!("Screenshot [{}]", self.timestamp.format("%H:%M:%S")))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));

        let inner = block.inner(area);
        block.render(area, buf);

        let paragraph = Paragraph::new(format!("📸 {}", self.name))
            .style(Style::default().fg(Color::Yellow));
        paragraph.render(inner, buf);
    }
}

struct ClipboardWidget<'a> {
    excerpt: &'a str,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> Widget for ClipboardWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(format!("Clipboard [{}]", self.timestamp.format("%H:%M:%S")))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta));

        let inner = block.inner(area);
        block.render(area, buf);

        let paragraph = Paragraph::new(format!("📋 {}...", self.excerpt))
            .style(Style::default().fg(Color::Magenta));
        paragraph.render(inner, buf);
    }
}

struct FunctionCallWidget<'a> {
    name: &'a str,
    args: &'a Option<String>,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> Widget for FunctionCallWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(format!("Function Call [{}]", self.timestamp.format("%H:%M:%S")))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::LightBlue));

        let inner = block.inner(area);
        block.render(area, buf);

        let text = if let Some(args) = self.args {
            format!("⚡ {}({})", self.name, args)
        } else {
            format!("⚡ {}()", self.name)
        };

        let paragraph = Paragraph::new(text)
            .style(Style::default().fg(Color::LightBlue));
        paragraph.render(inner, buf);
    }
}

struct FunctionResultWidget<'a> {
    name: &'a str,
    result: &'a str,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> Widget for FunctionResultWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(format!("Function Result [{}]", self.timestamp.format("%H:%M:%S")))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));

        let inner = block.inner(area);
        block.render(area, buf);

        let paragraph = Paragraph::new(format!("✓ {}: {}", self.name, self.result))
            .style(Style::default().fg(Color::Gray))
            .wrap(Wrap { trim: true });
        paragraph.render(inner, buf);
    }
}

struct CommandPromptWidget<'a> {
    command: &'a str,
    args: &'a [String],
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> Widget for CommandPromptWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(format!("Command Prompt [{}]", self.timestamp.format("%H:%M:%S")))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));

        let inner = block.inner(area);
        block.render(area, buf);

        let paragraph = Paragraph::new(format!("🔸 $ {} {}", self.command, self.args.join(" ")))
            .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
        paragraph.render(inner, buf);
    }
}

struct CommandResultWidget<'a> {
    command: &'a str,
    stdout: &'a str,
    stderr: &'a str,
    exit_code: &'a i32,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> Widget for CommandResultWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(format!("Command Result [{}]", self.timestamp.format("%H:%M:%S")))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Gray));

        let inner = block.inner(area);
        block.render(area, buf);

        let mut lines = vec![];
        lines.push(Line::from(vec![
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

        if !self.stdout.is_empty() {
            lines.push(Line::from(Span::styled("STDOUT:", Style::default().fg(Color::Green))));
            for line in self.stdout.lines() {
                lines.push(Line::from(Span::raw(line)));
            }
        }

        if !self.stderr.is_empty() {
            lines.push(Line::from(Span::styled("STDERR:", Style::default().fg(Color::Red))));
            for line in self.stderr.lines() {
                lines.push(Line::from(Span::raw(line)));
            }
        }

        let paragraph = Paragraph::new(lines)
            .wrap(Wrap { trim: true });
        paragraph.render(inner, buf);
    }
}

struct ErrorWidget<'a> {
    message: &'a str,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> Widget for ErrorWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(format!("Error [{}]", self.timestamp.format("%H:%M:%S")))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));

        let inner = block.inner(area);
        block.render(area, buf);

        let paragraph = Paragraph::new(format!("❌ {}", self.message))
            .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
            .wrap(Wrap { trim: true });
        paragraph.render(inner, buf);
    }
}

struct SystemWidget<'a> {
    message: &'a str,
    timestamp: &'a chrono::DateTime<chrono::Utc>,
}

impl<'a> Widget for SystemWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(format!("System [{}]", self.timestamp.format("%H:%M:%S")))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));

        let inner = block.inner(area);
        block.render(area, buf);

        let paragraph = Paragraph::new(format!("ℹ️  {}", self.message))
            .style(Style::default().fg(Color::Gray))
            .wrap(Wrap { trim: true });
        paragraph.render(inner, buf);
    }
}