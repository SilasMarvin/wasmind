use clap::{Parser, Subcommand};
use hive_cli::init_logger_with_path;

mod graph;
mod splash;
mod utils;

#[derive(Parser)]
#[command(name = "tui-preview")]
#[command(about = "A TUI preview system to test visualizations with mock scenarios")]
struct Args {
    #[command(subcommand)]
    scenario: Scenario,
}

#[derive(Subcommand)]
enum Scenario {
    /// Preview the splash screen
    Splash,
    /// Preview the agent graph
    Graph,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logger_with_path("log.txt");

    let args = Args::parse();

    // Run the preview scenario
    let _ = tokio::task::spawn_blocking(move || {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            match args.scenario {
                Scenario::Splash => splash::run().await,
                Scenario::Graph => graph::run().await,
            }
        })
    })
    .await?;

    Ok(())
}
