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
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
}

impl TemplateContext {
    /// Create a new template context with system state
    pub fn new(
        tools: Vec<ToolInfo>,
        whitelisted_commands: Vec<String>,
        system_state: &SystemState,
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
            task: None,
        }
    }

    /// Create a new template context with system state and task description
    pub fn with_task(
        tools: Vec<ToolInfo>,
        whitelisted_commands: Vec<String>,
        system_state: &SystemState,
        task_description: Option<String>,
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
    };

    tmpl.render(ctx)
}

/// Check if a string contains Jinja template syntax
pub fn is_template(s: &str) -> bool {
    s.contains("{{") || s.contains("{%") || s.contains("{#")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_template() {
        let template = "Hello {{ name }}!";
        let mut env = Environment::new();
        env.add_template("test", template).unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! { name => "World" }).unwrap();
        assert_eq!(result, "Hello World!");
    }

    #[test]
    fn test_is_template() {
        assert!(is_template("Hello {{ name }}!"));
        assert!(is_template("{% if true %}yes{% endif %}"));
        assert!(is_template("{# comment #}"));
        assert!(!is_template("Hello World!"));
    }

    #[test]
    fn test_template_context() {
        use crate::system_state::SystemState;

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
            ],
            vec!["ls".to_string(), "cat".to_string()],
            &system_state,
        );

        let template = "You are an assistant with access to {{ tools|length }} tools.";
        let result = render_template(template, &context).unwrap();
        assert_eq!(result, "You are an assistant with access to 2 tools.");
    }

    #[test]
    fn test_template_with_files_and_plan() {
        use crate::system_state::SystemState;
        use std::path::PathBuf;
        use std::time::SystemTime;

        let mut system_state = SystemState::new();
        system_state.update_file(
            PathBuf::from("test.txt"),
            "Hello World".to_string(),
            SystemTime::now(),
        );

        let context = TemplateContext::new(vec![], vec![], &system_state);

        let template = "Files loaded: {{ files.count }}";
        let result = render_template(template, &context).unwrap();
        assert_eq!(result, "Files loaded: 1");
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
        let context = TemplateContext::new(vec![], vec![], &system_state);

        let template = "Current directory: {{ cwd }}";
        let result = render_template(template, &context).unwrap();

        // Should not be unknown and should contain a path
        assert!(!result.contains("unknown"));
        assert!(result.contains("Current directory: "));
        assert!(result.len() > "Current directory: ".len());
    }

    #[test]
    fn test_xml_template_with_files_list() {
        use crate::system_state::SystemState;
        use std::path::PathBuf;
        use std::time::SystemTime;

        let mut system_state = SystemState::new();
        system_state.update_file(
            PathBuf::from("config.toml"),
            "[settings]\nvalue = 42".to_string(),
            SystemTime::now(),
        );
        system_state.update_file(
            PathBuf::from("data.json"),
            r#"{"key": "value"}"#.to_string(),
            SystemTime::now(),
        );

        let context = TemplateContext::new(vec![], vec![], &system_state);

        let template = r#"Files:
{% for file in files.list -%}
<file path="{{ file.path }}">
{{ file.content }}
</file>
{% endfor %}"#;

        let result = render_template(template, &context).unwrap();

        // Check both files are rendered with XML tags
        assert!(result.contains("<file path=\"config.toml\">"));
        assert!(result.contains("[settings]\nvalue = 42"));
        assert!(result.contains("</file>"));
        
        assert!(result.contains("<file path=\"data.json\">"));
        assert!(result.contains(r#"{"key": "value"}"#));
    }

    #[test]
    fn test_agents_list_template() {
        use crate::system_state::{SystemState, AgentTaskInfo};
        use crate::actors::AgentTaskStatus;
        use uuid::Uuid;

        let mut system_state = SystemState::new();
        
        let agent1 = AgentTaskInfo::new(
            Uuid::new_v4(),
            "Backend Developer".to_string(),
            "Create REST API".to_string(),
        );
        let agent1_id = agent1.agent_id;
        
        system_state.add_agent(agent1);
        system_state.update_agent_status(&agent1_id, AgentTaskStatus::InProgress);

        let context = TemplateContext::new(vec![], vec![], &system_state);

        let template = r#"Active Agents:
{% for agent in agents.list -%}
- {{ agent.role }} ({{ agent.status }}): {{ agent.task }}
{% endfor %}"#;

        let result = render_template(template, &context).unwrap();

        assert!(result.contains("Active Agents:"));
        assert!(result.contains("- Backend Developer (in_progress): Create REST API"));
    }

    #[test]
    fn test_plan_data_template() {
        use crate::system_state::SystemState;
        use crate::actors::tools::planner::{TaskPlan, Task, TaskStatus};

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

        let context = TemplateContext::new(vec![], vec![], &system_state);

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
}
