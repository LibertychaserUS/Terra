//! CLI UI Renderer
//!
//! Renders ReplOutput to terminal with colors, formatting, and status.
//! Pure ANSI escape codes — zero external dependencies.
//! Fully decoupled from REPL logic — consumes ReplOutput enum.
//!
//! Architecture:
//! - TermRenderer: colored terminal output
//! - PlainRenderer: no colors (for piping)
//! - PetRenderer: optional ASCII pet status (plug-in, not built-in)

#![allow(dead_code)] // Colors and methods used incrementally

use terra_agents::repl::ReplOutput;
use terra_types::event::TokenTracker;

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
                // Simple markdown-like rendering
                for line in text.lines() {
                    if line.starts_with("## ") {
                        println!("{BOLD}{BLUE}{}{RESET}", &line[3..]);
                    } else if line.starts_with("# ") {
                        println!("{BOLD}{MAGENTA}{}{RESET}", &line[2..]);
                    } else if line.starts_with("- ") || line.starts_with("  -") {
                        println!("{CYAN}{}{RESET}", line);
                    } else if line.starts_with("```") {
                        println!("{DIM}{}{RESET}", line);
                    } else if line.starts_with("  /") {
                        // Command in help
                        let parts: Vec<&str> = line.splitn(2, " — ").collect();
                        if parts.len() == 2 {
                            print!("{GREEN}{}{RESET}", parts[0]);
                            println!(" — {DIM}{}{RESET}", parts[1]);
                        } else {
                            println!("{GREEN}{}{RESET}", line);
                        }
                    } else {
                        println!("{}", line);
                    }
                }
            }
            ReplOutput::Notification(msg) => {
                println!("{YELLOW}[Terra]{RESET} {}", msg);
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
        println!("  ╔════════════════════════════════════╗");
        println!("  ║        Terra Agent OS              ║");
        println!("  ║    Efficiency is Intelligence      ║");
        println!("  ╚════════════════════════════════════╝{RESET}");
        println!();
        println!("  {DIM}Version{RESET}   {}", env!("CARGO_PKG_VERSION"));
        println!("  {DIM}Provider{RESET}  {GREEN}{}{RESET}", self.provider_name);
        println!("  {DIM}Mode{RESET}      Conversation");
        println!();
        println!("  Type {GREEN}/help{RESET} for commands, {GREEN}/quit{RESET} to exit.");
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

/// Create the appropriate renderer based on environment.
pub fn auto_renderer(provider_name: &str) -> Box<dyn OutputRenderer> {
    if is_terminal() {
        Box::new(TermRenderer::new(provider_name))
    } else {
        Box::new(PlainRenderer)
    }
}
