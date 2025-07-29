use clap::Parser;
use hive::HiveResult;

mod cli;

#[tokio::main]
async fn main() -> HiveResult<()> {
    hive::init_test_logger();

    let cli = cli::Cli::parse();

    // Load configuration
    let config = if let Some(config_path) = cli.config {
        hive_config::load_from_path(config_path)?
    } else {
        hive_config::load_default_config()?
    };

    let starting_actors = vec!["execute_bash", "assistant"];
    let loaded_actors = hive::load_actors(config.actors).await?;

    hive::hive::start_hive(&starting_actors, loaded_actors).await?;

    Ok(())
}

// use hive::{init_test_logger, run_headless_program, run_main_program, SResult};
//
// #[tokio::main]
// async fn main() -> SResult<()> {
//     use clap::Parser;
//
//     init_test_logger();
//
//     // Parse command line arguments
//     let cli = hive::cli::Cli::parse();
//
//     match cli.command {
//         None => {
//             // No subcommand provided, use top-level prompt if any
//             run_main_program(cli.prompt).await?;
//         }
//         Some(hive::cli::Commands::Headless {
//             prompt,
//             auto_approve_commands,
//         }) => {
//             run_headless_program(prompt, auto_approve_commands).await?;
//         }
//         Some(hive::cli::Commands::PromptPreview {
//             all,
//             empty,
//             files,
//             plan,
//             agents,
//             complete,
//             full,
//             agent_types,
//             config,
//         }) => {
//             if let Err(e) = hive::prompt_preview::execute_demo(
//                 all,
//                 empty,
//                 files,
//                 plan,
//                 agents,
//                 complete,
//                 full,
//                 agent_types,
//                 config,
//             ) {
//                 eprintln!("Prompt preview error: {}", e);
//                 std::process::exit(1);
//             }
//         }
//     }
//
//     Ok(())
// }
