//! CLI UI Renderer
//!
//! Renders ReplOutput to terminal with colors, formatting, and status.
//! Pure ANSI escape codes — zero external dependencies.
//! Fully decoupled from REPL logic — consumes ReplOutput enum.
//!
//! Architecture:
//! - TermRenderer: colored terminal output with code block tracking
//! - PlainRenderer: no colors (for piping)
//! - Spinner: dynamic "Thinking..." with elapsed time
//! - typewriter_print: gradual character-by-character display for LLM responses

#![allow(dead_code)] // Colors and methods used incrementally

use terra_agents::repl::ReplOutput;
use terra_types::event::TokenTracker;

/// Dynamic thinking spinner — shows elapsed time on same line.
pub struct Spinner {
    handle: Option<tokio::task::JoinHandle<()>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Spinner {
    /// Start a thinking spinner with live elapsed timer.
    /// Does nothing when stdout is not a terminal (piped output stays clean).
    pub fn start() -> Self {
        use std::io::Write;
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        if !color_enabled() {
            return Self { handle: None, stop };
        }
        let stop_clone = stop.clone();

        let handle = tokio::spawn(async move {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let start_time = std::time::Instant::now();
            let mut i = 0;
            loop {
                if stop_clone.load(std::sync::atomic::Ordering::Relaxed) { break; }
                let elapsed = start_time.elapsed().as_secs_f32();
                print!("\r\x1b[2m  {} Thinking... ({:.1}s)\x1b[0m  ", frames[i % frames.len()], elapsed);
                std::io::stdout().flush().ok();
                tokio::time::sleep(std::time::Duration::from_millis(80)).await;
                i += 1;
            }
            print!("\r{}\r", " ".repeat(60));
            std::io::stdout().flush().ok();
        });

        Self { handle: Some(handle), stop }
    }

    /// Stop the spinner.
    pub fn stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

// ANSI color codes
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";
const RED: &str = "\x1b[31m";
const MAGENTA: &str = "\x1b[35m";
const BLUE: &str = "\x1b[34m";

/// Format a single markdown line with ANSI codes.
/// Returns the formatted string (does not print).
fn format_markdown_line(line: &str) -> String {
    if let Some(heading) = line.strip_prefix("## ") {
        format!("{BOLD}{BLUE}{}{RESET}", heading)
    } else if let Some(title) = line.strip_prefix("# ") {
        format!("{BOLD}{MAGENTA}{}{RESET}", title)
    } else if line.starts_with("- ") || line.starts_with("  -") {
        format!("{CYAN}{}{RESET}", line)
    } else if line.starts_with("  /") {
        // Command in help
        let parts: Vec<&str> = line.splitn(2, " — ").collect();
        if parts.len() == 2 {
            format!("{GREEN}{}{RESET} — {DIM}{}{RESET}", parts[0], parts[1])
        } else {
            format!("{GREEN}{}{RESET}", line)
        }
    } else {
        line.to_string()
    }
}

/// Render a multi-line text block with code block state tracking.
/// Handles ``` fenced code blocks with box-drawing borders.
pub fn render_text_block(text: &str) {
    if !color_enabled() {
        println!("{}", text);
        return;
    }
    let mut in_code_block = false;

    for line in text.lines() {
        if let Some(fence_rest) = line.strip_prefix("```") {
            if !in_code_block {
                // Opening fence — extract language tag
                let lang = fence_rest.trim();
                if lang.is_empty() {
                    println!("{DIM}  ┌──────────────────────────────────{RESET}");
                } else {
                    println!("{DIM}  ┌── {lang} ──────────────────────────{RESET}");
                }
                in_code_block = true;
            } else {
                // Closing fence
                println!("{DIM}  └──────────────────────────────────{RESET}");
                in_code_block = false;
            }
            continue;
        }

        if in_code_block {
            // Code line: dim border, normal content (no markdown detection)
            println!("{DIM}  │{RESET} {}", line);
        } else {
            println!("{}", format_markdown_line(line));
        }
    }
}

/// Typewriter-style gradual display for LLM responses.
/// Renders markdown formatting while printing character-by-character.
pub async fn typewriter_print(text: &str) {
    use std::io::Write;
    let mut stdout = std::io::stdout();
    let mut in_code_block = false;

    for line in text.lines() {
        // Handle code fence transitions
        if let Some(fence_rest) = line.strip_prefix("```") {
            if !in_code_block {
                let lang = fence_rest.trim();
                let fence = if lang.is_empty() {
                    format!("{DIM}  ┌──────────────────────────────────{RESET}")
                } else {
                    format!("{DIM}  ┌── {lang} ──────────────────────────{RESET}")
                };
                for ch in fence.chars() {
                    print!("{ch}");
                    stdout.flush().ok();
                }
                println!();
                in_code_block = true;
            } else {
                let fence = format!("{DIM}  └──────────────────────────────────{RESET}");
                for ch in fence.chars() {
                    print!("{ch}");
                    stdout.flush().ok();
                }
                println!();
                in_code_block = false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            continue;
        }

        // Format the line
        let formatted = if in_code_block {
            format!("{DIM}  │{RESET} {}", line)
        } else {
            format_markdown_line(line)
        };

        // Print character-by-character
        for ch in formatted.chars() {
            print!("{ch}");
            stdout.flush().ok();
            // Skip delay for ANSI escape sequences (invisible chars)
            if ch == '\x1b' {
                // We're in an escape sequence — don't delay
                continue;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        println!();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

/// Trait for pluggable renderers. CLI uses TermRenderer by default.
/// Pet, TUI, or Web renderers can implement this trait.
pub trait OutputRenderer {
    fn render(&self, output: &ReplOutput);
    fn render_banner(&self);
    fn render_status_bar(&self, mode: &str, tokens: &TokenTracker, turns: usize);
}

/// Colored terminal renderer.
pub struct TermRenderer {
    pub provider_name: String,
}

impl TermRenderer {
    pub fn new(provider_name: &str) -> Self {
        Self { provider_name: provider_name.into() }
    }
}

impl OutputRenderer for TermRenderer {
    fn render(&self, output: &ReplOutput) {
        use std::io::Write;
        let mut stdout = std::io::stdout();

        match output {
            ReplOutput::Text(text) => {
                render_text_block(text);
            }
            ReplOutput::Notification(msg) => {
                if msg.starts_with("  ●") || msg.starts_with("  ✓") || msg.starts_with("  ─") {
                    // Operation chain: dim status lines
                    println!("{DIM}{}{RESET}", msg);
                } else if msg.contains("Thinking") {
                    println!("{DIM}  ● {}{RESET}", msg.trim());
                } else {
                    println!("{YELLOW}[Terra]{RESET} {}", msg);
                }
            }
            ReplOutput::Token(text) => {
                print!("{}", text);
                stdout.flush().ok();
            }
            ReplOutput::Event(_) => {
                // Events consumed by event-driven UIs, not terminal
            }
            ReplOutput::Prompt(prompt) => {
                print!("{GREEN}{BOLD}{}{RESET}", prompt);
                stdout.flush().ok();
            }
        }
    }

    fn render_banner(&self) {
        println!("{BOLD}{MAGENTA}");
        println!(r#"  ████████╗███████╗██████╗ ██████╗  █████╗ "#);
        println!(r#"  ╚══██╔══╝██╔════╝██╔══██╗██╔══██╗██╔══██╗"#);
        println!(r#"     ██║   █████╗  ██████╔╝██████╔╝███████║"#);
        println!(r#"     ██║   ██╔══╝  ██╔══██╗██╔══██╗██╔══██║"#);
        println!(r#"     ██║   ███████╗██║  ██║██║  ██║██║  ██║"#);
        println!(r#"     ╚═╝   ╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝"#);
        println!("{DIM}        Agent OS — Efficiency is Intelligence{RESET}");
        println!();
        println!("  {DIM}Version{RESET}   {}", env!("CARGO_PKG_VERSION"));
        println!("  {DIM}Provider{RESET}  {GREEN}{}{RESET}", self.provider_name);
        println!("  {DIM}Mode{RESET}      Conversation");
        println!();
        println!("  Type {GREEN}/help{RESET} for commands, {GREEN}/quit{RESET} to exit.");
        println!("  {DIM}Ctrl+C to interrupt, Ctrl+D to exit.{RESET}");
        println!();
    }

    fn render_status_bar(&self, mode: &str, tokens: &TokenTracker, turns: usize) {
        let mode_colored = match mode {
            "goal" => format!("{YELLOW}{BOLD}GOAL{RESET}"),
            _ => format!("{GREEN}CONV{RESET}"),
        };
        println!(
            "{DIM}─── {} │ tokens: {} │ turns: {} ───{RESET}",
            mode_colored,
            tokens.total_tokens(),
            turns,
        );
    }
}

/// Plain renderer (no colors) for piping or non-terminal use.
pub struct PlainRenderer;

impl OutputRenderer for PlainRenderer {
    fn render(&self, output: &ReplOutput) {
        use std::io::Write;
        match output {
            ReplOutput::Text(t) => println!("{}", t),
            ReplOutput::Notification(n) => println!("[Terra] {}", n),
            ReplOutput::Token(t) => {
                print!("{}", t);
                std::io::stdout().flush().ok();
            }
            ReplOutput::Event(_) => {}
            ReplOutput::Prompt(p) => {
                print!("{}", p);
                std::io::stdout().flush().ok();
            }
        }
    }

    fn render_banner(&self) {
        println!("Terra Agent OS v{}", env!("CARGO_PKG_VERSION"));
        println!("Type /help for commands, /quit to exit.");
        println!();
    }

    fn render_status_bar(&self, mode: &str, tokens: &TokenTracker, turns: usize) {
        println!("--- {} | tokens: {} | turns: {} ---", mode, tokens.total_tokens(), turns);
    }
}

/// Check if stdout is a terminal (for auto-selecting renderer).
pub fn is_terminal() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdout())
}

/// Colors, spinners and typing effects only on an interactive terminal,
/// and never when `NO_COLOR` is set (https://no-color.org).
pub fn color_enabled() -> bool {
    is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

/// Remove ANSI SGR escape sequences (`ESC [ ... m`).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Text for stdout: unchanged on a color terminal, escape codes removed otherwise.
pub fn paint(s: &str) -> std::borrow::Cow<'_, str> {
    if color_enabled() { s.into() } else { strip_ansi(s).into() }
}

/// Create the appropriate renderer based on environment.
pub fn auto_renderer(provider_name: &str) -> Box<dyn OutputRenderer> {
    if color_enabled() {
        Box::new(TermRenderer::new(provider_name))
    } else {
        Box::new(PlainRenderer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ansi_removes_sgr_codes_only() {
        assert_eq!(strip_ansi("\x1b[1m\x1b[35mTerra\x1b[0m ok"), "Terra ok");
        assert_eq!(strip_ansi("plain 中文 ✓"), "plain 中文 ✓");
        assert_eq!(strip_ansi("\x1b[2m  ● Running\x1b[0m"), "  ● Running");
    }
}
