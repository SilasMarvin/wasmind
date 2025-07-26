use hive::HiveResult;

#[tokio::main]
async fn main() -> HiveResult<()> {
    println!("RUNNING THE HIVE TUI");

    hive::init_test_logger();

    let starting_actors = vec!["execute_bash"];
    let config_actors = vec![hive_config::Actor {
        name: "execute_bash".to_string(),
        source: hive_config::ActorSource::Path(
            "/Users/silasmarvin/github/hive/actors/execute_bash".to_string(),
        ),
    }];

    let loaded_actors = hive::load_actors(config_actors).await?;

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
