use std::path::PathBuf;
use std::time::SystemTime;

use crate::{
    actors::tools::planner::{Task, TaskPlan, TaskStatus},
    config::{Config, ParsedConfig},
    system_state::SystemState,
    template::{TemplateContext, ToolInfo, is_template, render_template},
};

// TODO: Our prompt preveiw function should take in the prompt to preview for: either -
// manager, worker

/// Preview scenarios for system prompt rendering
pub struct PromptPreviewScenarios {
    config: ParsedConfig,
}

impl PromptPreviewScenarios {
    pub fn new(config_path: Option<String>) -> Result<Self, Box<dyn std::error::Error>> {
        let config = if let Some(path) = config_path {
            Config::from_file(&path)?
        } else {
            Config::new()?
        };

        let parsed_config = config.try_into()?;

        Ok(Self {
            config: parsed_config,
        })
    }

    /// Create mock tools for the demo
    fn create_mock_tools() -> Vec<ToolInfo> {
        vec![
            ToolInfo {
                name: "execute_command".to_string(),
                description: "Execute a shell command with specified arguments".to_string(),
            },
            ToolInfo {
                name: "read_file".to_string(),
                description: "Read file contents".to_string(),
            },
            ToolInfo {
                name: "edit_file".to_string(),
                description: "Edit file contents with various operations".to_string(),
            },
            ToolInfo {
                name: "planner".to_string(),
                description: "Creates and manages a task plan with numbered steps".to_string(),
            },
            ToolInfo {
                name: "github_list_repos".to_string(),
                description: "List GitHub repositories".to_string(),
            },
            ToolInfo {
                name: "playwright_navigate".to_string(),
                description: "Navigate to a URL in the browser".to_string(),
            },
        ]
    }

    /// Create a demo system state with no files or plans
    fn create_empty_state(&self) -> SystemState {
        SystemState::new()
    }

    /// Create a demo system state with files
    fn create_state_with_files(&self) -> SystemState {
        let mut state = SystemState::new();

        // Add some example files
        state.update_file(
            PathBuf::from("src/main.rs"),
            r#"use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    
    if args.len() > 1 {
        println!("Hello, {}!", args[1]);
    } else {
        println!("Hello, World!");
    }
}
"#
            .to_string(),
            SystemTime::now(),
        );

        state.update_file(
            PathBuf::from("README.md"),
            r#"# My Project

This is a simple Rust project that greets the user.

## Usage

```bash
cargo run -- YourName
```

## Features

- Command line argument parsing
- Friendly greetings
- Cross-platform compatibility
"#
            .to_string(),
            SystemTime::now(),
        );

        state.update_file(
            PathBuf::from("config.toml"),
            r#"[database]
host = "localhost"
port = 5432
name = "myapp"

[logging]
level = "info"
file = "app.log"
"#
            .to_string(),
            SystemTime::now(),
        );

        state
    }

    /// Create a demo system state with a plan
    fn create_state_with_plan(&self) -> SystemState {
        let mut state = SystemState::new();

        let plan = TaskPlan {
            title: "Implement User Authentication".to_string(),
            tasks: vec![
                Task {
                    description: "Set up database schema for users".to_string(),
                    status: TaskStatus::Completed,
                },
                Task {
                    description: "Create user registration endpoint".to_string(),
                    status: TaskStatus::Completed,
                },
                Task {
                    description: "Implement password hashing".to_string(),
                    status: TaskStatus::InProgress,
                },
                Task {
                    description: "Add login/logout functionality".to_string(),
                    status: TaskStatus::Pending,
                },
                Task {
                    description: "Create session management".to_string(),
                    status: TaskStatus::Pending,
                },
                Task {
                    description: "Add password reset feature".to_string(),
                    status: TaskStatus::Pending,
                },
                Task {
                    description: "Write unit tests".to_string(),
                    status: TaskStatus::Pending,
                },
            ],
        };

        state.update_plan(plan);
        state
    }

    /// Create a demo system state with both files and a plan
    fn create_complete_state(&self) -> SystemState {
        let mut state = self.create_state_with_files();

        let plan = TaskPlan {
            title: "Refactor Project Structure".to_string(),
            tasks: vec![
                Task {
                    description: "Review current codebase structure".to_string(),
                    status: TaskStatus::Completed,
                },
                Task {
                    description: "Create modules for better organization".to_string(),
                    status: TaskStatus::InProgress,
                },
                Task {
                    description: "Update import statements".to_string(),
                    status: TaskStatus::Pending,
                },
                Task {
                    description: "Update documentation".to_string(),
                    status: TaskStatus::Pending,
                },
            ],
        };

        state.update_plan(plan);
        state
    }

    /// Render system prompt for a given state
    fn render_prompt(&self, state: &SystemState) -> Result<String, Box<dyn std::error::Error>> {
        let tools = Self::create_mock_tools();

        // TODO: UPDATE THIS
        // if is_template(&self.config.model.system_prompt) {
        //     let context =
        //         TemplateContext::new(tools, self.config.whitelisted_commands.clone(), state);
        //
        //     Ok(render_template(&self.config.model.system_prompt, &context)?)
        // } else {
        //     Ok(self.config.model.system_prompt.clone())
        // }

        todo!()
    }

    /// Print a formatted scenario
    fn print_scenario(
        &self,
        title: &str,
        description: &str,
        state: &SystemState,
    ) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n{}", "=".repeat(80));
        println!("SCENARIO: {}", title);
        println!("{}", "=".repeat(80));
        println!("{}", description);
        println!();

        // Show state summary
        println!("STATE SUMMARY:");
        println!("  Files loaded: {}", state.file_count());
        println!("  Plan exists: {}", state.get_plan().is_some());
        if let Some(plan) = state.get_plan() {
            println!("  Plan: {} ({} tasks)", plan.title, plan.tasks.len());
        }
        println!();

        // Show rendered prompt with clear delimiters
        println!("RENDERED SYSTEM PROMPT:");
        println!("{}", "▼".repeat(80));

        let rendered = self.render_prompt(state)?;
        println!("{}", rendered);

        println!("{}", "▲".repeat(80));
        println!("Token estimate: ~{} tokens", estimate_tokens(&rendered));

        Ok(())
    }

    /// Show empty scenario
    pub fn show_empty(&self) -> Result<(), Box<dyn std::error::Error>> {
        let state = self.create_empty_state();
        self.print_scenario(
            "Empty State",
            "No files loaded, no active plan. This is the baseline system prompt.",
            &state,
        )
    }

    /// Show files scenario
    pub fn show_files(&self) -> Result<(), Box<dyn std::error::Error>> {
        let state = self.create_state_with_files();
        self.print_scenario(
            "Files Loaded",
            "Multiple files have been read and are available in the system context.",
            &state,
        )
    }

    /// Show plan scenario
    pub fn show_plan(&self) -> Result<(), Box<dyn std::error::Error>> {
        let state = self.create_state_with_plan();
        self.print_scenario(
            "Plan Active",
            "A task plan is being tracked with various task statuses.",
            &state,
        )
    }

    /// Show complete scenario
    pub fn show_complete(&self) -> Result<(), Box<dyn std::error::Error>> {
        let state = self.create_complete_state();
        self.print_scenario(
            "Complete State",
            "Both files and a plan are active, showing the full context.",
            &state,
        )
    }

    /// Show all scenarios
    pub fn show_all(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("SYSTEM PROMPT PREVIEW");
        println!(
            "This preview shows how the system prompt changes based on loaded files and active plans."
        );

        self.show_empty()?;
        self.show_files()?;
        self.show_plan()?;
        self.show_complete()?;

        println!("\n{}", "=".repeat(80));
        println!("KEY INSIGHTS:");
        println!("• File contents appear in the system prompt, not in tool responses");
        println!("• Plans are tracked in the system context for consistency");
        println!("• Token usage scales with loaded content");
        println!("• Template variables allow dynamic prompt construction");
        println!("{}", "=".repeat(80));

        Ok(())
    }
}

/// Rough token estimation (1 token ≈ 4 characters for English text)
fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

/// Execute the prompt preview subcommand
pub fn execute_demo(
    all: bool,
    empty: bool,
    files: bool,
    plan: bool,
    complete: bool,
    config_path: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let preview = PromptPreviewScenarios::new(config_path)?;

    // If no specific scenario is requested, show all
    if !all && !empty && !files && !plan && !complete {
        return preview.show_all();
    }

    if all {
        preview.show_all()?;
    }

    if empty {
        preview.show_empty()?;
    }

    if files {
        preview.show_files()?;
    }

    if plan {
        preview.show_plan()?;
    }

    if complete {
        preview.show_complete()?;
    }

    Ok(())
}
