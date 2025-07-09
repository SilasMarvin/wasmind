use crate::scope::Scope;
use crate::system_state::SystemState;
use minijinja::{Environment, context};
use serde::Serialize;

/// Context for rendering templates
#[derive(Debug, Clone, Serialize)]
pub struct TemplateContext {
    /// List of available tools
    pub tools: Vec<ToolInfo>,
    /// Current date and time
    pub current_datetime: String,
    /// Operating system
    pub os: String,
    /// Architecture
    pub arch: String,
    /// Current working directory
    pub cwd: String,
    /// Whitelisted commands
    pub whitelisted_commands: Vec<String>,
    /// System state with files and plans
    pub system_state: serde_json::Value,
    /// Agent's assigned task description
    pub task: Option<String>,
    /// Agent's unique identifier (scope)
    pub id: String,
    /// Agent's role (e.g., "Software Engineer", "QA Tester", "Project Lead Manager")
    pub role: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
}

impl TemplateContext {
    /// Create a new template context with system state, task description, and role
    pub fn new(
        tools: Vec<ToolInfo>,
        whitelisted_commands: Vec<String>,
        system_state: &SystemState,
        task_description: Option<String>,
        role: String,
        agent_id: Scope,
    ) -> Self {
        let cwd = std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        Self {
            tools,
            current_datetime: chrono::Utc::now()
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            cwd,
            whitelisted_commands,
            system_state: system_state.to_template_context(),
            task: task_description,
            id: agent_id.to_string(),
            role,
        }
    }
}

/// Render a template string with the given context
pub fn render_template(
    template_str: &str,
    context: &TemplateContext,
) -> Result<String, minijinja::Error> {
    let mut env = Environment::new();

    // Add the template
    env.add_template("system_prompt", template_str)?;

    // Get the template
    let tmpl = env.get_template("system_prompt")?;

    // Render with context
    let ctx = context! {
        tools => &context.tools,
        current_datetime => &context.current_datetime,
        os => &context.os,
        arch => &context.arch,
        cwd => &context.cwd,
        whitelisted_commands => &context.whitelisted_commands,
        files => &context.system_state["files"],
        plan => &context.system_state["plan"],
        agents => &context.system_state["agents"],
        task => &context.task,
        id => &context.id,
        role => &context.role,
    };

    tmpl.render(ctx)
}

/// Check if a string contains Jinja template syntax
pub fn is_template(s: &str) -> bool {
    s.contains("{{") || s.contains("{%") || s.contains("{#")
}

#[cfg(test)]
mod tests {
    use crate::actors::AgentType;

    use super::*;

    #[tokio::test]
    async fn test_simple_template() {
        let template = "Hello {{ name }}!";
        let mut env = Environment::new();
        env.add_template("test", template).unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! { name => "World" }).unwrap();
        assert_eq!(result, "Hello World!");
    }

    #[tokio::test]
    async fn test_is_template() {
        assert!(is_template("Hello {{ name }}!"));
        assert!(is_template("{% if true %}yes{% endif %}"));
        assert!(is_template("{# comment #}"));
        assert!(!is_template("Hello World!"));
    }

    #[test]
    fn test_system_prompt_template() {
        use crate::system_state::SystemState;

        let system_state = SystemState::new();
        let context = TemplateContext::new(
            vec![
                ToolInfo {
                    name: "execute_command".to_string(),
                    description: "Execute a shell command".to_string(),
                },
                ToolInfo {
                    name: "read_file".to_string(),
                    description: "Read file contents".to_string(),
                },
            ],
            vec!["ls".to_string(), "cat".to_string()],
            &system_state,
            None,
            "Filler Role".to_string(),
            Scope::new(),
        );

        let template = r#"You are a helpful assistant with access to {{ tools|length }} tools.

Available tools:
{% for tool in tools -%}
- {{ tool.name }}: {{ tool.description }}
{% endfor %}

Current date and time: {{ current_datetime }}
System: {{ os }} ({{ arch }})
Working directory: {{ cwd }}"#;

        let result = render_template(template, &context).unwrap();

        // Check that tool count is dynamic
        assert!(result.contains("access to 2 tools"));

        // Check that tools are listed
        assert!(result.contains("- execute_command: Execute a shell command"));
        assert!(result.contains("- read_file: Read file contents"));

        // Check that system info is included
        assert!(result.contains("Current date and time:"));
        assert!(result.contains("System:"));
        assert!(result.contains("Working directory:"));

        // Check that cwd is not "unknown" (should be actual directory)
        assert!(!result.contains("Working directory: unknown"));
    }

    #[test]
    fn test_cwd_template_variable() {
        use crate::system_state::SystemState;

        let system_state = SystemState::new();
        let context = TemplateContext::new(
            vec![],
            vec![],
            &system_state,
            None,
            "Filler Role".to_string(),
            Scope::new(),
        );

        let template = "Current directory: {{ cwd }}";
        let result = render_template(template, &context).unwrap();

        // Should not be unknown and should contain a path
        assert!(!result.contains("unknown"));
        assert!(result.contains("Current directory: "));
        assert!(result.len() > "Current directory: ".len());
    }

    #[test]
    fn test_xml_template_with_files_list() {
        use crate::actors::tools::file_reader::FileReader;
        use crate::system_state::SystemState;
        use std::fs;
        use std::sync::Arc;
        use tempfile::TempDir;

        // Create temporary files
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");
        let data_path = temp_dir.path().join("data.json");

        fs::write(&config_path, "[settings]\nvalue = 42").unwrap();
        fs::write(&data_path, r#"{"key": "value"}"#).unwrap();

        // Create FileReader and load files
        let mut file_reader = FileReader::default();
        file_reader
            .read_and_cache_file(&config_path, None, None)
            .unwrap();
        file_reader
            .read_and_cache_file(&data_path, None, None)
            .unwrap();

        // Create SystemState with FileReader
        let file_reader_arc = Arc::new(std::sync::Mutex::new(file_reader));
        let system_state = SystemState::with_file_reader(file_reader_arc);

        let context = TemplateContext::new(
            vec![],
            vec![],
            &system_state,
            None,
            "Filler Role".to_string(),
            Scope::new(),
        );

        let template = r#"Files:
{% for file in files.list -%}
<file path="{{ file.path }}">
{{ file.content }}
</file>
{% endfor %}"#;

        let result = render_template(template, &context).unwrap();

        // Check both files are rendered with XML tags
        assert!(result.contains("<file path="));
        assert!(result.contains("config.toml"));
        assert!(result.contains("1|[settings]"));
        assert!(result.contains("2|value = 42"));
        assert!(result.contains("</file>"));

        assert!(result.contains("data.json"));
        assert!(result.contains(r#"1|{"key": "value"}"#));
    }

    #[test]
    fn test_template_error_handling() {
        use crate::system_state::SystemState;

        let system_state = SystemState::new();
        let context = TemplateContext::new(
            vec![],
            vec![],
            &system_state,
            None,
            "Filler Role".to_string(),
            Scope::new(),
        );

        // Test invalid template syntax
        let invalid_template = "Hello {{ name"; // Missing closing braces
        let result = render_template(invalid_template, &context);
        assert!(result.is_err());

        // Test undefined variable - minijinja might be configured to not error
        let undefined_var_template = "Value: {{ undefined_variable }}";
        let result = render_template(undefined_var_template, &context);
        // Check if it errors or renders as empty/undefined
        match result {
            Ok(rendered) => {
                // If it doesn't error, it should at least not contain the variable name
                assert!(!rendered.contains("undefined_variable"));
            }
            Err(_) => {
                // This is also acceptable - undefined variables cause errors
            }
        }

        // Test invalid filter
        let invalid_filter = "{{ tools|invalid_filter }}";
        let result = render_template(invalid_filter, &context);
        assert!(result.is_err());
    }

    #[test]
    fn test_template_with_optional_variables() {
        use crate::system_state::SystemState;

        let system_state = SystemState::new();
        let context = TemplateContext::new(
            vec![],
            vec![],
            &system_state,
            None,
            "Filler Role".to_string(),
            Scope::new(),
        );

        // Test template that gracefully handles missing data
        let template = r#"System State Report:
===================

Tools: {{ tools|length }} available
{% if tools|length > 0 -%}
Tool List:
{% for tool in tools -%}
  - {{ tool.name }}
{% endfor %}
{% endif %}

Files: {{ files.count }} loaded
{% if files.count > 0 -%}
File Paths:
{% for file in files.list -%}
  - {{ file.path }}
{% endfor %}
{% endif %}

Plan: {% if plan.exists %}Active - "{{ plan.data.title }}"{% else %}No active plan{% endif %}

Agents: {{ agents.count }} spawned
{% if agents.count > 0 -%}
Agent List:
{% for agent in agents.list -%}
  - {{ agent.role }} ({{ agent.status }})
{% endfor %}
{% endif %}

Task: {% if task %}{{ task }}{% else %}No specific task{% endif %}"#;

        // Should render successfully even with all empty/missing data
        let result = render_template(template, &context).unwrap();
        assert!(result.contains("Tools: 0 available"));
        assert!(result.contains("Files: 0 loaded"));
        assert!(result.contains("Plan: No active plan"));
        assert!(result.contains("Agents: 0 spawned"));
        assert!(result.contains("Task: No specific task"));

        // Verify that conditional sections are not rendered
        assert!(!result.contains("Tool List:"));
        assert!(!result.contains("File Paths:"));
        assert!(!result.contains("Agent List:"));
    }

    #[test]
    fn test_agents_list_template() {
        use crate::actors::AgentStatus;
        use crate::scope::Scope;
        use crate::system_state::{AgentTaskInfo, SystemState};

        let mut system_state = SystemState::new();

        let agent1 = AgentTaskInfo::new(
            Scope::new(),
            AgentType::Worker,
            "Backend Developer".to_string(),
            "Create REST API".to_string(),
        );
        let agent1_id = agent1.agent_id;

        system_state.add_agent(agent1);
        system_state.update_agent_status(
            &agent1_id,
            AgentStatus::Processing {
                id: uuid::Uuid::new_v4(),
            },
        );

        let context = TemplateContext::new(
            vec![],
            vec![],
            &system_state,
            None,
            "Filler Role".to_string(),
            Scope::new(),
        );

        let template = r#"Active Agents:
{% for agent in agents.list -%}
- {{ agent.role }} ({{ agent.status }}): {{ agent.task }}
{% endfor %}"#;

        let result = render_template(template, &context).unwrap();

        assert!(result.contains("Active Agents:"));
        assert!(result.contains("- Backend Developer (in_progress): Create REST API"));
    }

    #[test]
    fn test_whitelisted_commands_template() {
        use crate::system_state::SystemState;

        let system_state = SystemState::new();
        let commands = vec!["ls".to_string(), "git".to_string(), "cargo".to_string()];
        let context = TemplateContext::new(
            vec![],
            commands,
            &system_state,
            None,
            "Filler Role".to_string(),
            Scope::new(),
        );

        // Test template with whitelisted_commands
        let template = r#"Allowed commands:
{% if whitelisted_commands -%}
{% for cmd in whitelisted_commands -%}
- {{ cmd }}
{% endfor %}
Total: {{ whitelisted_commands|length }} commands
{% else -%}
No commands whitelisted
{% endif %}"#;

        let result = render_template(template, &context).unwrap();
        assert!(result.contains("- ls"));
        assert!(result.contains("- git"));
        assert!(result.contains("- cargo"));
        assert!(result.contains("Total: 3 commands"));
    }

    #[test]
    fn test_empty_whitelisted_commands() {
        use crate::system_state::SystemState;

        let system_state = SystemState::new();
        let context = TemplateContext::new(
            vec![],
            vec![],
            &system_state,
            None,
            "Filler Role".to_string(),
            Scope::new(),
        );

        let template = r#"{% if whitelisted_commands and whitelisted_commands|length > 0 -%}
Commands: {{ whitelisted_commands|join(", ") }}
{% else -%}
No whitelisted commands available
{% endif %}"#;

        let result = render_template(template, &context).unwrap();
        assert!(result.contains("No whitelisted commands available"));
        assert!(!result.contains("Commands:"));
    }

    #[test]
    fn test_plan_data_template() {
        use crate::actors::tools::planner::{Task, TaskPlan, TaskStatus};
        use crate::system_state::SystemState;

        let mut system_state = SystemState::new();
        system_state.update_plan(TaskPlan {
            title: "Q1 Goals".to_string(),
            tasks: vec![
                Task {
                    description: "Launch v2.0".to_string(),
                    status: TaskStatus::Completed,
                },
                Task {
                    description: "Add monitoring".to_string(),
                    status: TaskStatus::Pending,
                },
            ],
        });

        let context = TemplateContext::new(
            vec![],
            vec![],
            &system_state,
            None,
            "Filler Role".to_string(),
            Scope::new(),
        );

        let template = r#"{% if plan.exists -%}
Current Plan: {{ plan.data.title }}
Tasks:
{% for task in plan.data.tasks -%}
{{ task.icon }} {{ task.description }} ({{ task.status }})
{% endfor %}
{% endif %}"#;

        let result = render_template(template, &context).unwrap();

        assert!(result.contains("Current Plan: Q1 Goals"));
        assert!(result.contains("[x] Launch v2.0 (completed)"));
        assert!(result.contains("[ ] Add monitoring (pending)"));
    }

    #[test]
    fn test_agent_id_template() {
        use crate::system_state::SystemState;

        let system_state = SystemState::new();
        let test_scope = Scope::new();
        let context = TemplateContext::new(
            vec![],
            vec![],
            &system_state,
            None,
            "Filler Role".to_string(),
            test_scope,
        );

        // Test template that uses the agent id
        let template = r#"Agent ID: {{ id }}
Your unique identifier is: {{ id }}
Agent {{ id }} is ready to assist."#;

        let result = render_template(template, &context).unwrap();
        let expected_id = test_scope.to_string();

        assert!(result.contains(&format!("Agent ID: {}", expected_id)));
        assert!(result.contains(&format!("Your unique identifier is: {}", expected_id)));
        assert!(result.contains(&format!("Agent {} is ready to assist.", expected_id)));
    }

    #[test]
    fn test_agent_id_with_task_template() {
        use crate::system_state::SystemState;

        let system_state = SystemState::new();
        let test_scope = Scope::new();
        let task_description = Some("Implement feature X".to_string());

        let context = TemplateContext::new(
            vec![],
            vec![],
            &system_state,
            task_description.clone(),
            "Filler Role".to_string(),
            test_scope,
        );

        // Test template that uses both id and task
        let template = r#"Agent {{ id }} is working on: {{ task }}
{% if task -%}
Agent {{ id }} has been assigned the task: {{ task }}
{% else -%}
Agent {{ id }} has no specific task assigned.
{% endif %}"#;

        let result = render_template(template, &context).unwrap();
        let expected_id = test_scope.to_string();

        assert!(result.contains(&format!(
            "Agent {} is working on: Implement feature X",
            expected_id
        )));
        assert!(result.contains(&format!(
            "Agent {} has been assigned the task: Implement feature X",
            expected_id
        )));
        assert!(!result.contains("has no specific task assigned"));
    }

    #[test]
    fn test_system_prompt_with_agent_id() {
        use crate::system_state::SystemState;

        let system_state = SystemState::new();
        let test_scope = Scope::new();

        // Test a realistic system prompt template that includes agent ID
        let template = r#"You are Assistant {{ id }}, a helpful AI agent.

Your unique identifier is: {{ id }}

Available tools: {{ tools|length }}
Current time: {{ current_datetime }}
Working directory: {{ cwd }}

{% if task -%}
Your assigned task: {{ task }}
{% endif %}

Remember that you are Agent {{ id }}. Always include your ID when communicating important updates."#;

        let context = TemplateContext::new(
            vec![ToolInfo {
                name: "test_tool".to_string(),
                description: "A test tool".to_string(),
            }],
            vec![],
            &system_state,
            Some("Test the new feature".to_string()),
            "Filler Role".to_string(),
            test_scope,
        );

        let result = render_template(template, &context).unwrap();
        let expected_id = test_scope.to_string();

        // Verify agent ID appears in multiple places
        assert!(result.contains(&format!("You are Assistant {}", expected_id)));
        assert!(result.contains(&format!("Your unique identifier is: {}", expected_id)));
        assert!(result.contains(&format!("Remember that you are Agent {}", expected_id)));

        // Verify other template variables still work
        assert!(result.contains("Available tools: 1"));
        assert!(result.contains("Your assigned task: Test the new feature"));
        assert!(result.contains("Current time:"));
        assert!(result.contains("Working directory:"));

        // Output the actual rendered result to show the functionality works
        println!("=== Rendered System Prompt with Agent ID ===");
        println!("{}", result);
        println!("=== End ===");
    }

    #[test]
    fn test_template_with_role() {
        use crate::system_state::SystemState;

        let system_state = SystemState::new();
        let test_scope = Scope::new();

        // Test template that uses role
        let template = r#"You are a {{ role }} agent.

Your role: {{ role }}

{% if task -%}
Your task: {{ task }}
{% endif %}

Agent ID: {{ id }}
Tools available: {{ tools|length }}"#;

        // Test with role
        let context = TemplateContext::new(
            vec![ToolInfo {
                name: "test_tool".to_string(),
                description: "A test tool".to_string(),
            }],
            vec![],
            &system_state,
            Some("Build a web app".to_string()),
            "Software Engineer".to_string(),
            test_scope,
        );

        let result = render_template(template, &context).unwrap();
        assert!(result.contains("You are a Software Engineer agent"));
        assert!(result.contains("Your role: Software Engineer"));
        assert!(!result.contains("No specific role assigned"));
        assert!(result.contains("Your task: Build a web app"));
        assert!(result.contains(&format!("Agent ID: {}", test_scope)));
        assert!(result.contains("Tools available: 1"));
    }

    #[test]
    fn test_template_with_file_reader_integration() {
        use crate::actors::tools::file_reader::FileReader;
        use crate::system_state::SystemState;
        use std::fs;
        use std::sync::Arc;
        use tempfile::TempDir;

        // Create temporary files
        let temp_dir = TempDir::new().unwrap();
        let file1_path = temp_dir.path().join("config.rs");
        let file2_path = temp_dir.path().join("main.rs");

        fs::write(&file1_path, "pub const VERSION: &str = \"1.0.0\";").unwrap();
        fs::write(
            &file2_path,
            "fn main() {\n    println!(\"Hello, world!\");\n}",
        )
        .unwrap();

        // Create FileReader and load files
        let mut file_reader = FileReader::default();
        file_reader
            .read_and_cache_file(&file1_path, None, None)
            .unwrap();
        file_reader
            .read_and_cache_file(&file2_path, Some(1), Some(2))
            .unwrap();

        // Create SystemState with FileReader
        let file_reader_arc = Arc::new(std::sync::Mutex::new(file_reader));
        let system_state = SystemState::with_file_reader(file_reader_arc);

        // Create template context
        let context = TemplateContext::new(
            vec![],
            vec![],
            &system_state,
            Some("Build a web app".to_string()),
            "Filler Role".to_string(),
            Scope::new(),
        );

        // Test file count template
        let template = "Files loaded: {{ files.count }}";
        let result = render_template(template, &context).unwrap();
        assert_eq!(result, "Files loaded: 2");

        // Test file list template with content
        let template = r#"Files:
{% for file in files.list -%}
- {{ file.path }} ({{ file.lines }} lines)
{% endfor %}"#;
        let result = render_template(template, &context).unwrap();
        assert!(result.contains("config.rs (1 lines)"));
        assert!(result.contains("main.rs (3 lines)")); // 3 lines because partial content includes numbering

        // Test file content in template
        let template = r#"{% for file in files.list -%}
## {{ file.path }}
```
{{ file.content }}
```
{% endfor %}"#;
        let result = render_template(template, &context).unwrap();
        assert!(result.contains("1|pub const VERSION: &str = \"1.0.0\";"));
        assert!(result.contains("1|fn main() {"));
        assert!(result.contains("2|    println!(\"Hello, world!\");"));
    }

    #[test]
    fn test_file_reader_integration_with_partial_content() {
        use crate::actors::tools::file_reader::FileReader;
        use crate::system_state::SystemState;
        use std::fs;
        use std::sync::Arc;
        use tempfile::TempDir;

        // Create temporary file with multiple lines
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("large_file.rs");
        let content =
            "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10";
        fs::write(&file_path, content).unwrap();

        // Create FileReader and load partial content
        let mut file_reader = FileReader::default();
        // Read only lines 3-6
        file_reader
            .read_and_cache_file(&file_path, Some(3), Some(6))
            .unwrap();

        // Create SystemState with FileReader
        let file_reader_arc = Arc::new(std::sync::Mutex::new(file_reader));
        let system_state = SystemState::with_file_reader(file_reader_arc);

        // Create template context
        let context = TemplateContext::new(
            vec![],
            vec![],
            &system_state,
            Some("Build a web app".to_string()),
            "Filler Role".to_string(),
            Scope::new(),
        );

        // Test that partial content is rendered with omitted lines indicators
        let template = r#"{% for file in files.list -%}
{{ file.content }}
{% endfor %}"#;
        let result = render_template(template, &context).unwrap();

        // Debug output removed for cleaner test output

        // Should contain the omitted lines indicators and the actual content
        assert!(result.contains("[... 2 lines omitted ...]")); // Lines 1-2 omitted
        assert!(result.contains("3|line 3"));
        assert!(result.contains("4|line 4"));
        assert!(result.contains("5|line 5"));
        assert!(result.contains("6|line 6"));
        assert!(result.contains("[... 4 lines omitted ...]")); // Lines 7-10 omitted
    }

    #[test]
    fn test_system_prompt_template_rendering() {
        use crate::system_state::SystemState;

        let template = r#"You are an AI assistant with access to {{ tools|length }} tools.

Available tools:
{% for tool in tools -%}
- {{ tool.name }}: {{ tool.description }}
{% endfor %}

Current time: {{ current_datetime }}
System: {{ os }} ({{ arch }})

{% if whitelisted_commands -%}
Whitelisted commands: {{ whitelisted_commands|join(', ') }}
{% endif %}"#;

        let system_state = SystemState::new();
        let context = TemplateContext::new(
            vec![
                ToolInfo {
                    name: "command".to_string(),
                    description: "Execute system commands".to_string(),
                },
                ToolInfo {
                    name: "file_reader".to_string(),
                    description: "Read file contents".to_string(),
                },
                ToolInfo {
                    name: "edit_file".to_string(),
                    description: "Edit files".to_string(),
                },
                ToolInfo {
                    name: "planner".to_string(),
                    description: "Create and track task plans".to_string(),
                },
            ],
            vec!["ls".to_string(), "cat".to_string(), "git".to_string()],
            &system_state,
            Some("Build a web app".to_string()),
            "Filler Role".to_string(),
            Scope::new(),
        );

        let result = render_template(template, &context).unwrap();

        // Verify the rendered output contains expected content
        assert!(result.contains("You are an AI assistant with access to 4 tools"));
        assert!(result.contains("- command: Execute system commands"));
        assert!(result.contains("- file_reader: Read file contents"));
        assert!(result.contains("- edit_file: Edit files"));
        assert!(result.contains("- planner: Create and track task plans"));
        assert!(result.contains("Current time:"));
        assert!(result.contains("System:"));
        assert!(result.contains("Whitelisted commands: ls, cat, git"));
    }

    #[test]
    fn test_non_template_passthrough() {
        use crate::system_state::SystemState;

        let plain_prompt = "You are a helpful assistant.";
        let system_state = SystemState::new();
        let context = TemplateContext::new(
            vec![],
            vec![],
            &system_state,
            Some("Build a web app".to_string()),
            "Filler Role".to_string(),
            Scope::new(),
        );

        // Should return the same string if it's not a template
        assert!(!is_template(plain_prompt));

        // Verify that non-templates work as well in render_template
        let result = render_template(plain_prompt, &context).unwrap();
        assert_eq!(result, plain_prompt);
    }
}
