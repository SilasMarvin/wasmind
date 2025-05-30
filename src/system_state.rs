use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::template::{self, TemplateContext, ToolInfo};
use crate::actors::tools::planner::TaskPlan;

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

/// Manages the system state that gets injected into the system prompt
#[derive(Debug, Clone)]
pub struct SystemState {
    /// Currently loaded files and their contents
    files: HashMap<PathBuf, FileInfo>,
    /// Current task plan if any
    current_plan: Option<TaskPlan>,
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
            max_file_lines: 50, // Max lines per file
            modified: false,
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

    /// Generate the complete system state context for templates
    pub fn to_template_context(&self) -> serde_json::Value {
        serde_json::json!({
            "files": {
                "count": self.file_count(),
                "section": self.render_files_section()
            },
            "plan": {
                "exists": self.current_plan.is_some(),
                "section": self.render_plan_section()
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
    ) -> Result<String> {
        // Check if it's a template
        if !template::is_template(prompt_template) {
            return Ok(prompt_template.to_string());
        }

        // Build template context
        let context = TemplateContext::new(
            tools.to_vec(),
            whitelisted_commands,
            self,
        );

        // Render the template
        template::render_template(prompt_template, &context)
            .context(TemplateRenderFailedSnafu)
    }
}

#[cfg(test)]
mod tests {
    use crate::tools::planner::{Task, TaskStatus};

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
        assert!(template::is_template("Mixed {{ var }} and {% tag %} content"));
        
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
        let context = TemplateContext::new(
            vec![],
            vec![],
            &system_state,
        );

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
        let result = state.render_system_prompt(
            template,
            &[],
            vec![],
        ).unwrap();
        assert!(result.contains("You are an AI assistant."));
        assert!(!result.contains("Currently loaded files"));
        assert!(!result.contains("Current plan"));

        // Add a file
        state.update_file(
            PathBuf::from("test.txt"),
            "test content".to_string(),
            SystemTime::now(),
        );

        let result = state.render_system_prompt(
            template,
            &[],
            vec![],
        ).unwrap();
        assert!(result.contains("Currently loaded files: 1"));
        assert!(!result.contains("Current plan"));

        // Add a plan
        state.update_plan(TaskPlan {
            title: "Test Plan".to_string(),
            tasks: vec![],
        });

        let result = state.render_system_prompt(
            template,
            &[],
            vec![],
        ).unwrap();
        assert!(result.contains("Currently loaded files: 1"));
        assert!(result.contains("Current plan: active"));
    }

    #[test]
    fn test_modified_flag_tracking() {
        let mut state = SystemState::new();
        
        // Initially not modified
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
}
