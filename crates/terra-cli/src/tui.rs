//! Terra TUI — ratatui-based terminal user interface
//!
//! Architecture:
//! - Single task: REPL lives inline (no Send requirement)
//! - Key events polled at 50ms; pending input processed each loop iteration
//! - UI blocks briefly during LLM calls (acceptable for CLI TUI)
//!
//! Horizontal layout, spacious. Chat area dominates.
//! Activated only when stdout is a terminal.

#![allow(dead_code, unused_imports)]

use std::io;
use std::sync::Arc;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Terminal;

use terra_agents::repl::{Repl, ReplOutput};
use terra_core::IntelligenceCore;
use terra_executor::CommandExecutor;

/// TUI application state.
struct TuiState {
    input: String,
    /// Cursor position in characters (not bytes) within `input`.
    cursor_pos: usize,
    messages: Vec<(String, String, Style)>,
    model_name: String,
    mode: String,
    tokens: u64,
    thinking: bool,
    greeter_lines: Vec<String>,
    input_history: Vec<String>,
    history_pos: Option<usize>,
}

impl TuiState {
    fn new(model_name: &str, greeter_lines: Vec<String>) -> Self {
        Self {
            input: String::new(),
            cursor_pos: 0,
            messages: Vec::new(),
            model_name: model_name.to_string(),
            mode: "conversation".into(),
            tokens: 0,
            thinking: false,
            greeter_lines,
            input_history: Vec::new(),
            history_pos: None,
        }
    }

    /// Byte offset of the character at `char_idx` (or the end of the input).
    fn byte_offset(&self, char_idx: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_idx)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len())
    }

    fn insert_char(&mut self, c: char) {
        let at = self.byte_offset(self.cursor_pos);
        self.input.insert(at, c);
        self.cursor_pos += 1;
    }

    fn backspace(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
            let at = self.byte_offset(self.cursor_pos);
            self.input.remove(at);
        }
    }

    fn move_left(&mut self) {
        self.cursor_pos = self.cursor_pos.saturating_sub(1);
    }

    fn move_right(&mut self) {
        if self.cursor_pos < self.input.chars().count() {
            self.cursor_pos += 1;
        }
    }

    fn set_input(&mut self, text: String) {
        self.cursor_pos = text.chars().count();
        self.input = text;
    }

    /// Terminal columns between the start of the input and the cursor
    /// (wide characters such as CJK take two columns).
    fn cursor_columns(&self) -> u16 {
        Span::raw(&self.input[..self.byte_offset(self.cursor_pos)]).width() as u16
    }

    fn add_msg(&mut self, prefix: &str, content: &str, style: Style) {
        self.messages.push((prefix.into(), content.into(), style));
    }

    fn process_outputs(&mut self, outputs: Vec<ReplOutput>) {
        self.thinking = false;
        for output in outputs {
            match output {
                ReplOutput::Text(t) => self.add_msg("", &t, Style::default()),
                ReplOutput::Notification(n) => {
                    let style = if n.starts_with("  ●") || n.starts_with("  ✓") || n.starts_with("  ─") {
                        Style::default().fg(Color::DarkGray)
                    } else {
                        Style::default().fg(Color::Yellow)
                    };
                    let prefix = if style.fg == Some(Color::Yellow) { "[Terra] " } else { "" };
                    self.add_msg(prefix, &n, style);
                }
                ReplOutput::Prompt(_) => {}
                ReplOutput::Token(t) => {
                    if let Some(last) = self.messages.last_mut() { last.1.push_str(&t); }
                }
                ReplOutput::Event(_) => {}
            }
        }
    }
}

/// Run full TUI with inline REPL processing.
///
/// The REPL lives in the same task — no `tokio::spawn`, no `Send` requirement.
/// Input is processed inline via `pending_input`. The UI blocks briefly during
/// LLM calls, which is acceptable for a CLI TUI.
pub async fn run_tui_async(
    executor: Arc<CommandExecutor>,
    core: Arc<IntelligenceCore>,
    model_name: &str,
    greeter_lines: Vec<String>,
) -> io::Result<()> {
    // REPL lives in the SAME task — no Send requirement.
    // No confirm callback here: dangerous tools (shell, delete) are declined
    // in the TUI until it has its own confirmation prompt.
    let mut repl = Repl::new(executor, core);

    // Restore the terminal before a panic message is printed, otherwise the
    // message is lost in the alternate screen and the shell stays in raw mode.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    // Terminal setup
    enable_raw_mode()?;
    let _guard = TerminalGuard;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = TuiState::new(model_name, greeter_lines);
    let mut running = true;
    let mut pending_input: Option<String> = None;

    while running {
        terminal.draw(|f| draw_ui(f, &state))?;

        // Process pending input inline (no spawn needed)
        if let Some(input) = pending_input.take() {
            let outputs = repl.process_input(&input).await;
            let quit = Repl::should_quit(&outputs);
            state.process_outputs(outputs);
            state.mode = if repl.session.mode().is_goal() { "goal" } else { "conversation" }.into();
            state.tokens = repl.tokens().total_tokens() as u64;
            if quit {
                running = false;
                continue;
            }
        }

        // Key events (50ms poll for responsive UI)
        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if state.thinking {
                            state.thinking = false;
                            state.add_msg("", "[interrupted]", Style::default().fg(Color::Yellow));
                        }
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        running = false;
                    }
                    KeyCode::Enter => {
                        if !state.input.is_empty() {
                            let input = std::mem::take(&mut state.input);
                            state.input_history.push(input.clone());
                            state.history_pos = None;
                            state.cursor_pos = 0;
                            state.add_msg("you ❯ ", &input, Style::default().fg(Color::Green));
                            state.thinking = true;
                            pending_input = Some(input);
                        }
                    }
                    KeyCode::Char(c) => state.insert_char(c),
                    KeyCode::Backspace => state.backspace(),
                    KeyCode::Left => state.move_left(),
                    KeyCode::Right => state.move_right(),
                    KeyCode::Up => {
                        if !state.input_history.is_empty() {
                            let pos = state.history_pos.map(|p| p.saturating_sub(1)).unwrap_or(state.input_history.len() - 1);
                            state.history_pos = Some(pos);
                            state.set_input(state.input_history[pos].clone());
                        }
                    }
                    KeyCode::Down => {
                        if let Some(pos) = state.history_pos {
                            if pos + 1 < state.input_history.len() {
                                state.history_pos = Some(pos + 1);
                                state.set_input(state.input_history[pos + 1].clone());
                            } else {
                                state.history_pos = None;
                                state.set_input(String::new());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    terminal.show_cursor()?;
    Ok(())
}

/// Leaves raw mode and the alternate screen when the TUI exits for any reason.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
}

/// Draw UI — 3 zones: status bar (top), chat (middle), input (bottom).
/// Horizontal, spacious, chat dominates.
fn draw_ui(f: &mut ratatui::Frame, state: &TuiState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),   // status bar
            Constraint::Min(5),     // chat
            Constraint::Length(3),   // input
        ])
        .split(f.area());

    // Status bar
    let status = Line::from(vec![
        Span::styled(" ◆ Terra ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        Span::styled("│ ", Style::default().fg(Color::DarkGray)),
        Span::styled(&state.model_name, Style::default().fg(Color::Green)),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(&state.mode, Style::default().fg(Color::Yellow)),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{} tok", state.tokens), Style::default().fg(Color::DarkGray)),
        Span::styled(" │ Ctrl+C interrupt │ Ctrl+D exit", Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(status), chunks[0]);

    // Chat area
    let mut lines: Vec<Line> = Vec::new();

    if state.messages.is_empty() {
        for gl in &state.greeter_lines {
            lines.push(Line::from(Span::styled(gl.as_str(), Style::default().fg(Color::DarkGray))));
        }
        lines.push(Line::from(""));
    }

    for (prefix, content, style) in &state.messages {
        if !prefix.is_empty() {
            lines.push(Line::from(vec![
                Span::styled(prefix.as_str(), style.add_modifier(Modifier::BOLD)),
                Span::raw(content.as_str()),
            ]));
        } else {
            lines.push(Line::from(Span::styled(content.as_str(), *style)));
        }
    }

    if state.thinking {
        lines.push(Line::from(Span::styled("  ⠹ Thinking...", Style::default().fg(Color::DarkGray))));
    }

    let chat_h = chunks[1].height.saturating_sub(2) as usize;
    let scroll = if lines.len() > chat_h { (lines.len() - chat_h) as u16 } else { 0 };

    let chat = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0))
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));
    f.render_widget(chat, chunks[1]);

    // Input
    let prompt = if state.mode == "goal" { "goal ❯ " } else { "terra ❯ " };
    let input = Paragraph::new(Line::from(vec![
        Span::styled(prompt, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw(&state.input),
    ]))
    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Green)));
    f.render_widget(input, chunks[2]);

    let prompt_width = Span::raw(prompt).width() as u16;
    f.set_cursor_position((chunks[2].x + 1 + prompt_width + state.cursor_columns(), chunks[2].y + 1));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(keys: &str) -> TuiState {
        let mut state = TuiState::new("test", vec![]);
        for c in keys.chars() {
            state.insert_char(c);
        }
        state
    }

    // Regression: typing a multi-byte char followed by another char panicked.
    #[test]
    fn input_accepts_non_ascii() {
        let state = typed("éa中文🦀");
        assert_eq!(state.input, "éa中文🦀");
        assert_eq!(state.cursor_pos, 5);
    }

    #[test]
    fn edit_in_middle_of_multibyte_text() {
        let mut state = typed("中文");
        state.move_left();
        state.insert_char('x');
        assert_eq!(state.input, "中x文");
        state.backspace();
        state.backspace();
        assert_eq!(state.input, "文");
        state.move_right();
        state.move_right();
        assert_eq!(state.cursor_pos, 1);
    }

    #[test]
    fn cursor_columns_count_wide_chars() {
        let state = typed("a中");
        assert_eq!(state.cursor_columns(), 3);
    }

    #[test]
    fn history_recall_puts_cursor_at_end() {
        let mut state = TuiState::new("test", vec![]);
        state.set_input("你好".into());
        assert_eq!(state.cursor_pos, 2);
        state.insert_char('!');
        assert_eq!(state.input, "你好!");
    }
}
