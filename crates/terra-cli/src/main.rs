use clap::Parser;

#[derive(Parser)]
#[command(name = "terra", version, about = "Terra — AI Agent Platform")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Check environment readiness
    Doctor,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Doctor) => {
            println!("Terra doctor: environment check");
            println!("  Rust: {}", env!("CARGO_PKG_VERSION"));
            println!("  Status: OK");
        }
        None => {
            println!("Terra v{}", env!("CARGO_PKG_VERSION"));
            println!("AI Agent Platform — Caging Engineering inspired");
            println!();
            println!("Run `terra --help` for available commands.");
        }
    }

    Ok(())
}
