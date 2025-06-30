use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::actors::tools::planner::{TaskPlan, TaskStatus};
use crate::actors::{AgentStatus, AgentTaskResult, AgentType};
use crate::scope::Scope;
use crate::template::{self, TemplateContext, ToolInfo};

/// Errors that can occur when working with SystemState
#[derive(Debug, Snafu)]
pub enum SystemStateError {
    #[snafu(display("Failed to render system prompt template"))]
    TemplateRenderFailed {
        #[snafu(source)]
        source: minijinja::Error,
    },
}

pub type Result<T> = std::result::Result<T, SystemStateError>;

/// Represents a file's content and metadata for the system prompt
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: PathBuf,
    pub content: String,
    pub lines: usize,
    pub last_modified: std::time::SystemTime,
}

impl FileInfo {
    pub fn new(path: PathBuf, content: String, last_modified: std::time::SystemTime) -> Self {
        let lines = content.lines().count();
        Self {
            path,
            content,
            lines,
            last_modified,
        }
    }

    /// Get a truncated version of the content for display
    pub fn get_preview(&self, max_lines: usize) -> String {
        let lines: Vec<&str> = self.content.lines().collect();
        if lines.len() <= max_lines {
            self.content.clone()
        } else {
            let preview_lines = &lines[..max_lines];
            format!(
                "{}\n... ({} more lines)",
                preview_lines.join("\n"),
                lines.len() - max_lines
            )
        }
    }

    /// Format file with truncation for display
    pub fn format_with_limit(&self, max_lines: usize) -> String {
        let content = if self.lines <= max_lines {
            self.content.clone()
        } else {
            self.get_preview(max_lines)
        };

        format!(
            "## {}\n```\n{}\n```\n({} lines total)",
            self.path.display(),
            content,
            self.lines
        )
    }
}

impl std::fmt::Display for FileInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "## {}\n```\n{}\n```\n({} lines total)",
            self.path.display(),
            self.content,
            self.lines
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentDisplayStatus {
    Done(AgentTaskResult),
    InProgress,
    AwaitingManagerPlanApproval,
}

impl From<AgentStatus> for AgentDisplayStatus {
    fn from(value: AgentStatus) -> Self {
        match value {
            AgentStatus::Processing { .. } => Self::InProgress,
            AgentStatus::Wait { reason } => match reason {
                crate::actors::WaitReason::WaitingForPlanApproval { .. } => {
                    Self::AwaitingManagerPlanApproval
                }
                _ => Self::InProgress,
            },
            AgentStatus::Done(agent_task_result) => Self::Done(agent_task_result),
        }
    }
}

/// Information about a spawned agent and its assigned task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTaskInfo {
    pub agent_id: Scope,
    pub agent_role: String,
    pub task_description: String,
    pub status: AgentDisplayStatus,
    pub spawned_at: std::time::SystemTime,
}

impl AgentTaskInfo {
    pub fn new(
        agent_id: Scope,
        agent_type: AgentType,
        agent_role: String,
        task_description: String,
    ) -> Self {
        Self {
            agent_id,
            agent_role,
            task_description,
            status: AgentDisplayStatus::InProgress,
            spawned_at: std::time::SystemTime::now(),
        }
    }

    /// Get status icon using same style as planner
    /// Treat InProgress and Waiting as the same for right now.
    pub fn status_icon(&self) -> &'static str {
        match &self.status {
            AgentDisplayStatus::InProgress => "[~]",
            AgentDisplayStatus::Done(Ok(_)) => "[x]",
            AgentDisplayStatus::Done(Err(_)) => "[!]",
            AgentDisplayStatus::AwaitingManagerPlanApproval => "[*]",
        }
    }

    /// Format for display in system prompt
    /// Treat InProgress and Waiting as the same for right now.
    pub fn format_for_prompt(&self) -> String {
        // TODO: Update this maybe to use xml style tags??
        // Is this where we want to put all agent information?
        // What are the pros and cons of putting it here over putting it in normal messages?
        // let details = match &self.status {
        //     AgentStatus::Done(Ok(result)) => format!(" - {}", result),
        //     AgentStatus::Done(Err(error)) => format!(" - Error: {}", error),
        //     AgentStatus::AwaitingManager(awaiting) => match awaiting {
        //         TaskAwaitingManager::AwaitingPlanApproval(_) => {
        //             " - Awaiting plan approval".to_string()
        //         }
        //         TaskAwaitingManager::AwaitingMoreInformation { request, .. } => {
        //             format!(" - Needs: {}", info)
        //         }
        //     },
        //     AgentStatus::InProgress | AgentStatus::Wait { tool_call_id: _ } => {
        //         String::new()
        //     }
        // };
        let details = "".to_string();

        format!(
            "{} {} ({}): {}{}",
            self.status_icon(),
            self.agent_role,
            self.agent_id,
            self.task_description,
            details
        )
    }
}

/// Manages the system state that gets injected into the system prompt
#[derive(Debug, Clone)]
pub struct SystemState {
    /// Currently loaded files and their contents
    files: HashMap<PathBuf, FileInfo>,
    /// Current task plan if any
    current_plan: Option<TaskPlan>,
    /// Spawned agents and their tasks
    agents: HashMap<Scope, AgentTaskInfo>,
    /// Maximum number of lines to show per file in system prompt
    max_file_lines: usize,
    /// Track whether the state has been modified since last check
    modified: bool,
}

impl Default for SystemState {
    fn default() -> Self {
        Self {
            files: HashMap::new(),
            current_plan: None,
            agents: HashMap::new(),
            max_file_lines: 50, // Max lines per file
            modified: true,
        }
    }
}

impl SystemState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add or update a file in the system state
    pub fn update_file(
        &mut self,
        path: PathBuf,
        content: String,
        last_modified: std::time::SystemTime,
    ) {
        let file_info = FileInfo::new(path.clone(), content, last_modified);
        self.files.insert(path, file_info);
        self.modified = true;
    }

    /// Remove a file from the system state
    pub fn remove_file(&mut self, path: &PathBuf) {
        if self.files.remove(path).is_some() {
            self.modified = true;
        }
    }

    /// Update the current task plan
    pub fn update_plan(&mut self, plan: TaskPlan) {
        self.current_plan = Some(plan);
        self.modified = true;
    }

    /// Clear the current task plan
    pub fn clear_plan(&mut self) {
        if self.current_plan.is_some() {
            self.current_plan = None;
            self.modified = true;
        }
    }

    /// Get the current task plan
    pub fn get_plan(&self) -> Option<&TaskPlan> {
        self.current_plan.as_ref()
    }

    /// Add or update a spawned agent
    pub fn add_agent(&mut self, agent_info: AgentTaskInfo) {
        self.agents.insert(agent_info.agent_id.clone(), agent_info);
        self.modified = true;
    }

    /// Update an agent's task status
    pub fn update_agent_status(&mut self, agent_id: &Scope, status: AgentStatus) {
        if let Some(agent_info) = self.agents.get_mut(agent_id) {
            agent_info.status = status.into();
            self.modified = true;
        }
    }

    /// Remove an agent (when task is complete)
    pub fn remove_agent(&mut self, agent_id: &Scope) {
        if self.agents.remove(agent_id).is_some() {
            self.modified = true;
        }
    }

    /// Get all agents
    pub fn get_agents(&self) -> &HashMap<Scope, AgentTaskInfo> {
        &self.agents
    }

    /// Get agent count
    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    /// Get all files currently loaded
    pub fn get_files(&self) -> &HashMap<PathBuf, FileInfo> {
        &self.files
    }

    /// Get a specific file by path
    pub fn get_file(&self, path: &PathBuf) -> Option<&FileInfo> {
        self.files.get(path)
    }

    /// Check if a file is currently loaded
    pub fn has_file(&self, path: &PathBuf) -> bool {
        self.files.contains_key(path)
    }

    /// Get the number of currently loaded files
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Generate the files section for the system prompt
    pub fn render_files_section(&self) -> String {
        if self.files.is_empty() {
            return "No files currently loaded.".to_string();
        }

        let mut sections = Vec::new();

        // Sort files by path for consistent output
        let mut sorted_files: Vec<_> = self.files.iter().collect();
        sorted_files.sort_by_key(|(path, _)| path.as_path());

        for (_path, file_info) in sorted_files {
            let formatted_file = file_info.format_with_limit(self.max_file_lines);
            sections.push(formatted_file);
        }

        format!("## Currently Loaded Files\n\n{}", sections.join("\n\n"))
    }

    /// Generate the plan section for the system prompt
    pub fn render_plan_section(&self) -> String {
        match &self.current_plan {
            Some(plan) => {
                format!("{}", plan)
            }
            None => "No current task plan.".to_string(),
        }
    }

    /// Generate the agents section for the system prompt
    pub fn render_agents_section(&self) -> String {
        if self.agents.is_empty() {
            return "No agents currently spawned.".to_string();
        }

        let mut lines = vec!["## Spawned Agents and Tasks".to_string()];

        // Sort agents by spawn time for consistent output
        let mut sorted_agents: Vec<_> = self.agents.values().collect();
        sorted_agents.sort_by_key(|agent| agent.spawned_at);

        for (i, agent) in sorted_agents.iter().enumerate() {
            lines.push(format!("{}. {}", i + 1, agent.format_for_prompt()));
        }

        lines.join("\n")
    }

    /// Generate the complete system state context for templates
    pub fn to_template_context(&self) -> serde_json::Value {
        // Sort files by path for consistent output
        let mut sorted_files: Vec<_> = self.files.iter().collect();
        sorted_files.sort_by_key(|(path, _)| path.as_path());

        // Convert files to a list with path and content
        let files_list: Vec<serde_json::Value> = sorted_files
            .into_iter()
            .map(|(path, file_info)| {
                serde_json::json!({
                    "path": path.display().to_string(),
                    "content": file_info.content,
                    "lines": file_info.lines,
                    "preview": file_info.get_preview(self.max_file_lines)
                })
            })
            .collect();

        // Sort agents by spawn time for consistent output
        let mut sorted_agents: Vec<_> = self.agents.values().collect();
        sorted_agents.sort_by_key(|agent| agent.spawned_at);

        // Convert agents to a list with individual details
        let agents_list: Vec<serde_json::Value> = sorted_agents
            .into_iter()
            .map(|agent| {
                serde_json::json!({
                    "id": agent.agent_id.to_string(),
                    "role": agent.agent_role,
                    "task": agent.task_description,
                    "status": match &agent.status {
                        // TODO: Probably update this error display
                        AgentDisplayStatus::Done(_) => "done".to_string(),
                        AgentDisplayStatus::InProgress => "in_progress".to_string(),
                        AgentDisplayStatus::AwaitingManagerPlanApproval => "plan_awaiting_your_approval".to_string(),
                    },
                    "status_icon": agent.status_icon(),
                    "formatted": agent.format_for_prompt()
                })
            })
            .collect();

        serde_json::json!({
            "files": {
                "count": self.file_count(),
                "list": files_list,
                "section": self.render_files_section()
            },
            "plan": {
                "exists": self.current_plan.is_some(),
                "data": self.current_plan.as_ref().map(|plan| serde_json::json!({
                    "title": plan.title,
                    "tasks": plan.tasks.iter().map(|task| serde_json::json!({
                        "description": task.description,
                        "status": match task.status {
                            TaskStatus::Pending => "pending",
                            TaskStatus::InProgress => "in_progress",
                            TaskStatus::Completed => "completed",
                            TaskStatus::Skipped => "skipped",
                        },
                        "icon": task.status_icon()
                    })).collect::<Vec<_>>()
                })),
                "section": self.render_plan_section()
            },
            "agents": {
                "count": self.agent_count(),
                "list": agents_list,
                "section": self.render_agents_section()
            }
        })
    }

    /// Set the maximum lines per file for system prompt
    pub fn set_max_file_lines(&mut self, max_lines: usize) {
        self.max_file_lines = max_lines;
    }

    /// Check if the state has been modified
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Reset the modified flag
    pub fn reset_modified(&mut self) {
        self.modified = false;
    }

    /// Render the system prompt with the given template and tools
    pub fn render_system_prompt(
        &self,
        prompt_template: &str,
        tools: &[ToolInfo],
        whitelisted_commands: Vec<String>,
        agent_id: Scope,
    ) -> Result<String> {
        self.render_system_prompt_with_task(
            prompt_template,
            tools,
            whitelisted_commands,
            None,
            agent_id,
        )
    }

    /// Render the system prompt with the given template, tools, and task description
    pub fn render_system_prompt_with_task(
        &self,
        prompt_template: &str,
        tools: &[ToolInfo],
        whitelisted_commands: Vec<String>,
        task_description: Option<String>,
        agent_id: Scope,
    ) -> Result<String> {
        // Check if it's a template
        if !template::is_template(prompt_template) {
            return Ok(prompt_template.to_string());
        }

        // Build template context with task
        let context = TemplateContext::with_task(
            tools.to_vec(),
            whitelisted_commands,
            self,
            task_description,
            agent_id,
        );

        // Render the template
        template::render_template(prompt_template, &context).context(TemplateRenderFailedSnafu)
    }

    /// Render the system prompt with the given template, tools, task description, and role
    pub fn render_system_prompt_with_task_and_role(
        &self,
        prompt_template: &str,
        tools: &[ToolInfo],
        whitelisted_commands: Vec<String>,
        task_description: Option<String>,
        role: Option<String>,
        agent_id: Scope,
    ) -> Result<String> {
        // Check if it's a template
        if !template::is_template(prompt_template) {
            return Ok(prompt_template.to_string());
        }

        // Build template context with task and role
        let context = TemplateContext::with_task_and_role(
            tools.to_vec(),
            whitelisted_commands,
            self,
            task_description,
            role,
            agent_id,
        );

        // Render the template
        template::render_template(prompt_template, &context).context(TemplateRenderFailedSnafu)
    }
}

#[cfg(test)]
mod tests {
    use crate::actors::tools::planner::{Task, TaskStatus};

    use super::*;
    use std::time::SystemTime;

    #[test]
    fn test_file_info_creation() {
        let content = "line 1\nline 2\nline 3".to_string();
        let path = PathBuf::from("test.txt");
        let now = SystemTime::now();

        let file_info = FileInfo::new(path.clone(), content.clone(), now);

        assert_eq!(file_info.path, path);
        assert_eq!(file_info.content, content);
        assert_eq!(file_info.lines, 3);
        assert_eq!(file_info.last_modified, now);
    }

    #[test]
    fn test_file_info_preview() {
        let content = "line 1\nline 2\nline 3\nline 4\nline 5".to_string();
        let path = PathBuf::from("test.txt");
        let now = SystemTime::now();

        let file_info = FileInfo::new(path, content, now);

        let preview = file_info.get_preview(3);
        assert_eq!(preview, "line 1\nline 2\nline 3\n... (2 more lines)");

        let full = file_info.get_preview(10);
        assert_eq!(full, file_info.content);
    }

    #[test]
    fn test_system_state_file_operations() {
        let mut state = SystemState::new();
        let path = PathBuf::from("test.txt");
        let content = "test content".to_string();
        let now = SystemTime::now();

        // Initially no files
        assert_eq!(state.file_count(), 0);
        assert!(!state.has_file(&path));

        // Add file
        state.update_file(path.clone(), content.clone(), now);
        assert_eq!(state.file_count(), 1);
        assert!(state.has_file(&path));

        let file_info = state.get_file(&path).unwrap();
        assert_eq!(file_info.content, content);

        // Remove file
        state.remove_file(&path);
        assert_eq!(state.file_count(), 0);
        assert!(!state.has_file(&path));
    }

    #[test]
    fn test_system_state_plan_operations() {
        let mut state = SystemState::new();

        // Initially no plan
        assert!(state.get_plan().is_none());

        // Add plan
        let plan = TaskPlan {
            title: "Test Plan".to_string(),
            tasks: vec![],
        };
        state.update_plan(plan.clone());
        assert_eq!(state.get_plan().unwrap().title, "Test Plan");

        // Clear plan
        state.clear_plan();
        assert!(state.get_plan().is_none());
    }

    #[test]
    fn test_render_files_section_empty() {
        let state = SystemState::new();
        let section = state.render_files_section();
        assert_eq!(section, "No files currently loaded.");
    }

    #[test]
    fn test_render_files_section_with_files() {
        let mut state = SystemState::new();
        let path = PathBuf::from("test.txt");
        let content = "line 1\nline 2".to_string();
        let now = SystemTime::now();

        state.update_file(path, content, now);
        let section = state.render_files_section();

        assert!(section.contains("## Currently Loaded Files"));
        assert!(section.contains("## test.txt"));
        assert!(section.contains("line 1"));
        assert!(section.contains("(2 lines total)"));
    }

    #[test]
    fn test_render_plan_section_empty() {
        let state = SystemState::new();
        let section = state.render_plan_section();
        assert_eq!(section, "No current task plan.");
    }

    #[test]
    fn test_render_plan_section_with_plan() {
        let mut state = SystemState::new();
        let plan = TaskPlan {
            title: "Test Plan".to_string(),
            tasks: vec![
                Task {
                    description: "Task 1".to_string(),
                    status: TaskStatus::Completed,
                },
                Task {
                    description: "Task 2".to_string(),
                    status: TaskStatus::Pending,
                },
            ],
        };

        state.update_plan(plan);
        let section = state.render_plan_section();

        assert!(section.contains("## Current Task Plan: Test Plan"));
        assert!(section.contains("1. [x] Task 1"));
        assert!(section.contains("2. [ ] Task 2"));
    }

    #[test]
    fn test_to_template_context() {
        let mut state = SystemState::new();
        let path = PathBuf::from("test.txt");
        let content = "test".to_string();
        let now = SystemTime::now();

        state.update_file(path, content, now);

        let context = state.to_template_context();
        assert_eq!(context["files"]["count"], 1);
        assert!(
            context["files"]["section"]
                .as_str()
                .unwrap()
                .contains("test.txt")
        );
        assert_eq!(context["plan"]["exists"], false);
    }

    #[test]
    fn test_template_context_with_file_list() {
        let mut state = SystemState::new();
        let path1 = PathBuf::from("src/main.rs");
        let content1 = "fn main() {\n    println!(\"Hello\");\n}".to_string();
        let path2 = PathBuf::from("src/lib.rs");
        let content2 = "pub fn hello() {}".to_string();
        let now = SystemTime::now();

        state.update_file(path1.clone(), content1.clone(), now);
        state.update_file(path2.clone(), content2.clone(), now);

        let context = state.to_template_context();

        // Check files count
        assert_eq!(context["files"]["count"], 2);

        // Check files list
        let files_list = context["files"]["list"].as_array().unwrap();
        assert_eq!(files_list.len(), 2);

        // Files should be sorted by path
        assert_eq!(files_list[0]["path"], "src/lib.rs");
        assert_eq!(files_list[0]["content"], content2);
        assert_eq!(files_list[0]["lines"], 1);

        assert_eq!(files_list[1]["path"], "src/main.rs");
        assert_eq!(files_list[1]["content"], content1);
        assert_eq!(files_list[1]["lines"], 3);
    }

    #[test]
    fn test_template_context_with_plan_data() {
        let mut state = SystemState::new();
        let plan = TaskPlan {
            title: "Test Plan".to_string(),
            tasks: vec![
                Task {
                    description: "Task 1".to_string(),
                    status: TaskStatus::Completed,
                },
                Task {
                    description: "Task 2".to_string(),
                    status: TaskStatus::InProgress,
                },
                Task {
                    description: "Task 3".to_string(),
                    status: TaskStatus::Pending,
                },
            ],
        };

        state.update_plan(plan);
        let context = state.to_template_context();

        assert_eq!(context["plan"]["exists"], true);

        let plan_data = &context["plan"]["data"];
        assert_eq!(plan_data["title"], "Test Plan");

        let tasks = plan_data["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 3);

        assert_eq!(tasks[0]["description"], "Task 1");
        assert_eq!(tasks[0]["status"], "completed");
        assert_eq!(tasks[0]["icon"], "[x]");

        assert_eq!(tasks[1]["description"], "Task 2");
        assert_eq!(tasks[1]["status"], "in_progress");
        assert_eq!(tasks[1]["icon"], "[~]");

        assert_eq!(tasks[2]["description"], "Task 3");
        assert_eq!(tasks[2]["status"], "pending");
        assert_eq!(tasks[2]["icon"], "[ ]");
    }

    #[test]
    fn test_template_context_with_agents_list() {
        let mut state = SystemState::new();

        let agent1 = AgentTaskInfo::new(
            Scope::new(),
            AgentType::Worker,
            "Software Engineer".to_string(),
            "Implement feature X".to_string(),
        );
        let agent1_id = agent1.agent_id;

        let agent2 = AgentTaskInfo::new(
            Scope::new(),
            AgentType::Worker,
            "QA Engineer".to_string(),
            "Test feature X".to_string(),
        );
        let agent2_id = agent2.agent_id;

        state.add_agent(agent1);
        state.add_agent(agent2);

        // Update one agent's status
        state.update_agent_status(
            &agent1_id,
            AgentStatus::Done(Ok(crate::actors::AgentTaskResultOk {
                summary: "Completed".to_string(),
                success: true,
            })),
        );

        let context = state.to_template_context();

        assert_eq!(context["agents"]["count"], 2);

        let agents_list = context["agents"]["list"].as_array().unwrap();
        assert_eq!(agents_list.len(), 2);

        // Find the done agent
        let done_agent = agents_list
            .iter()
            .find(|a| a["status"] == "done")
            .unwrap();

        assert_eq!(done_agent["role"], "Software Engineer");
        assert_eq!(done_agent["task"], "Implement feature X");
        assert_eq!(done_agent["status_icon"], "[x]");

        // Find the in-progress agent
        let in_progress_agent = agents_list
            .iter()
            .find(|a| a["status"] == "in_progress")
            .unwrap();

        assert_eq!(in_progress_agent["role"], "QA Engineer");
        assert_eq!(in_progress_agent["task"], "Test feature X");
        assert_eq!(in_progress_agent["status_icon"], "[~]");
    }

    #[test]
    fn test_system_prompt_template_rendering() {
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
            Scope::new(),
        );

        let result = template::render_template(template, &context).unwrap();

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
    fn test_is_template_detection() {
        // Should detect templates
        assert!(template::is_template("Hello {{ name }}!"));
        assert!(template::is_template("{% if condition %}yes{% endif %}"));
        assert!(template::is_template("{# This is a comment #}"));
        assert!(template::is_template(
            "Mixed {{ var }} and {% tag %} content"
        ));

        // Should not detect templates
        assert!(!template::is_template("Hello World!"));
        assert!(!template::is_template("Just plain text"));
        assert!(!template::is_template("{ not a template }"));
        assert!(!template::is_template("[ brackets ]"));
    }

    #[test]
    fn test_non_template_passthrough() {
        let plain_prompt = "You are a helpful assistant.";
        let system_state = SystemState::new();
        let context = TemplateContext::new(vec![], vec![], &system_state, Scope::new());

        // Should return the same string if it's not a template
        assert!(!template::is_template(plain_prompt));

        // Verify that non-templates work as well in render_template
        let result = template::render_template(plain_prompt, &context).unwrap();
        assert_eq!(result, plain_prompt);
    }

    #[test]
    fn test_system_state_render_system_prompt() {
        let template = r#"You are an AI assistant.

{% if files.count > 0 -%}
Currently loaded files: {{ files.count }}
{% endif %}

{% if plan.exists -%}
Current plan: active
{% endif %}"#;

        let mut state = SystemState::new();

        // Test with no files or plan
        let result = state
            .render_system_prompt(template, &[], vec![], Scope::new())
            .unwrap();
        assert!(result.contains("You are an AI assistant."));
        assert!(!result.contains("Currently loaded files"));
        assert!(!result.contains("Current plan"));

        // Add a file
        state.update_file(
            PathBuf::from("test.txt"),
            "test content".to_string(),
            SystemTime::now(),
        );

        let result = state
            .render_system_prompt(template, &[], vec![], Scope::new())
            .unwrap();
        assert!(result.contains("Currently loaded files: 1"));
        assert!(!result.contains("Current plan"));

        // Add a plan
        state.update_plan(TaskPlan {
            title: "Test Plan".to_string(),
            tasks: vec![],
        });

        let result = state
            .render_system_prompt(template, &[], vec![], Scope::new())
            .unwrap();
        assert!(result.contains("Currently loaded files: 1"));
        assert!(result.contains("Current plan: active"));
    }

    #[test]
    fn test_modified_flag_tracking() {
        let mut state = SystemState::new();

        // Initially modified (to ensure first render happens)
        assert!(state.is_modified());
        
        // Reset clears the flag
        state.reset_modified();
        assert!(!state.is_modified());

        // Adding a file sets modified
        state.update_file(
            PathBuf::from("test.txt"),
            "content".to_string(),
            SystemTime::now(),
        );
        assert!(state.is_modified());

        // Reset clears the flag
        state.reset_modified();
        assert!(!state.is_modified());

        // Updating plan sets modified
        state.update_plan(TaskPlan {
            title: "Plan".to_string(),
            tasks: vec![],
        });
        assert!(state.is_modified());

        state.reset_modified();
        assert!(!state.is_modified());

        // Removing a file sets modified
        let path = PathBuf::from("test.txt");
        state.update_file(path.clone(), "content".to_string(), SystemTime::now());
        state.reset_modified();
        state.remove_file(&path);
        assert!(state.is_modified());

        // Clearing plan sets modified
        state.reset_modified();
        state.clear_plan();
        assert!(state.is_modified());
    }

    #[test]
    fn test_render_system_prompt_with_task() {
        let template = r#"You are an AI assistant.

{% if task -%}
Your assigned task: {{ task }}
{% else -%}
No specific task assigned.
{% endif %}

Available tools: {{ tools|length }}"#;

        let state = SystemState::new();
        let tools = vec![ToolInfo {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
        }];

        // Test without task
        let result = state
            .render_system_prompt_with_task(template, &tools, vec![], None, Scope::new())
            .unwrap();
        assert!(result.contains("No specific task assigned"));
        assert!(result.contains("Available tools: 1"));

        // Test with task
        let result = state
            .render_system_prompt_with_task(
                template,
                &tools,
                vec![],
                Some("Implement user authentication".to_string()),
                Scope::new(),
            )
            .unwrap();
        assert!(result.contains("Your assigned task: Implement user authentication"));
        assert!(!result.contains("No specific task assigned"));
    }

    #[test]
    fn test_render_system_prompt_with_task_and_role() {
        let template = r#"You are an AI assistant.

{% if role -%}
Your role: {{ role }}
{% else -%}
No specific role assigned.
{% endif %}

{% if task -%}
Your assigned task: {{ task }}
{% else -%}
No specific task assigned.
{% endif %}

Available tools: {{ tools|length }}"#;

        let state = SystemState::new();
        let tools = vec![ToolInfo {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
        }];

        // Test without role and task
        let result = state
            .render_system_prompt_with_task_and_role(template, &tools, vec![], None, None, Scope::new())
            .unwrap();
        assert!(result.contains("No specific role assigned"));
        assert!(result.contains("No specific task assigned"));
        assert!(result.contains("Available tools: 1"));

        // Test with role and task
        let result = state
            .render_system_prompt_with_task_and_role(
                template,
                &tools,
                vec![],
                Some("Implement user authentication".to_string()),
                Some("Backend Developer".to_string()),
                Scope::new(),
            )
            .unwrap();
        assert!(result.contains("Your role: Backend Developer"));
        assert!(result.contains("Your assigned task: Implement user authentication"));
        assert!(!result.contains("No specific role assigned"));
        assert!(!result.contains("No specific task assigned"));
    }

    #[test]
    fn test_xml_style_template_rendering() {
        use std::path::PathBuf;
        use std::time::SystemTime;

        // Create a template that uses XML tags
        let xml_template = r#"You are an AI assistant.

{% if files.count > 0 -%}
<read_and_edited_files>
{% for file in files.list -%}
<file path="{{ file.path }}">{{ file.content }}</file>
{% endfor %}
</read_and_edited_files>
{% endif %}

{% if plan.exists -%}
<plan title="{{ plan.data.title }}">
{% for task in plan.data.tasks -%}
<task status="{{ task.status }}">{{ task.description }}</task>
{% endfor %}
</plan>
{% endif %}

{% if agents.count > 0 -%}
<spawned_agents>
{% for agent in agents.list -%}
<agent id="{{ agent.id }}" role="{{ agent.role }}" status="{{ agent.status }}">
<task>{{ agent.task }}</task>
</agent>
{% endfor %}
</spawned_agents>
{% endif %}"#;

        let mut state = SystemState::new();

        // Add files
        state.update_file(
            PathBuf::from("src/main.rs"),
            "fn main() {\n    println!(\"Hello\");\n}".to_string(),
            SystemTime::now(),
        );
        state.update_file(
            PathBuf::from("src/lib.rs"),
            "pub fn hello() {}".to_string(),
            SystemTime::now(),
        );

        // Add plan
        state.update_plan(TaskPlan {
            title: "Sprint 1".to_string(),
            tasks: vec![
                Task {
                    description: "Setup project".to_string(),
                    status: TaskStatus::Completed,
                },
                Task {
                    description: "Implement feature".to_string(),
                    status: TaskStatus::InProgress,
                },
            ],
        });

        // Add agents
        let agent1 = AgentTaskInfo::new(
            Scope::new(),
            AgentType::Worker,
            "Developer".to_string(),
            "Build the API".to_string(),
        );
        let agent1_id = agent1.agent_id;
        state.add_agent(agent1);
        state.update_agent_status(
            &agent1_id,
            AgentStatus::Done(Ok(crate::actors::AgentTaskResultOk {
                summary: "Done".to_string(),
                success: true,
            })),
        );

        // Render the template
        let result = state
            .render_system_prompt(xml_template, &[], vec![], Scope::new())
            .unwrap();

        // Verify XML structure for files
        assert!(result.contains("<read_and_edited_files>"));
        assert!(result.contains("</read_and_edited_files>"));
        assert!(result.contains("<file path=\"src/lib.rs\">pub fn hello() {}</file>"));
        assert!(result.contains(
            "<file path=\"src/main.rs\">fn main() {\n    println!(\"Hello\");\n}</file>"
        ));

        // Verify XML structure for plan
        assert!(result.contains("<plan title=\"Sprint 1\">"));
        assert!(result.contains("</plan>"));
        assert!(result.contains("<task status=\"completed\">Setup project</task>"));
        assert!(result.contains("<task status=\"in_progress\">Implement feature</task>"));

        // Verify XML structure for agents
        assert!(result.contains("<spawned_agents>"));
        assert!(result.contains("</spawned_agents>"));
        assert!(result.contains("role=\"Developer\""));
        assert!(result.contains("status=\"completed\""));
        assert!(result.contains("<task>Build the API</task>"));
    }
}
