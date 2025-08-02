use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "hive_tui")]
#[command(about = "Hive Actor System TUI")]
pub struct Cli {
    /// Path to the configuration file
    #[arg(short, long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Optional prompt to send as initial user message to assistant
    #[arg(short, long)]
    pub prompt: Option<String>,
}
