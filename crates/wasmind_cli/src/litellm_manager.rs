use serde::{Deserialize, Serialize};
use snafu::{Location, ResultExt, Snafu, location};
use std::collections::HashMap;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::{sleep, timeout};
use tracing::{debug, info, warn};

use crate::config::LiteLLMConfig;

#[derive(Debug, Snafu)]
pub enum LiteLLMError {
    #[snafu(display("Docker command failed"))]
    DockerCommand {
        #[snafu(implicit)]
        location: Location,
        #[snafu(source)]
        source: std::io::Error,
    },

    #[snafu(display("Docker not available: {message}"))]
    DockerNotAvailable {
        message: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Container failed to start: {message}"))]
    ContainerStartFailed {
        message: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Health check failed after {attempts} attempts"))]
    HealthCheckFailed {
        attempts: u32,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("HTTP request failed during health check"))]
    HealthCheckRequest {
        #[snafu(implicit)]
        location: Location,
        #[snafu(source)]
        source: reqwest::Error,
    },

    #[snafu(display("Failed to create config file: {message}"))]
    ConfigFileCreation {
        message: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to write config file"))]
    ConfigFileWrite {
        #[snafu(implicit)]
        location: Location,
        #[snafu(source)]
        source: std::io::Error,
    },

    #[snafu(display("Failed to serialize config"))]
    ConfigSerialization {
        #[snafu(implicit)]
        location: Location,
        #[snafu(source)]
        source: serde_yaml::Error,
    },

    #[snafu(display("Unhealthy endpoints detected: {} unhealthy endpoints", unhealthy_endpoints.len()))]
    UnhealthyEndpoints {
        unhealthy_endpoints: Vec<serde_json::Value>,
        #[snafu(implicit)]
        location: Location,
    },
}

/// Static list of environment variables to pass through to LiteLLM container
const LITELLM_ENV_VARS: &[&str] = &[
    // OpenAI
    "OPENAI_API_KEY",
    "OPENAI_API_BASE",
    "OPENAI_ORGANIZATION",
    "OPENAI_PROJECT",
    // Anthropic
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_API_BASE",
    // Google/Gemini
    "GOOGLE_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "VERTEX_AI_PROJECT_ID",
    "VERTEX_AI_LOCATION",
    // Azure
    "AZURE_API_KEY",
    "AZURE_API_BASE",
    "AZURE_API_VERSION",
    "AZURE_OPENAI_ENDPOINT",
    "AZURE_OPENAI_API_KEY",
    // Other providers
    "COHERE_API_KEY",
    "DEEPSEEK_API_KEY",
    "GROQ_API_KEY",
    "TOGETHER_API_KEY",
    "REPLICATE_API_TOKEN",
    "HUGGINGFACE_API_KEY",
    "HF_TOKEN",
    "MISTRAL_API_KEY",
    "PERPLEXITYAI_API_KEY",
    "CLAUDE_API_KEY",
    // AWS Bedrock
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_REGION",
    "AWS_PROFILE",
    // Ollama
    "OLLAMA_API_BASE",
    // LiteLLM specific
    "LITELLM_MASTER_KEY",
    "LITELLM_SALT_KEY",
    "LITELLM_LOG_LEVEL",
    // Cerebras
    "CEREBRAS_API_KEY",
    // Open Router
    "OPENROUTER_API_KEY",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiteLLMModelEntry {
    pub model_name: String,
    pub litellm_params: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiteLLMConfigFile {
    pub model_list: Vec<LiteLLMModelEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiteLLMHealthResponse {
    pub healthy_endpoints: Vec<serde_json::Value>,
    pub unhealthy_endpoints: Vec<serde_json::Value>,
    pub healthy_count: u32,
    pub unhealthy_count: u32,
}

pub struct LiteLLMManager {
    config: LiteLLMConfig,
    container_id: Option<String>,
}

impl Drop for LiteLLMManager {
    fn drop(&mut self) {
        if let Some(container_id) = &self.container_id {
            tracing::info!("Stopping LiteLLM container on Drop: {}", container_id);
            // Use blocking command since we can't use async in Drop
            match std::process::Command::new("docker")
                .args(["stop", container_id])
                .output()
            {
                Ok(output) => {
                    if output.status.success() {
                        tracing::info!("LiteLLM container stopped successfully on Drop");
                    } else {
                        tracing::warn!(
                            "Docker stop failed on Drop: {}",
                            String::from_utf8_lossy(&output.stderr)
                        );
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to execute docker stop on Drop: {}", e);
                }
            }
        } else {
            tracing::debug!("LiteLLM Drop called but no container was running");
        }
    }
}

impl LiteLLMManager {
    pub fn new(config: LiteLLMConfig) -> Self {
        Self {
            config,
            container_id: None,
        }
    }

    /// Generate LiteLLM config from the configuration
    pub fn generate_config(litellm_config: &LiteLLMConfig) -> Result<String, LiteLLMError> {
        let mut model_list = Vec::new();

        // Convert all model definitions to LiteLLM entries
        for model_def in &litellm_config.models {
            // Convert toml::Value to serde_json::Value for each parameter
            let litellm_params: HashMap<String, serde_json::Value> = model_def
                .litellm_params
                .iter()
                .map(|(k, v)| (k.clone(), Self::convert_toml_to_json(v.clone())))
                .collect();

            model_list.push(LiteLLMModelEntry {
                model_name: model_def.model_name.clone(),
                litellm_params,
            });
        }

        if model_list.is_empty() {
            return Err(LiteLLMError::ConfigFileCreation {
                message: "No models configured".to_string(),
                location: location!(),
            });
        }

        let model_count = model_list.len();
        let config_file = LiteLLMConfigFile { model_list };
        let yaml_content = serde_yaml::to_string(&config_file).context(ConfigSerializationSnafu)?;

        info!("Generated LiteLLM config for {} models", model_count);
        debug!("Config content: {}", yaml_content);

        Ok(yaml_content)
    }

    fn convert_toml_to_json(toml: toml::Value) -> serde_json::Value {
        match toml {
            toml::Value::String(s) => serde_json::Value::String(s),
            toml::Value::Integer(i) => serde_json::Value::Number(i.into()),
            toml::Value::Float(f) => {
                let n =
                    serde_json::Number::from_f64(f).expect("float infinite and nan not allowed");
                serde_json::Value::Number(n)
            }
            toml::Value::Boolean(b) => serde_json::Value::Bool(b),
            toml::Value::Array(arr) => {
                serde_json::Value::Array(arr.into_iter().map(Self::convert_toml_to_json).collect())
            }
            toml::Value::Table(table) => serde_json::Value::Object(
                table
                    .into_iter()
                    .map(|(k, v)| (k, Self::convert_toml_to_json(v)))
                    .collect(),
            ),
            toml::Value::Datetime(dt) => serde_json::Value::String(dt.to_string()),
        }
    }

    pub async fn start(&mut self) -> Result<(), LiteLLMError> {
        info!("Starting LiteLLM Docker container...");

        // Docker is required for containerized LiteLLM deployment
        Self::check_docker_available().await?;

        // Stop any existing container with our name
        Self::cleanup_existing_container(&self.config.container_name).await?;

        // Generate config content
        let config_content = Self::generate_config(&self.config)?;

        // Collect environment variables
        let env_vars = Self::collect_env_vars(&self.config.env_overrides);

        // Start the container
        let container_id = Self::start_container(&self.config, &config_content, env_vars).await?;
        self.container_id = Some(container_id);

        // Start streaming container logs in background for debugging
        let container_name_for_logs = self.config.container_name.clone();
        tokio::spawn(async move {
            if let Err(e) = Self::stream_container_logs(&container_name_for_logs).await {
                warn!("Container log streaming failed: {}", e);
            }
        });

        info!(
            "LiteLLM container started successfully at {}",
            self.config.get_base_url()
        );

        self.wait_for_health().await?;

        Ok(())
    }

    async fn check_docker_available() -> Result<(), LiteLLMError> {
        let output = Command::new("docker")
            .arg("--version")
            .output()
            .await
            .context(DockerCommandSnafu)?;

        if !output.status.success() {
            return Err(LiteLLMError::DockerNotAvailable {
                message: "Docker command failed".to_string(),
                location: location!(),
            });
        }
        Ok(())
    }

    async fn cleanup_existing_container(container_name: &str) -> Result<(), LiteLLMError> {
        // Try to stop existing container
        let _ = Command::new("docker")
            .args(["stop", container_name])
            .output()
            .await;

        // Try to remove existing container
        let _ = Command::new("docker")
            .args(["rm", container_name])
            .output()
            .await;

        debug!("Cleaned up any existing container: {}", container_name);
        Ok(())
    }

    fn collect_env_vars(overrides: &HashMap<String, String>) -> Vec<(String, String)> {
        let mut env_vars = Vec::new();

        // Collect known LiteLLM environment variables from current environment
        for &env_var in LITELLM_ENV_VARS {
            if let Ok(value) = std::env::var(env_var) {
                env_vars.push((env_var.to_string(), value));
                debug!("Found environment variable: {}", env_var);
            }
        }

        // Apply overrides
        for (key, value) in overrides {
            env_vars.retain(|(k, _)| k != key);
            env_vars.push((key.clone(), value.clone()));
            debug!("Applied override for: {}", key);
        }

        info!(
            "Collected {} environment variables for LiteLLM container",
            env_vars.len()
        );
        env_vars
    }

    async fn start_container(
        config: &LiteLLMConfig,
        config_content: &str,
        env_vars: Vec<(String, String)>,
    ) -> Result<String, LiteLLMError> {
        let mut cmd = Command::new("docker");
        cmd.arg("run")
            .arg("--rm")
            .arg("-d") // Detached mode - runs container in background and returns immediately
            .arg("--name")
            .arg(&config.container_name)
            .arg("-p")
            .arg(format!("{}:4000", config.port));

        info!(
            "Starting LiteLLM container: {} with {} environment variables",
            config.container_name,
            env_vars.len()
        );
        debug!("Docker image: {}, port: {}", config.image, config.port);

        // Add environment variables
        for (key, value) in env_vars {
            cmd.arg("-e").arg(format!("{key}={value}"));
        }

        // Use the provided config content

        cmd.arg("--entrypoint")
            .arg("sh")
            .arg(&config.image)
            .arg("-c")
            .arg(format!(
                "echo '{}' > /tmp/config.yaml && litellm --config /tmp/config.yaml",
                config_content.replace("'", "'\"'\"'")
            ));

        let output = cmd.output().await.context(DockerCommandSnafu)?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        info!("Docker command stdout: {}", stdout);
        if !stderr.is_empty() {
            info!("Docker command stderr: {}", stderr);
        }

        if !output.status.success() {
            return Err(LiteLLMError::ContainerStartFailed {
                message: format!("Docker run failed: {stderr}"),
                location: location!(),
            });
        }

        let container_id = stdout.trim().to_string();
        info!("Container started with ID: {}", container_id);

        Ok(container_id)
    }

    pub async fn wait_for_health(&self) -> Result<(), LiteLLMError> {
        let base_url = self.config.get_base_url();
        let client = reqwest::Client::new();
        let health_url = format!("{base_url}/health");
        let max_attempts = 30;
        let delay = Duration::from_secs(1);

        info!("Waiting for LiteLLM to become healthy at {}...", health_url);

        for attempt in 1..=max_attempts {
            match timeout(Duration::from_secs(15), client.get(&health_url).send()).await {
                Ok(Ok(response)) if response.status().is_success() => {
                    // Try to parse the health response
                    match response.text().await {
                        Ok(body) => {
                            debug!("Health check response: {}", body);

                            // Try to deserialize the health response
                            match serde_json::from_str::<LiteLLMHealthResponse>(&body) {
                                Ok(health_response) => {
                                    // Check if there are unhealthy endpoints
                                    if health_response.unhealthy_count > 0 {
                                        return Err(LiteLLMError::UnhealthyEndpoints {
                                            unhealthy_endpoints: health_response
                                                .unhealthy_endpoints,
                                            location: location!(),
                                        });
                                    }

                                    info!(
                                        "LiteLLM health check passed on attempt {} - {} healthy endpoints",
                                        attempt, health_response.healthy_count
                                    );
                                    return Ok(());
                                }
                                Err(e) => {
                                    debug!("Failed to parse health response as JSON: {}", e);
                                    // Fall back to simple status check for backward compatibility
                                    info!(
                                        "LiteLLM health check passed on attempt {} (simple status check)",
                                        attempt
                                    );
                                    return Ok(());
                                }
                            }
                        }
                        Err(e) => {
                            debug!("Failed to read health response body: {}", e);
                            // Fall back to simple status check
                            info!(
                                "LiteLLM health check passed on attempt {} (simple status check)",
                                attempt
                            );
                            return Ok(());
                        }
                    }
                }
                Ok(Ok(response)) => {
                    debug!(
                        "Health check attempt {} failed with status: {}",
                        attempt,
                        response.status()
                    );
                }
                Ok(Err(e)) => {
                    debug!("Health check attempt {} failed with error: {}", attempt, e);
                }
                Err(_) => {
                    debug!("Health check attempt {} timed out", attempt);
                }
            }

            if attempt < max_attempts {
                sleep(delay).await;
            }
        }

        Err(LiteLLMError::HealthCheckFailed {
            attempts: max_attempts,
            location: location!(),
        })
    }

    pub fn get_base_url(&self) -> String {
        self.config.get_base_url()
    }

    async fn stream_container_logs(
        container_name: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use tokio::io::{AsyncBufReadExt, BufReader};

        debug!("Starting log stream for container: {}", container_name);

        // Start following logs from the container
        let mut child = Command::new("docker")
            .args(["logs", "-f", "--tail", "20", container_name])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().expect("Failed to capture stdout");
        let stderr = child.stderr.take().expect("Failed to capture stderr");

        // Spawn task to handle stdout
        let container_name_stdout = container_name.to_string();
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                info!("[{}] {}", container_name_stdout, line);
            }
        });

        // Spawn task to handle stderr
        let container_name_stderr = container_name.to_string();
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::error!("[{}] {}", container_name_stderr, line);
            }
        });

        // Wait for the docker logs process (it will run until container stops)
        let _ = child.wait().await;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ModelDefinition;
    use serde_json::json;
    use std::collections::HashMap;

    fn create_test_litellm_config() -> LiteLLMConfig {
        let mut models = Vec::new();

        // Add gpt-4o model
        let mut gpt4_params = HashMap::new();
        gpt4_params.insert(
            "model".to_string(),
            toml::Value::String("azure/gpt-4o".to_string()),
        );
        gpt4_params.insert(
            "api_base".to_string(),
            toml::Value::String("os.environ/AZURE_API_BASE".to_string()),
        );
        gpt4_params.insert(
            "api_key".to_string(),
            toml::Value::String("os.environ/AZURE_API_KEY".to_string()),
        );

        models.push(ModelDefinition {
            model_name: "gpt-4o".to_string(),
            litellm_params: gpt4_params,
        });

        // Add gpt-3.5-turbo model
        let mut gpt35_params = HashMap::new();
        gpt35_params.insert(
            "model".to_string(),
            toml::Value::String("openai/gpt-3.5-turbo".to_string()),
        );
        gpt35_params.insert(
            "api_key".to_string(),
            toml::Value::String("os.environ/OPENAI_API_KEY".to_string()),
        );

        models.push(ModelDefinition {
            model_name: "gpt-3.5-turbo".to_string(),
            litellm_params: gpt35_params,
        });

        // Add claude model
        let mut claude_params = HashMap::new();
        claude_params.insert(
            "model".to_string(),
            toml::Value::String("anthropic/claude-3-sonnet".to_string()),
        );
        claude_params.insert(
            "api_key".to_string(),
            toml::Value::String("os.environ/ANTHROPIC_API_KEY".to_string()),
        );

        models.push(ModelDefinition {
            model_name: "claude-3-sonnet".to_string(),
            litellm_params: claude_params,
        });

        LiteLLMConfig {
            image: "ghcr.io/berriai/litellm:main-latest".to_string(),
            port: 4000,
            container_name: "wasmind-litellm-test".to_string(),
            env_overrides: HashMap::new(),
            models,
        }
    }

    #[test]
    fn test_generate_config() {
        let litellm_config = create_test_litellm_config();

        let config_content =
            LiteLLMManager::generate_config(&litellm_config).expect("Should generate config");

        // Parse the generated YAML
        let config_file: LiteLLMConfigFile =
            serde_yaml::from_str(&config_content).expect("Should parse YAML");

        // Verify we have the expected models
        assert_eq!(config_file.model_list.len(), 3);

        // Find each model and verify their parameters
        let gpt4_model = config_file
            .model_list
            .iter()
            .find(|m| m.model_name == "gpt-4o")
            .expect("Should have gpt-4o model");

        assert_eq!(
            gpt4_model.litellm_params.get("model").unwrap(),
            &json!("azure/gpt-4o")
        );
        assert_eq!(
            gpt4_model.litellm_params.get("api_base").unwrap(),
            &json!("os.environ/AZURE_API_BASE")
        );

        let gpt35_model = config_file
            .model_list
            .iter()
            .find(|m| m.model_name == "gpt-3.5-turbo")
            .expect("Should have gpt-3.5-turbo model");

        assert_eq!(
            gpt35_model.litellm_params.get("model").unwrap(),
            &json!("openai/gpt-3.5-turbo")
        );

        let claude_model = config_file
            .model_list
            .iter()
            .find(|m| m.model_name == "claude-3-sonnet")
            .expect("Should have claude-3-sonnet model");

        assert_eq!(
            claude_model.litellm_params.get("model").unwrap(),
            &json!("anthropic/claude-3-sonnet")
        );
    }

    #[test]
    fn test_generate_config_empty_models() {
        let litellm_config = LiteLLMConfig {
            image: "ghcr.io/berriai/litellm:main-latest".to_string(),
            port: 4000,
            container_name: "wasmind-litellm-test".to_string(),
            env_overrides: HashMap::new(),
            models: Vec::new(),
        };

        let result = LiteLLMManager::generate_config(&litellm_config);

        // Should fail with no models configured
        assert!(result.is_err());
        match result.unwrap_err() {
            LiteLLMError::ConfigFileCreation { message, .. } => {
                assert_eq!(message, "No models configured");
            }
            _ => panic!("Expected ConfigFileCreation error"),
        }
    }

    #[test]
    fn test_litellm_config_file_serialization() {
        // Use a single model to avoid HashMap ordering issues in tests
        let mut params = HashMap::new();
        params.insert("model".to_string(), json!("azure/gpt-4"));
        params.insert("api_base".to_string(), json!("os.environ/AZURE_API_BASE"));
        params.insert("api_key".to_string(), json!("os.environ/AZURE_API_KEY"));
        params.insert("api_version".to_string(), json!("2025-01-01-preview"));

        let config_file = LiteLLMConfigFile {
            model_list: vec![LiteLLMModelEntry {
                model_name: "gpt-4".to_string(),
                litellm_params: params,
            }],
        };

        let yaml = serde_yaml::to_string(&config_file).expect("Should serialize to YAML");

        // Check for required structure components instead of exact string matching
        // to avoid HashMap ordering issues
        assert!(yaml.contains("model_list:"));
        assert!(yaml.contains("- model_name: gpt-4"));
        assert!(yaml.contains("  litellm_params:"));
        assert!(yaml.contains("    model: azure/gpt-4"));
        assert!(yaml.contains("    api_base: os.environ/AZURE_API_BASE"));
        assert!(yaml.contains("    api_key: os.environ/AZURE_API_KEY"));
        assert!(yaml.contains("    api_version: 2025-01-01-preview"));

        // Verify we can deserialize it back
        let parsed: LiteLLMConfigFile =
            serde_yaml::from_str(&yaml).expect("Should deserialize from YAML");

        assert_eq!(parsed.model_list.len(), 1);
        assert_eq!(parsed.model_list[0].model_name, "gpt-4");
        assert_eq!(
            parsed.model_list[0].litellm_params.get("model").unwrap(),
            &json!("azure/gpt-4")
        );
        assert_eq!(
            parsed.model_list[0].litellm_params.get("api_base").unwrap(),
            &json!("os.environ/AZURE_API_BASE")
        );
        assert_eq!(
            parsed.model_list[0].litellm_params.get("api_key").unwrap(),
            &json!("os.environ/AZURE_API_KEY")
        );
        assert_eq!(
            parsed.model_list[0]
                .litellm_params
                .get("api_version")
                .unwrap(),
            &json!("2025-01-01-preview")
        );
    }

    #[test]
    fn test_convert_toml_to_json() {
        // Test string conversion
        let toml_str = toml::Value::String("test".to_string());
        let json_str = LiteLLMManager::convert_toml_to_json(toml_str);
        assert_eq!(json_str, json!("test"));

        // Test integer conversion
        let toml_int = toml::Value::Integer(42);
        let json_int = LiteLLMManager::convert_toml_to_json(toml_int);
        assert_eq!(json_int, json!(42));

        // Test boolean conversion
        let toml_bool = toml::Value::Boolean(true);
        let json_bool = LiteLLMManager::convert_toml_to_json(toml_bool);
        assert_eq!(json_bool, json!(true));

        // Test table conversion
        let mut toml_table = toml::Table::new();
        toml_table.insert("key".to_string(), toml::Value::String("value".to_string()));
        let toml_table_val = toml::Value::Table(toml_table);
        let json_table = LiteLLMManager::convert_toml_to_json(toml_table_val);
        assert_eq!(json_table, json!({"key": "value"}));
    }
}
