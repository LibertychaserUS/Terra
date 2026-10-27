//! Terminal output: colors on an interactive terminal, plain text otherwise.

use terra_agents::Output;

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const CYAN: &str = "\x1b[36m";
pub const MAGENTA: &str = "\x1b[35m";

/// "Thinking..." with elapsed time on one line. Silent when output is piped.
pub struct Spinner {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Spinner {
    pub fn start() -> Self {
        use std::io::Write;
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        if !color_enabled() {
            return Self { stop };
        }
        let flag = stop.clone();
        tokio::spawn(async move {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let start = std::time::Instant::now();
            let mut i = 0;
            while !flag.load(std::sync::atomic::Ordering::Relaxed) {
                print!("\r{DIM}  {} Thinking... ({:.1}s){RESET}  ", frames[i % frames.len()], start.elapsed().as_secs_f32());
                std::io::stdout().flush().ok();
                tokio::time::sleep(std::time::Duration::from_millis(80)).await;
                i += 1;
            }
            print!("\r{}\r", " ".repeat(60));
            std::io::stdout().flush().ok();
        });
        Self { stop }
    }

    pub fn stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Print a text block; fenced code is dimmed and headings bold on a terminal.
pub fn render_text_block(text: &str) {
    if !color_enabled() {
        println!("{}", text);
        return;
    }
    let mut in_code = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            println!("{DIM}{}{RESET}", line);
        } else if in_code {
            println!("{CYAN}{}{RESET}", line);
        } else if let Some(h) = line.strip_prefix("## ") {
            println!("{BOLD}{}{RESET}", h);
        } else if let Some(h) = line.strip_prefix("# ") {
            println!("{BOLD}{MAGENTA}{}{RESET}", h);
        } else {
            println!("{}", line);
        }
    }
}

/// Render one piece of agent output.
pub fn render(output: &Output) {
    match output {
        Output::Text(t) => render_text_block(t),
        Output::Answer(a) => {
            render_text_block(a);
            println!();
        }
        Output::Notice(n) => {
            if n.starts_with("  >>") || n.starts_with("  <<") || n.ends_with("tokens") {
                println!("{}", paint(&format!("{DIM}{}{RESET}", n)));
            } else {
                println!("{}", paint(&format!("{YELLOW}[Terra]{RESET} {}", n)));
            }
        }
        Output::Quit => {}
    }
}

pub fn banner(provider_name: &str) {
    println!("{}", paint(&format!("{BOLD}{MAGENTA}Terra{RESET} {DIM}v{} — a caged coding agent{RESET}", env!("CARGO_PKG_VERSION"))));
    println!("{}", paint(&format!("  {DIM}Provider{RESET}  {GREEN}{}{RESET}", provider_name)));
    println!("{}", paint(&format!("  Type {GREEN}/help{RESET} for commands, {GREEN}/quit{RESET} to exit.")));
    println!();
}

pub fn is_terminal() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdout())
}

/// Colors and spinners only on an interactive terminal, never with `NO_COLOR`.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ansi_removes_sgr_codes_only() {
        assert_eq!(strip_ansi("\x1b[1m\x1b[35mTerra\x1b[0m ok"), "Terra ok");
        assert_eq!(strip_ansi("plain 中文 ✓"), "plain 中文 ✓");
    }
}
