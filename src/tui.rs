use crossbeam::channel::Sender;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    Terminal,
};
use std::io::Stdout;
use crossterm::{
    event::{self, Event as CrosstermEvent, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io;
use crate::worker::Event;

pub struct Tui {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    input_buffer: String,
    conversation: Vec<ConversationEntry>,
}

#[derive(Clone)]
pub enum ConversationEntry {
    UserText(String),
    UserClipboard(String),
    UserScreenshot(String),
    Assistant(String),
}

impl Tui {
    pub fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(Self {
            terminal,
            input_buffer: String::new(),
            conversation: Vec::new(),
        })
    }

    pub fn cleanup(&mut self) -> io::Result<()> {
        disable_raw_mode()?;
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)?;
        Ok(())
    }

    pub fn add_user_text(&mut self, text: String) {
        self.conversation.push(ConversationEntry::UserText(text));
        self.render().unwrap();
    }

    pub fn add_clipboard_content(&mut self, content: String) {
        self.conversation.push(ConversationEntry::UserClipboard(content));
        self.render().unwrap();
    }

    pub fn add_screenshot(&mut self, timestamp: String) {
        self.conversation.push(ConversationEntry::UserScreenshot(timestamp));
        self.render().unwrap();
    }

    pub fn add_assistant_response(&mut self, response: String) {
        self.conversation.push(ConversationEntry::Assistant(response));
        self.render().unwrap();
    }

    pub fn handle_input(&mut self, event_tx: &Sender<Event>) -> io::Result<bool> {
        if event::poll(std::time::Duration::from_millis(100))? {
            if let CrosstermEvent::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Enter => {
                        if !self.input_buffer.is_empty() {
                            let input = std::mem::take(&mut self.input_buffer);
                            event_tx.send(Event::UserTUIInput(input)).unwrap();
                            self.render()?;
                        }
                    }
                    KeyCode::Char(c) => {
                        self.input_buffer.push(c);
                        self.render()?;
                    }
                    KeyCode::Backspace => {
                        self.input_buffer.pop();
                        self.render()?;
                    }
                    KeyCode::Esc => return Ok(true),
                    _ => {}
                }
            }
        }
        Ok(false)
    }

    fn render(&mut self) -> io::Result<()> {
        self.terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(1),
                    Constraint::Length(3),
                ])
                .split(frame.size());

            let mut conversation_text = Vec::new();
            for entry in &self.conversation {
                match entry {
                    ConversationEntry::UserText(text) => {
                        conversation_text.push(Line::from(vec![
                            Span::styled("(you) >>> ", Style::default().fg(Color::Green)),
                            Span::raw(text),
                        ]));
                    }
                    ConversationEntry::UserClipboard(content) => {
                        conversation_text.push(Line::from(vec![
                            Span::styled("(you) >>> ", Style::default().fg(Color::Green)),
                            Span::styled("[clipboard] ", Style::default().fg(Color::Yellow)),
                            Span::raw(content),
                        ]));
                    }
                    ConversationEntry::UserScreenshot(timestamp) => {
                        conversation_text.push(Line::from(vec![
                            Span::styled("(you) >>> ", Style::default().fg(Color::Green)),
                            Span::styled(
                                format!("[screenshot @ {}]", timestamp),
                                Style::default().fg(Color::Yellow),
                            ),
                        ]));
                    }
                    ConversationEntry::Assistant(text) => {
                        conversation_text.push(Line::from(vec![
                            Span::styled("(assistant) >>> ", Style::default().fg(Color::Blue)),
                            Span::raw(text),
                        ]));
                    }
                }
            }

            let conversation = Paragraph::new(Text::from(conversation_text))
                .block(Block::default().borders(Borders::ALL))
                .wrap(Wrap { trim: true });

            let input = Paragraph::new(format!("(you) >>> {}", self.input_buffer))
                .style(Style::default())
                .block(Block::default().borders(Borders::ALL))
                .alignment(Alignment::Left);

            frame.render_widget(conversation, chunks[0]);
            frame.render_widget(input, chunks[1]);
        })?;

        Ok(())
    }
}