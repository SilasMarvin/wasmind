use clap::{Parser, Subcommand};

mod command;
mod edit_file;
mod graph;
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
    /// Preview command execution flow with user approval
    Command,
    /// Preview file editing workflow
    EditFile,
    /// Preview the agent graph
    Graph,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logger to write to log.txt in current directory
    hive::init_test_logger();

    let args = Args::parse();

    // Run the preview scenario
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            match args.scenario {
                Scenario::Command => command::run().await,
                Scenario::Graph => graph::run().await,
                Scenario::EditFile => edit_file::run().await,
            }
        })
    })
    .await?;

    Ok(())
}
