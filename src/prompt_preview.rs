use snafu::{ResultExt, Snafu};
use std::fs;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

use crate::{
    actors::{
        AgentType,
        tools::{
            file_reader::FileReader,
            planner::{Task, TaskPlan, TaskStatus},
        },
    },
    config::{Config, ParsedConfig},
    system_state::{SystemState, SystemStateError},
    template::{ToolInfo, is_template},
};

/// Errors that can occur during prompt preview
#[derive(Debug, Snafu)]
pub enum PromptPreviewError {
    #[snafu(display("Failed to load config: {}", source))]
    ConfigLoadFailed {
        #[snafu(source)]
        source: crate::config::ConfigError,
    },

    #[snafu(display("Failed to render system prompt"))]
    RenderFailed {
        #[snafu(source)]
        source: SystemStateError,
    },
}

pub type Result<T> = std::result::Result<T, PromptPreviewError>;

// TODO: Our prompt preveiw function should take in the prompt to preview for: either -
// manager, worker and whether it is headless or not

/// Preview scenarios for system prompt rendering
pub struct PromptPreviewScenarios {
    config: ParsedConfig,
}

impl PromptPreviewScenarios {
    pub fn new(config_path: Option<String>) -> Result<Self> {
        let config = if let Some(path) = config_path {
            Config::from_file(&path, false).context(ConfigLoadFailedSnafu)?
        } else {
            Config::new(false).context(ConfigLoadFailedSnafu)?
        };

        let parsed_config = config.try_into().context(ConfigLoadFailedSnafu)?;

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
        // Create temporary directory and files for preview
        let temp_dir = TempDir::new().expect("Failed to create temp dir for preview");
        let file_reader = Arc::new(Mutex::new(FileReader::default()));

        // Create example files
        let main_rs_content = r#"use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    
    if args.len() > 1 {
        println!("Hello, {}!", args[1]);
    } else {
        println!("Hello, World!");
    }
}
"#;

        let readme_content = r#"# My Project

This is a simple Rust project that greets the user.

## Usage

```bash
cargo run -- YourName
```

## Features

- Command line argument parsing
- Friendly greetings
- Cross-platform compatibility
"#;

        let config_content = r#"[database]
host = "localhost"
port = 5432
name = "myapp"

[logging]
level = "info"
file = "app.log"
"#;

        // Write files to temp directory
        let main_path = temp_dir.path().join("src").join("main.rs");
        fs::create_dir_all(main_path.parent().unwrap()).expect("Failed to create src dir");
        fs::write(&main_path, main_rs_content).expect("Failed to write main.rs");

        let readme_path = temp_dir.path().join("README.md");
        fs::write(&readme_path, readme_content).expect("Failed to write README.md");

        let config_path = temp_dir.path().join("config.toml");
        fs::write(&config_path, config_content).expect("Failed to write config.toml");

        // Create a large file to demonstrate partial reading
        let large_file_content = r#"// Large data file with many lines
line 1: data entry
line 2: more data
line 3: even more data
line 4: lots of data here
line 5: continuing with data
line 6: data keeps going
line 7: more and more data
line 8: still more data
line 9: data never ends
line 10: final line of data
"#;

        let large_file_path = temp_dir.path().join("large_data.txt");
        fs::write(&large_file_path, large_file_content).expect("Failed to write large file");

        // Create another file to demonstrate reading from the middle
        let log_file_content = r#"[2024-01-01 00:00:01] INFO: Application started
[2024-01-01 00:00:02] DEBUG: Loading configuration
[2024-01-01 00:00:03] INFO: Configuration loaded successfully
[2024-01-01 00:00:04] DEBUG: Connecting to database
[2024-01-01 00:00:05] INFO: Database connection established
[2024-01-01 00:00:06] DEBUG: Starting background tasks
[2024-01-01 00:00:07] INFO: Background tasks started
[2024-01-01 00:00:08] WARN: High memory usage detected
[2024-01-01 00:00:09] DEBUG: Running garbage collection
[2024-01-01 00:00:10] INFO: Garbage collection completed
[2024-01-01 00:00:11] ERROR: Connection timeout occurred
[2024-01-01 00:00:12] WARN: Retrying connection
[2024-01-01 00:00:13] INFO: Connection restored
[2024-01-01 00:00:14] DEBUG: Processing requests
[2024-01-01 00:00:15] INFO: All systems operational
"#;

        let log_file_path = temp_dir.path().join("app.log");
        fs::write(&log_file_path, log_file_content).expect("Failed to write log file");

        // Cache files in FileReader
        {
            let mut reader = file_reader.lock().unwrap();
            reader.read_and_cache_file(&main_path, None, None).ok();
            reader.read_and_cache_file(&readme_path, None, None).ok();
            reader.read_and_cache_file(&config_path, None, None).ok();
            
            // Read only the first 3 lines of the large file to demonstrate partial reading from start
            reader.read_and_cache_file(&large_file_path, Some(1), Some(3)).ok();
            
            // Read lines 8-12 of the log file to demonstrate reading from middle
            reader.read_and_cache_file(&log_file_path, Some(8), Some(12)).ok();
        }

        // Create SystemState with FileReader
        let mut state = SystemState::with_file_reader(file_reader);

        // Keep temp_dir alive by storing it (this is a bit of a hack for preview)
        std::mem::forget(temp_dir);

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

    /// Create a demo system state with spawned agents
    fn create_state_with_agents(&self) -> SystemState {
        let mut state = SystemState::new();

        // Add some agents in different states
        use crate::actors::AgentStatus;
        use crate::scope::Scope;
        use crate::system_state::AgentTaskInfo;

        let agent1 = AgentTaskInfo::new(
            Scope::new(),
            AgentType::Worker,
            "Software Engineer".to_string(),
            "Implement user authentication system".to_string(),
        );
        let _agent1_id = agent1.agent_id;
        state.add_agent(agent1);

        let agent2 = AgentTaskInfo::new(
            Scope::new(),
            AgentType::Worker,
            "Database Architect".to_string(),
            "Design and optimize database schema".to_string(),
        );
        let agent2_id = agent2.agent_id;
        state.add_agent(agent2);
        state.update_agent_status(
            &agent2_id,
            AgentStatus::Done(Ok(crate::actors::AgentTaskResultOk {
                summary: "Database schema completed successfully".to_string(),
                success: true,
            })),
        );

        let agent3 = AgentTaskInfo::new(
            Scope::new(),
            AgentType::Worker,
            "Frontend Developer".to_string(),
            "Create responsive user interface".to_string(),
        );
        state.add_agent(agent3);

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

    /// Create a demo system state with everything: files, plan, and agents
    fn create_full_state(&self) -> SystemState {
        let mut state = self.create_state_with_files();

        // Add plan
        let plan = TaskPlan {
            title: "Full Stack Application Development".to_string(),
            tasks: vec![
                Task {
                    description: "Set up project structure".to_string(),
                    status: TaskStatus::Completed,
                },
                Task {
                    description: "Implement backend API".to_string(),
                    status: TaskStatus::InProgress,
                },
                Task {
                    description: "Build frontend interface".to_string(),
                    status: TaskStatus::InProgress,
                },
                Task {
                    description: "Setup database".to_string(),
                    status: TaskStatus::Completed,
                },
                Task {
                    description: "Write tests".to_string(),
                    status: TaskStatus::Pending,
                },
                Task {
                    description: "Deploy to production".to_string(),
                    status: TaskStatus::Pending,
                },
            ],
        };
        state.update_plan(plan);

        // Add agents
        use crate::actors::AgentStatus;
        use crate::scope::Scope;
        use crate::system_state::AgentTaskInfo;

        let backend_agent = AgentTaskInfo::new(
            Scope::new(),
            AgentType::Worker,
            "Backend Developer".to_string(),
            "Implement REST API endpoints with authentication".to_string(),
        );
        state.add_agent(backend_agent);

        let frontend_agent = AgentTaskInfo::new(
            Scope::new(),
            AgentType::Worker,
            "Frontend Developer".to_string(),
            "Build React components for user dashboard".to_string(),
        );
        state.add_agent(frontend_agent);

        let db_agent = AgentTaskInfo::new(
            Scope::new(),
            AgentType::Worker,
            "Database Administrator".to_string(),
            "Optimize database queries and setup indexing".to_string(),
        );
        let db_agent_id = db_agent.agent_id;
        state.add_agent(db_agent);
        state.update_agent_status(
            &db_agent_id,
            AgentStatus::Done(Ok(crate::actors::AgentTaskResultOk {
                summary: "Database optimization completed with 40% performance improvement"
                    .to_string(),
                success: true,
            })),
        );

        state
    }

    /// Render system prompt for a given state
    fn render_prompt(&self, state: &SystemState, agent_type: Option<&str>) -> Result<String> {
        let tools = Self::create_mock_tools();

        // Get the appropriate system prompt based on agent type
        let system_prompt = match agent_type {
            Some("main_manager") => &self.config.hive.main_manager_model.system_prompt,
            Some("sub_manager") => &self.config.hive.sub_manager_model.system_prompt,
            Some("worker") => &self.config.hive.worker_model.system_prompt,
            _ => &self.config.hive.main_manager_model.system_prompt, // Default to main manager
        };

        if is_template(system_prompt) {
            let rendered = state
                .render_system_prompt(
                    system_prompt,
                    &tools,
                    self.config.whitelisted_commands.clone(),
                    crate::scope::Scope::new(), // Preview scope - dummy ID
                )
                .context(RenderFailedSnafu)?;
            Ok(rendered)
        } else {
            Ok(system_prompt.clone())
        }
    }

    /// Print a formatted scenario
    fn print_scenario(
        &self,
        title: &str,
        description: &str,
        state: &SystemState,
        agent_type: Option<&str>,
    ) -> Result<()> {
        println!("\n{}", "=".repeat(80));
        println!("SCENARIO: {}", title);
        if let Some(agent) = agent_type {
            println!("AGENT TYPE: {}", agent);
        }
        println!("{}", "=".repeat(80));
        println!("{}", description);
        println!();

        // Show state summary
        println!("STATE SUMMARY:");
        println!("  Files loaded: {}", state.file_count());
        println!("  Plan exists: {}", state.get_plan().is_some());
        if let Some(plan) = state.get_plan() {
            let completed = plan
                .tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Completed)
                .count();
            let in_progress = plan
                .tasks
                .iter()
                .filter(|t| t.status == TaskStatus::InProgress)
                .count();
            let pending = plan
                .tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Pending)
                .count();
            println!(
                "  Plan: {} ({} total: {} completed, {} in progress, {} pending)",
                plan.title,
                plan.tasks.len(),
                completed,
                in_progress,
                pending
            );
        }
        println!("  Agents spawned: {}\n", state.agent_count());

        // Show rendered prompt with clear delimiters
        let agent_label = agent_type.unwrap_or("main_manager");
        println!("RENDERED SYSTEM PROMPT ({}):", agent_label.to_uppercase());
        println!("{}", "▼".repeat(80));

        let rendered = self.render_prompt(state, agent_type)?;
        println!("{}", rendered);

        println!("{}", "▲".repeat(80));
        println!("Token estimate: ~{} tokens", estimate_tokens(&rendered));

        Ok(())
    }

    /// Show empty scenario
    pub fn show_empty(&self) -> Result<()> {
        let state = self.create_empty_state();
        self.print_scenario(
            "Empty State",
            "No files loaded, no active plan, no agents. This is the baseline system prompt.",
            &state,
            None,
        )
    }

    /// Show files scenario
    pub fn show_files(&self) -> Result<()> {
        let state = self.create_state_with_files();
        self.print_scenario(
            "Files Loaded",
            "Multiple files have been read and are available in the system context using XML tags.",
            &state,
            None,
        )
    }

    /// Show plan scenario
    pub fn show_plan(&self) -> Result<()> {
        let state = self.create_state_with_plan();
        self.print_scenario(
            "Plan Active",
            "A task plan is being tracked with various task statuses, displayed in XML format.",
            &state,
            None,
        )
    }

    /// Show agents scenario
    pub fn show_agents(&self) -> Result<()> {
        let state = self.create_state_with_agents();
        self.print_scenario(
            "Agents Spawned",
            "Multiple agents are working on different tasks, shown in XML format with status tracking.",
            &state,
            None,
        )
    }

    /// Show complete scenario
    pub fn show_complete(&self) -> Result<()> {
        let state = self.create_complete_state();
        self.print_scenario(
            "Files and Plan",
            "Both files and a plan are active, showing combined context in XML format.",
            &state,
            None,
        )
    }

    /// Show full scenario with everything
    pub fn show_full(&self) -> Result<()> {
        let state = self.create_full_state();
        self.print_scenario(
            "Full State",
            "Complete scenario with files, plan, and agents - maximum context size.",
            &state,
            None,
        )
    }

    /// Show different agent types with the same state
    pub fn show_agent_types(&self) -> Result<()> {
        let state = self.create_full_state();

        self.print_scenario(
            "Main Manager View",
            "How the Main Manager sees the system state (delegating tasks).",
            &state,
            Some("main_manager"),
        )?;

        self.print_scenario(
            "Sub-Manager View",
            "How a Sub-Manager sees the system state (receiving delegated tasks).",
            &state,
            Some("sub_manager"),
        )?;

        self.print_scenario(
            "Worker View",
            "How a Worker Agent sees the system state (executing tasks with tools).",
            &state,
            Some("worker"),
        )?;

        Ok(())
    }

    /// Show all scenarios
    pub fn show_all(&self) -> Result<()> {
        println!("SYSTEM PROMPT PREVIEW");
        println!(
            "This preview shows how the system prompt changes based on loaded files, plans, and agents."
        );

        self.show_empty()?;
        self.show_files()?;
        self.show_plan()?;
        self.show_agents()?;
        self.show_complete()?;
        self.show_full()?;
        self.show_agent_types()?;
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
    agents: bool,
    complete: bool,
    full: bool,
    agent_types: bool,
    config_path: Option<String>,
) -> Result<()> {
    let preview = PromptPreviewScenarios::new(config_path)?;

    // If no specific scenario is requested, show all
    if !all && !empty && !files && !plan && !agents && !complete && !full && !agent_types {
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

    if agents {
        preview.show_agents()?;
    }

    if complete {
        preview.show_complete()?;
    }

    if full {
        preview.show_full()?;
    }

    if agent_types {
        preview.show_agent_types()?;
    }

    Ok(())
}
