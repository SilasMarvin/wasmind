use hive_actor_utils::common_messages::assistant::{Section, SystemPromptContent, SystemPromptContribution};
use minijinja::{Environment, context};
use regex::Regex;
use std::collections::HashMap;
use chrono::{Local, Utc};

/// Configuration for system prompt template rendering
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SystemPromptConfig {
    /// Base template for the overall system prompt structure
    #[serde(default = "default_base_template")]
    pub base_template: String,
    /// User-defined template overrides for specific contribution keys
    #[serde(default)]
    pub overrides: HashMap<String, String>,
    /// List of contribution keys to exclude from the prompt
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Default content for specific sections
    #[serde(default)]
    pub defaults: Option<HashMap<String, String>>,
}

fn default_base_template() -> String {
    r#"{% for section_name, contributions in sections -%}
## {{ section_name | title }}

{% for contribution in contributions -%}
{{ contribution }}

{% endfor -%}
{% endfor -%}"#
        .to_string()
}

impl Default for SystemPromptConfig {
    fn default() -> Self {
        Self {
            base_template: default_base_template(),
            overrides: HashMap::new(),
            exclude: Vec::new(),
            defaults: None,
        }
    }
}

/// A rendered contribution ready for inclusion in the system prompt
#[derive(Debug, Clone)]
pub struct RenderedContribution {
    pub key: String,
    pub content: String,
    pub priority: i32,
    pub section: Section,
}

/// System prompt template renderer
pub struct SystemPromptRenderer {
    config: SystemPromptConfig,
    contributions: HashMap<String, SystemPromptContribution>,
    key_validation_regex: Regex,
    agent_scope: String,
}

impl SystemPromptRenderer {
    pub fn new(config: SystemPromptConfig, agent_scope: String) -> Self {
        let mut renderer = Self {
            config,
            contributions: HashMap::new(),
            key_validation_regex: Regex::new(r"^[a-z0-9_-]+:[a-z0-9_-]+$").unwrap(),
            agent_scope,
        };
        
        // Automatically add system context variables
        renderer.add_system_context();
        
        // Add default contributions
        renderer.add_default_contributions();
        
        renderer
    }
    
    /// Adds default system context variables that are always available
    fn add_system_context(&mut self) {
        // Current working directory
        if let Ok(cwd) = std::env::current_dir() {
            let _ = self.add_contribution_internal(SystemPromptContribution {
                agent: self.agent_scope.clone(),
                key: "system:current_directory".to_string(),
                content: SystemPromptContent::Text(format!("Current working directory: {}", cwd.display())),
                priority: 0, // Low priority so it appears at the end
                section: Some(Section::SystemContext),
            });
        }
        
        // Current date and time
        let now = Utc::now();
        let local_time = Local::now();
        let _ = self.add_contribution_internal(SystemPromptContribution {
            agent: self.agent_scope.clone(),
            key: "system:datetime".to_string(),
            content: SystemPromptContent::Text(format!(
                "Current date and time: {} UTC (Local: {})",
                now.format("%Y-%m-%d %H:%M:%S"),
                local_time.format("%Y-%m-%d %H:%M:%S %Z")
            )),
            priority: 0,
            section: Some(Section::SystemContext),
        });
        
        // Operating system information
        let os_info = format!(
            "Operating system: {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        let _ = self.add_contribution_internal(SystemPromptContribution {
            agent: self.agent_scope.clone(),
            key: "system:os_info".to_string(),
            content: SystemPromptContent::Text(os_info),
            priority: 0,
            section: Some(Section::SystemContext),
        });
    }

    /// Adds default system prompt contributions from config
    fn add_default_contributions(&mut self) {
        if let Some(defaults) = self.config.defaults.clone() {
            for (section_name, content) in defaults {
                let section = Section::from(section_name.as_str());
                let key = format!("config:{}", section_name);
                
                let _ = self.add_contribution_internal(SystemPromptContribution {
                    agent: self.agent_scope.clone(),
                    key,
                    content: SystemPromptContent::Text(content),
                    priority: 500, // Medium priority
                    section: Some(section),
                });
            }
        }
    }

    /// Validates a contribution key format
    pub fn validate_key(&self, key: &str) -> Result<(), String> {
        if self.key_validation_regex.is_match(key) {
            Ok(())
        } else {
            Err(format!(
                "Invalid key format '{}'. Must contain exactly one colon (:) in the format 'actor_type:contribution_name'. Only lowercase letters, numbers, hyphens (-), and underscores (_) are allowed. Example: 'main_manager:identity'",
                key
            ))
        }
    }

    /// Adds or updates a system prompt contribution
    pub fn add_contribution(
        &mut self,
        contribution: SystemPromptContribution,
    ) -> Result<(), String> {
        // Only accept contributions targeting this agent
        if contribution.agent != self.agent_scope {
            return Ok(()); // Silently ignore contributions for other agents
        }

        // Validate the key format
        self.validate_key(&contribution.key)?;

        // Check if this contribution is excluded
        if self.config.exclude.contains(&contribution.key) {
            return Ok(()); // Silently ignore excluded contributions
        }

        self.contributions
            .insert(contribution.key.clone(), contribution);
        Ok(())
    }

    /// Internal method to add contributions without validation (used for system context)
    fn add_contribution_internal(&mut self, contribution: SystemPromptContribution) -> Result<(), String> {
        self.contributions
            .insert(contribution.key.clone(), contribution);
        Ok(())
    }

    /// Removes a contribution by key
    pub fn remove_contribution(&mut self, key: &str) {
        self.contributions.remove(key);
    }

    /// Renders a single contribution to text
    fn render_contribution(
        &self,
        contribution: &SystemPromptContribution,
    ) -> Result<String, String> {
        match &contribution.content {
            SystemPromptContent::Text(text) => Ok(text.clone()),
            SystemPromptContent::Data {
                data,
                default_template,
            } => {
                // Check if user has provided a custom template override
                let template_str = self
                    .config
                    .overrides
                    .get(&contribution.key)
                    .unwrap_or(default_template);

                // Render the template with the data
                let env = Environment::new();
                let template = env.template_from_str(template_str).map_err(|e| {
                    format!("Template parse error for key '{}': {}", contribution.key, e)
                })?;

                template.render(context!(data => data)).map_err(|e| {
                    format!(
                        "Template render error for key '{}': {}",
                        contribution.key, e
                    )
                })
            }
        }
    }

    /// Organizes contributions by section and priority
    fn organize_contributions(&self) -> Result<Vec<RenderedContribution>, String> {
        let mut rendered = Vec::new();

        for contribution in self.contributions.values() {
            let content = self.render_contribution(contribution)?;
            let section = contribution
                .section
                .clone()
                .unwrap_or(Section::Custom("default".to_string()));

            rendered.push(RenderedContribution {
                key: contribution.key.clone(),
                content,
                priority: contribution.priority,
                section,
            });
        }

        // Sort by section name first, then by priority (highest first) within sections
        rendered.sort_by(|a, b| {
            match a.section.cmp(&b.section) {
                std::cmp::Ordering::Equal => b.priority.cmp(&a.priority), // Higher priority first
                other => other,
            }
        });

        Ok(rendered)
    }

    /// Renders the complete system prompt
    pub fn render(&self) -> Result<String, String> {
        let rendered_contributions = self.organize_contributions()?;

        // Group contributions by section
        let mut sections: HashMap<Section, Vec<String>> = HashMap::new();
        for contribution in rendered_contributions {
            sections
                .entry(contribution.section)
                .or_insert_with(Vec::new)
                .push(contribution.content);
        }

        // Sort sections by enum ordering (Identity, Context, etc., then Custom alphabetically)
        let mut section_pairs: Vec<(Section, Vec<String>)> = sections.into_iter().collect();
        section_pairs.sort_by(|a, b| a.0.cmp(&b.0));
        
        let sorted_sections: Vec<(String, Vec<String>)> = section_pairs
            .into_iter()
            .map(|(section, content)| (section.display_name(), content))
            .collect();

        // Render the base template
        let env = Environment::new();
        let template = env
            .template_from_str(&self.config.base_template)
            .map_err(|e| format!("Base template parse error: {}", e))?;

        template
            .render(context!(sections => sorted_sections))
            .map_err(|e| format!("Base template render error: {}", e))
    }

    /// Returns the current configuration
    pub fn config(&self) -> &SystemPromptConfig {
        &self.config
    }

    /// Updates the configuration
    pub fn update_config(&mut self, config: SystemPromptConfig) {
        self.config = config;
    }

    /// Returns all current contributions
    pub fn contributions(&self) -> &HashMap<String, SystemPromptContribution> {
        &self.contributions
    }

    /// Clears all contributions
    pub fn clear_contributions(&mut self) {
        self.contributions.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_agent_scope() -> String {
        "test-agent-scope".to_string()
    }

    #[test]
    fn test_key_validation() {
        let renderer = SystemPromptRenderer::new(SystemPromptConfig::default(), test_agent_scope());

        // Valid keys
        assert!(renderer.validate_key("file_reader:open_files").is_ok());
        assert!(renderer.validate_key("git-status:branch_info").is_ok());
        assert!(renderer.validate_key("shell:current_directory").is_ok());
        assert!(renderer.validate_key("actor123:data-1").is_ok());

        // Invalid keys
        assert!(renderer.validate_key("FileReader:OpenFiles").is_err()); // camelCase
        assert!(renderer.validate_key("file_reader").is_err()); // missing colon
        assert!(renderer.validate_key("file_reader::data").is_err()); // double colon
        assert!(renderer.validate_key("file@reader:data").is_err()); // special char
        assert!(renderer.validate_key("").is_err()); // empty
    }

    #[test]
    fn test_text_contribution() {
        let mut renderer =
            SystemPromptRenderer::new(SystemPromptConfig::default(), test_agent_scope());

        let contribution = SystemPromptContribution {
            agent: test_agent_scope(),
            key: "shell:cwd".to_string(),
            content: SystemPromptContent::Text("Current directory: /home/user/project".to_string()),
            priority: 100,
            section: Some(Section::Context),
        };

        renderer.add_contribution(contribution).unwrap();
        let result = renderer.render().unwrap();

        assert!(result.contains("Current directory: /home/user/project"));
        assert!(result.contains("## Context"));
    }

    #[test]
    fn test_data_contribution_with_default_template() {
        let mut renderer =
            SystemPromptRenderer::new(SystemPromptConfig::default(), test_agent_scope());

        let contribution = SystemPromptContribution {
            agent: test_agent_scope(),
            key: "file_reader:files".to_string(),
            content: SystemPromptContent::Data {
                data: json!({
                    "files": [
                        {"name": "main.rs", "lines": 100},
                        {"name": "lib.rs", "lines": 50}
                    ]
                }),
                default_template: r#"Files:
{% for file in data.files -%}
- {{ file.name }} ({{ file.lines }} lines)
{% endfor %}"#
                    .to_string(),
            },
            priority: 200,
            section: Some(Section::Context),
        };

        renderer.add_contribution(contribution).unwrap();
        let result = renderer.render().unwrap();

        assert!(result.contains("Files:"));
        assert!(result.contains("- main.rs (100 lines)"));
        assert!(result.contains("- lib.rs (50 lines)"));
    }

    #[test]
    fn test_template_override() {
        let mut config = SystemPromptConfig::default();
        config.overrides.insert(
            "file_reader:files".to_string(),
            "Custom template: {{ data.files | length }} files".to_string(),
        );

        let mut renderer = SystemPromptRenderer::new(config, test_agent_scope());

        let contribution = SystemPromptContribution {
            agent: test_agent_scope(),
            key: "file_reader:files".to_string(),
            content: SystemPromptContent::Data {
                data: json!({"files": [1, 2, 3]}),
                default_template: "Default template".to_string(),
            },
            priority: 100,
            section: Some(Section::Context),
        };

        renderer.add_contribution(contribution).unwrap();
        let result = renderer.render().unwrap();

        assert!(result.contains("Custom template: 3 files"));
        assert!(!result.contains("Default template"));
    }

    #[test]
    fn test_contribution_exclusion() {
        let mut config = SystemPromptConfig::default();
        config.exclude.push("excluded:item".to_string());

        let mut renderer = SystemPromptRenderer::new(config, test_agent_scope());

        let included = SystemPromptContribution {
            agent: test_agent_scope(),
            key: "included:item".to_string(),
            content: SystemPromptContent::Text("This should appear".to_string()),
            priority: 100,
            section: Some(Section::Context),
        };

        let excluded = SystemPromptContribution {
            agent: test_agent_scope(),
            key: "excluded:item".to_string(),
            content: SystemPromptContent::Text("This should NOT appear".to_string()),
            priority: 100,
            section: Some(Section::Context),
        };

        renderer.add_contribution(included).unwrap();
        renderer.add_contribution(excluded).unwrap(); // Should be silently ignored

        let result = renderer.render().unwrap();

        assert!(result.contains("This should appear"));
        assert!(!result.contains("This should NOT appear"));
    }

    #[test]
    fn test_priority_ordering() {
        let mut renderer =
            SystemPromptRenderer::new(SystemPromptConfig::default(), test_agent_scope());

        let low_priority = SystemPromptContribution {
            agent: test_agent_scope(),
            key: "test:low".to_string(),
            content: SystemPromptContent::Text("Low priority item".to_string()),
            priority: 10,
            section: Some(Section::Context),
        };

        let high_priority = SystemPromptContribution {
            agent: test_agent_scope(),
            key: "test:high".to_string(),
            content: SystemPromptContent::Text("High priority item".to_string()),
            priority: 100,
            section: Some(Section::Context),
        };

        renderer.add_contribution(low_priority).unwrap();
        renderer.add_contribution(high_priority).unwrap();

        let result = renderer.render().unwrap();

        // High priority should come before low priority
        let high_pos = result.find("High priority item").unwrap();
        let low_pos = result.find("Low priority item").unwrap();
        assert!(high_pos < low_pos);
    }

    #[test]
    fn test_multiple_sections() {
        let mut renderer =
            SystemPromptRenderer::new(SystemPromptConfig::default(), test_agent_scope());

        let context_item = SystemPromptContribution {
            agent: test_agent_scope(),
            key: "test:context".to_string(),
            content: SystemPromptContent::Text("Context item".to_string()),
            priority: 100,
            section: Some(Section::Context),
        };

        let tools_item = SystemPromptContribution {
            agent: test_agent_scope(),
            key: "test:tools".to_string(),
            content: SystemPromptContent::Text("Tools item".to_string()),
            priority: 100,
            section: Some(Section::Tools),
        };

        renderer.add_contribution(context_item).unwrap();
        renderer.add_contribution(tools_item).unwrap();

        let result = renderer.render().unwrap();

        assert!(result.contains("## Context"));
        assert!(result.contains("## Tools"));
        assert!(result.contains("Context item"));
        assert!(result.contains("Tools item"));
    }

    #[test]
    fn test_default_section() {
        let mut renderer =
            SystemPromptRenderer::new(SystemPromptConfig::default(), test_agent_scope());

        let contribution = SystemPromptContribution {
            agent: test_agent_scope(),
            key: "test:item".to_string(),
            content: SystemPromptContent::Text("No section specified".to_string()),
            priority: 100,
            section: None, // No section specified
        };

        renderer.add_contribution(contribution).unwrap();
        let result = renderer.render().unwrap();
        
        assert!(result.contains("## Default"));
        assert!(result.contains("No section specified"));
    }

    #[test]
    fn test_agent_filtering() {
        let mut renderer =
            SystemPromptRenderer::new(SystemPromptConfig::default(), test_agent_scope());

        let for_this_agent = SystemPromptContribution {
            agent: test_agent_scope(),
            key: "test:correct".to_string(),
            content: SystemPromptContent::Text("For this agent".to_string()),
            priority: 100,
            section: Some(Section::Context),
        };

        let for_other_agent = SystemPromptContribution {
            agent: "other-agent-scope".to_string(),
            key: "test:wrong".to_string(),
            content: SystemPromptContent::Text("For other agent".to_string()),
            priority: 100,
            section: Some(Section::Context),
        };

        renderer.add_contribution(for_this_agent).unwrap();
        renderer.add_contribution(for_other_agent).unwrap(); // Should be ignored

        let result = renderer.render().unwrap();

        assert!(result.contains("For this agent"));
        assert!(!result.contains("For other agent"));
    }

    #[test]
    fn test_system_context_variables() {
        let renderer = SystemPromptRenderer::new(SystemPromptConfig::default(), test_agent_scope());
        let result = renderer.render().unwrap();


        // Check that system context variables are included
        assert!(result.contains("## System Context")); // Matches actual output with display name
        assert!(result.contains("Current working directory:"));
        assert!(result.contains("Current date and time:"));
        assert!(result.contains("Operating system:"));
        
        // Verify the system context keys are in the contributions
        assert!(renderer.contributions.contains_key("system:current_directory"));
        assert!(renderer.contributions.contains_key("system:datetime"));
        assert!(renderer.contributions.contains_key("system:os_info"));
    }
}

