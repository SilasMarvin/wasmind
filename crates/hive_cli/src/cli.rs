use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "hive_cli")]
#[command(about = "Hive Actor System CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Path to the configuration file
    #[arg(short, long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Optional prompt to send as initial user message to assistant
    #[arg(short, long)]
    pub prompt: Option<String>,

    /// Path to log file (defaults to ~/.local/share/hive/hive.log)
    #[arg(long, value_name = "FILE")]
    pub log_file: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Show configuration and cache information
    Info,
    /// Clean the actor cache
    Clean,
    /// Validate and show configuration details
    Check,
}
