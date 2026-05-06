//! Terra TUI — ratatui-based terminal user interface
//!
#![allow(dead_code, unused_imports)] // TUI module built but activated only in terminal mode
//! 5-zone layout:
//! ┌──────────────────────────────────────┐
//! │ Logo + Status (model | mode)          │ ← compact
//! ├──────────────────────────────────────┤
//! │ Smart Greeter / Notification area     │ ← 3-5 lines
//! ├──────────────────────────────────────┤
//! │                                      │
//! │ Chat area (scrollable)               │ ← main body
//! │                                      │
//! ├──────────────────────────────────────┤
//! │ Status bar: mode | model | tokens    │ ← 1 line
//! ├──────────────────────────────────────┤
//! │ Input area                           │ ← bottom
//! └──────────────────────────────────────┘
//!
//! Activated only when stdout is a terminal.
//! Falls back to readline REPL when piped.

use std::io;
use std::sync::{Arc, Mutex};

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Terminal;

/// TUI application state.
pub struct TuiApp {
    pub input: String,
    pub cursor_pos: usize,
    pub messages: Vec<ChatMessage>,
    pub status_line: String,
    pub greeter_text: String,
    pub model_name: String,
    pub mode: String,
    pub tokens: u64,
    pub running: bool,
    pub thinking: bool,
}

/// A message in the chat area.
pub struct ChatMessage {
    pub role: String,    // "user", "assistant", "system"
    pub content: String,
}

impl TuiApp {
    pub fn new(model_name: &str, greeter: &str) -> Self {
        Self {
            input: String::new(),
            cursor_pos: 0,
            messages: Vec::new(),
            status_line: String::new(),
            greeter_text: greeter.to_string(),
            model_name: model_name.to_string(),
            mode: "conversation".into(),
            tokens: 0,
            running: true,
            thinking: false,
        }
    }

    pub fn add_message(&mut self, role: &str, content: &str) {
        self.messages.push(ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
        });
    }
}

/// Initialize and run the TUI. Returns when user quits.
pub fn run_tui(app: &mut TuiApp) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_event_loop(&mut terminal, app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

/// Main event loop.
fn run_event_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut TuiApp) -> io::Result<()> {
    loop {
        terminal.draw(|f| draw_ui(f, app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if app.thinking {
                            app.thinking = false;
                            app.add_message("system", "[interrupted]");
                        }
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.running = false;
                        return Ok(());
                    }
                    KeyCode::Enter => {
                        if !app.input.is_empty() {
                            let input = app.input.clone();
                            app.input.clear();
                            app.cursor_pos = 0;

                            if input == "/quit" || input == "/q" {
                                app.running = false;
                                return Ok(());
                            }

                            app.add_message("user", &input);
                            // Note: actual LLM processing would be async
                            // For now, mark as thinking — real integration
                            // requires async channel between TUI and REPL
                            app.thinking = true;
                        }
                    }
                    KeyCode::Char(c) => {
                        app.input.insert(app.cursor_pos, c);
                        app.cursor_pos += 1;
                    }
                    KeyCode::Backspace => {
                        if app.cursor_pos > 0 {
                            app.cursor_pos -= 1;
                            app.input.remove(app.cursor_pos);
                        }
                    }
                    KeyCode::Left => {
                        if app.cursor_pos > 0 { app.cursor_pos -= 1; }
                    }
                    KeyCode::Right => {
                        if app.cursor_pos < app.input.len() { app.cursor_pos += 1; }
                    }
                    _ => {}
                }
            }
        }

        if !app.running { break; }
    }
    Ok(())
}

/// Draw the 5-zone UI.
fn draw_ui(f: &mut ratatui::Frame, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),   // header
            Constraint::Length(5),   // greeter/notifications
            Constraint::Min(10),    // chat area
            Constraint::Length(1),   // status bar
            Constraint::Length(3),   // input
        ])
        .split(f.area());

    // Header
    let header = Paragraph::new(Line::from(vec![
        Span::styled(" Terra ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        Span::styled("Agent OS", Style::default().fg(Color::DarkGray)),
        Span::raw("  │  "),
        Span::styled(&app.model_name, Style::default().fg(Color::Green)),
        Span::raw("  │  "),
        Span::styled(&app.mode, Style::default().fg(Color::Yellow)),
    ]))
    .block(Block::default().borders(Borders::BOTTOM));
    f.render_widget(header, chunks[0]);

    // Greeter / Notification area
    let greeter = Paragraph::new(Text::raw(&app.greeter_text))
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::NONE));
    f.render_widget(greeter, chunks[1]);

    // Chat area
    let mut chat_lines: Vec<Line> = Vec::new();
    for msg in &app.messages {
        let (prefix, color) = match msg.role.as_str() {
            "user" => ("you > ", Color::Green),
            "assistant" => ("terra > ", Color::Cyan),
            "system" => ("", Color::Yellow),
            _ => ("", Color::White),
        };
        chat_lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(color).add_modifier(Modifier::BOLD)),
            Span::raw(&msg.content),
        ]));
        chat_lines.push(Line::from("")); // spacing
    }
    if app.thinking {
        chat_lines.push(Line::from(Span::styled(
            "  ⠹ Thinking...",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let chat = Paragraph::new(chat_lines)
        .wrap(Wrap { trim: false })
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" Chat "));
    f.render_widget(chat, chunks[2]);

    // Status bar
    let status = Paragraph::new(Line::from(vec![
        Span::styled(format!(" {} ", app.mode), Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw("  "),
        Span::styled(format!("{}", app.model_name), Style::default().fg(Color::Green)),
        Span::raw("  │  "),
        Span::styled(format!("{} tokens", app.tokens), Style::default().fg(Color::DarkGray)),
    ]));
    f.render_widget(status, chunks[3]);

    // Input area
    let input = Paragraph::new(Line::from(vec![
        Span::styled(
            if app.mode == "goal" { "goal > " } else { "terra > " },
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ),
        Span::raw(&app.input),
    ]))
    .block(Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green))
        .title(" Input "));
    f.render_widget(input, chunks[4]);

    // Cursor position
    let cursor_x = chunks[4].x + 1 + (if app.mode == "goal" { 7 } else { 8 }) as u16 + app.cursor_pos as u16;
    let cursor_y = chunks[4].y + 1;
    f.set_cursor_position((cursor_x, cursor_y));
}
