use copilot::{init_logger, run_main_program, run_headless_program, SResult};

fn main() -> SResult<()> {
    use clap::Parser;

    init_logger();

    // Parse command line arguments
    let cli = copilot::cli::Cli::parse();

    match cli.command.unwrap_or_default() {
        copilot::cli::Commands::Run => {
            run_main_program()?;
        }
        copilot::cli::Commands::Headless {
            prompt,
            auto_approve_commands,
        } => {
            run_headless_program(prompt, auto_approve_commands)?;
        }
        copilot::cli::Commands::PromptPreview {
            all,
            empty,
            files,
            plan,
            complete,
            config,
        } => {
            if let Err(e) = copilot::prompt_preview::execute_demo(all, empty, files, plan, complete, config)
            {
                eprintln!("Prompt preview error: {}", e);
                std::process::exit(1);
            }
        }
    }

    Ok(())
}
