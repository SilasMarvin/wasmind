use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "copilot")]
#[command(about = "AI-powered assistant with file and plan management")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run the main copilot assistant (default behavior)
    Run,
    /// Run in headless mode with an initial prompt
    Headless {
        /// The initial prompt to send to the LLM
        prompt: String,
        /// Auto-approve non-whitelisted commands (overrides config setting)
        #[arg(long)]
        auto_approve_commands: bool,
    },
    /// Preview how system prompts are rendered with different states
    PromptPreview {
        /// Show all preview scenarios
        #[arg(long)]
        all: bool,
        /// Show scenario with no files or plans
        #[arg(long)]
        empty: bool,
        /// Show scenario with files loaded
        #[arg(long)]
        files: bool,
        /// Show scenario with a plan
        #[arg(long)]
        plan: bool,
        /// Show scenario with both files and plan
        #[arg(long)]
        complete: bool,
        /// Use a custom config file for the preview
        #[arg(long, value_name = "FILE")]
        config: Option<String>,
    },
}

impl Default for Commands {
    fn default() -> Self {
        Commands::Run
    }
}
