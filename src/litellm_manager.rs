use serde::{Deserialize, Serialize};
use snafu::{Location, ResultExt, Snafu, location};
use std::collections::HashMap;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::{sleep, timeout};
use tracing::{debug, info, warn};

use crate::config::ParsedConfig;

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

#[derive(Debug, Clone)]
pub struct LiteLLMConfig {
    /// Docker image to use
    pub image: String,

    /// Port to expose LiteLLM on
    pub port: u16,

    /// Container name (for easy management)
    pub container_name: String,

    /// Whether to remove container on exit
    pub auto_remove: bool,

    /// Additional env var overrides (optional)
    pub env_overrides: HashMap<String, String>,
}

impl Default for LiteLLMConfig {
    fn default() -> Self {
        Self {
            image: "ghcr.io/berriai/litellm:main-latest".to_string(),
            port: 4000,
            container_name: "hive-litellm".to_string(),
            auto_remove: true,
            env_overrides: HashMap::new(),
        }
    }
}

pub struct LiteLLMManager {
    config: LiteLLMConfig,
    container_id: Option<String>,
}

impl LiteLLMManager {
    /// Generate LiteLLM config from parsed config
    pub fn generate_config(parsed_config: &ParsedConfig) -> Result<String, LiteLLMError> {
        let mut model_list = Vec::new();

        // Add main manager model
        if !parsed_config.hive.main_manager_model.model_name.is_empty() {
            model_list.push(LiteLLMModelEntry {
                model_name: parsed_config.hive.main_manager_model.model_name.clone(),
                litellm_params: parsed_config.hive.main_manager_model.litellm_params.clone(),
            });
        }

        // Add sub manager model (if different)
        if !parsed_config.hive.sub_manager_model.model_name.is_empty()
            && parsed_config.hive.sub_manager_model.model_name
                != parsed_config.hive.main_manager_model.model_name
        {
            model_list.push(LiteLLMModelEntry {
                model_name: parsed_config.hive.sub_manager_model.model_name.clone(),
                litellm_params: parsed_config.hive.sub_manager_model.litellm_params.clone(),
            });
        }

        // Add worker model (if different)
        if !parsed_config.hive.worker_model.model_name.is_empty()
            && parsed_config.hive.worker_model.model_name
                != parsed_config.hive.main_manager_model.model_name
            && parsed_config.hive.worker_model.model_name
                != parsed_config.hive.sub_manager_model.model_name
        {
            model_list.push(LiteLLMModelEntry {
                model_name: parsed_config.hive.worker_model.model_name.clone(),
                litellm_params: parsed_config.hive.worker_model.litellm_params.clone(),
            });
        }

        // Add temporal check_health model (if exists and different)
        if let Some(check_health_model) = &parsed_config.hive.temporal.check_health {
            if !check_health_model.model_name.is_empty()
                && !model_list
                    .iter()
                    .any(|m| m.model_name == check_health_model.model_name)
            {
                model_list.push(LiteLLMModelEntry {
                    model_name: check_health_model.model_name.clone(),
                    litellm_params: check_health_model.litellm_params.clone(),
                });
            }
        }

        if model_list.is_empty() {
            return Err(LiteLLMError::ConfigFileCreation {
                message: "No models configured".to_string(),
                location: location!(),
            });
        }

        let config_file = LiteLLMConfigFile { model_list };
        let yaml_content = serde_yaml::to_string(&config_file).context(ConfigSerializationSnafu)?;

        info!("Generated LiteLLM config");
        debug!("Config content: {}", yaml_content);

        Ok(yaml_content)
    }

    pub async fn start(
        config: &LiteLLMConfig,
        parsed_config: &ParsedConfig,
    ) -> Result<Self, LiteLLMError> {
        info!("Starting LiteLLM Docker container...");

        // Check if Docker is available
        Self::check_docker_available().await?;

        // Stop any existing container with our name
        Self::cleanup_existing_container(&config.container_name).await?;

        // Generate config content
        let config_content = Self::generate_config(parsed_config)?;

        // Collect environment variables
        let env_vars = Self::collect_env_vars(&config.env_overrides);

        // Start the container
        let container_id = Self::start_container(config, &config_content, env_vars).await?;

        // Start streaming container logs in background for debugging
        let container_name_for_logs = config.container_name.clone();
        tokio::spawn(async move {
            if let Err(e) = Self::stream_container_logs(&container_name_for_logs).await {
                warn!("Container log streaming failed: {}", e);
            }
        });

        // Wait for health check
        Self::wait_for_health(&format!("http://localhost:{}", config.port)).await?;

        info!(
            "LiteLLM container started successfully at http://localhost:{}",
            config.port
        );

        Ok(Self {
            config: config.clone(),
            container_id: Some(container_id),
        })
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
            .arg("-d") // Detached mode - runs container in background and returns immediately
            .arg("--name")
            .arg(&config.container_name)
            .arg("-p")
            .arg(format!("{}:4000", config.port));

        // Add environment variables
        for (key, value) in env_vars {
            cmd.arg("-e").arg(format!("{}={}", key, value));
        }

        // Use the provided config content

        cmd.arg("--entrypoint")
            .arg("sh")
            .arg(&config.image)
            .arg("-c")
            .arg(format!("echo '{}' > /tmp/config.yaml && litellm --config /tmp/config.yaml --detailed_debug", 
                config_content.replace("'", "'\"'\"'")));

        info!("Running docker command: {:?}", cmd);

        let output = cmd.output().await.context(DockerCommandSnafu)?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        info!("Docker command stdout: {}", stdout);
        if !stderr.is_empty() {
            info!("Docker command stderr: {}", stderr);
        }

        if !output.status.success() {
            return Err(LiteLLMError::ContainerStartFailed {
                message: format!("Docker run failed: {}", stderr),
                location: location!(),
            });
        }

        let container_id = stdout.trim().to_string();
        info!("Container started with ID: {}", container_id);

        Ok(container_id)
    }

    async fn wait_for_health(base_url: &str) -> Result<(), LiteLLMError> {
        let client = reqwest::Client::new();
        let health_url = format!("{}/health", base_url);
        let max_attempts = 30;
        let delay = Duration::from_secs(1);

        info!("Waiting for LiteLLM to become healthy at {}...", health_url);

        for attempt in 1..=max_attempts {
            match timeout(Duration::from_secs(5), client.get(&health_url).send()).await {
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
        format!("http://localhost:{}", self.config.port)
    }

    pub async fn stop(&mut self) -> Result<(), LiteLLMError> {
        if let Some(container_id) = &self.container_id {
            info!("Stopping LiteLLM container: {}", container_id);

            let output = Command::new("docker")
                .args(["stop", container_id])
                .output()
                .await
                .context(DockerCommandSnafu)?;

            if !output.status.success() {
                warn!(
                    "Failed to stop container: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }

            if self.config.auto_remove {
                let output = Command::new("docker")
                    .args(["rm", container_id])
                    .output()
                    .await
                    .context(DockerCommandSnafu)?;

                if !output.status.success() {
                    warn!(
                        "Failed to remove container: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }

            self.container_id = None;
            info!("LiteLLM container stopped");
        }

        Ok(())
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

impl Drop for LiteLLMManager {
    fn drop(&mut self) {
        if let Some(container_id) = &self.container_id {
            // Try to stop the container in a blocking way
            // This is not ideal but Drop doesn't support async
            let _ = std::process::Command::new("docker")
                .args(["stop", container_id])
                .output();

            if self.config.auto_remove {
                let _ = std::process::Command::new("docker")
                    .args(["rm", container_id])
                    .output();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{ParsedConfig, ParsedHiveConfig, ParsedModelConfig, ParsedTemporalConfig},
        llm_client::Tool,
    };
    use serde_json::json;
    use std::collections::HashMap;

    fn create_test_model_config(
        model_name: &str,
        litellm_params: HashMap<String, serde_json::Value>,
    ) -> ParsedModelConfig {
        ParsedModelConfig {
            model_name: model_name.to_string(),
            system_prompt: "You are a helpful assistant.".to_string(),
            litellm_params,
            base_url: "http://localhost:4000".to_string(),
        }
    }

    fn create_test_parsed_config() -> ParsedConfig {
        let mut main_manager_params = HashMap::new();
        main_manager_params.insert("model".to_string(), json!("azure/gpt-4o"));
        main_manager_params.insert("api_base".to_string(), json!("os.environ/AZURE_API_BASE"));
        main_manager_params.insert("api_key".to_string(), json!("os.environ/AZURE_API_KEY"));

        let mut worker_params = HashMap::new();
        worker_params.insert("model".to_string(), json!("openai/gpt-3.5-turbo"));
        worker_params.insert("api_key".to_string(), json!("os.environ/OPENAI_API_KEY"));

        let mut temporal_params = HashMap::new();
        temporal_params.insert("model".to_string(), json!("anthropic/claude-3-sonnet"));
        temporal_params.insert("api_key".to_string(), json!("os.environ/ANTHROPIC_API_KEY"));

        ParsedConfig {
            keys: crate::config::ParsedKeyConfig {
                bindings: HashMap::new(),
            },
            mcp_servers: HashMap::new(),
            whitelisted_commands: vec![],
            auto_approve_commands: false,
            hive: ParsedHiveConfig {
                main_manager_model: create_test_model_config("gpt-4o", main_manager_params),
                sub_manager_model: create_test_model_config("gpt-4o", HashMap::new()), // Same as main manager
                worker_model: create_test_model_config("gpt-3.5-turbo", worker_params),
                temporal: ParsedTemporalConfig {
                    check_health: Some(create_test_model_config(
                        "claude-3-sonnet",
                        temporal_params,
                    )),
                },
                litellm: crate::config::LiteLLMConfig::default(),
            },
        }
    }

    #[test]
    fn test_generate_config() {
        let parsed_config = create_test_parsed_config();

        let config_content =
            LiteLLMManager::generate_config(&parsed_config).expect("Should generate config");

        // Parse the generated YAML
        let config_file: LiteLLMConfigFile =
            serde_yaml::from_str(&config_content).expect("Should parse YAML");

        // Verify we have the expected models (should deduplicate)
        assert_eq!(config_file.model_list.len(), 3); // gpt-4o, gpt-3.5-turbo, claude-3-sonnet

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
    fn test_generate_config_deduplication() {
        let mut parsed_config = create_test_parsed_config();

        // Make all models the same
        let same_params = {
            let mut params = HashMap::new();
            params.insert("model".to_string(), json!("openai/gpt-4"));
            params.insert("api_key".to_string(), json!("os.environ/OPENAI_API_KEY"));
            params
        };

        let same_model = create_test_model_config("gpt-4", same_params);

        parsed_config.hive.main_manager_model = same_model.clone();
        parsed_config.hive.sub_manager_model = same_model.clone();
        parsed_config.hive.worker_model = same_model.clone();
        parsed_config.hive.temporal.check_health = Some(same_model);

        let config_content =
            LiteLLMManager::generate_config(&parsed_config).expect("Should generate config");

        let config_file: LiteLLMConfigFile =
            serde_yaml::from_str(&config_content).expect("Should parse YAML");

        // Should only have one model due to deduplication
        assert_eq!(config_file.model_list.len(), 1);
        assert_eq!(config_file.model_list[0].model_name, "gpt-4");
    }

    #[test]
    fn test_generate_config_empty_models() {
        let mut parsed_config = create_test_parsed_config();

        // Set all model names to empty
        parsed_config.hive.main_manager_model.model_name = "".to_string();
        parsed_config.hive.sub_manager_model.model_name = "".to_string();
        parsed_config.hive.worker_model.model_name = "".to_string();
        parsed_config.hive.temporal.check_health = None;

        let result = LiteLLMManager::generate_config(&parsed_config);

        // Should fail with no models configured
        assert!(result.is_err());
        match result.unwrap_err() {
            LiteLLMError::ConfigFileCreation { message, .. } => {
                assert_eq!(message, "No models configured");
            }
            _ => panic!("Expected ConfigFileCreation error"),
        }
    }

    #[tokio::test]
    #[ignore] // Skip by default - requires Docker and API keys
    async fn test_integration_docker_container_with_llm_client() {
        use crate::llm_client::{ChatMessage, LLMClient};
        use std::env;

        // Initialize tracing for this test to see debug output
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::from_default_env()
                    .add_directive("hive=info".parse().unwrap()),
            )
            .try_init();

        // Skip if no API key is available
        if env::var("OPENAI_API_KEY").is_err() {
            println!("❌ Skipping integration test - OPENAI_API_KEY not set");
            return;
        }

        let parsed_config = create_test_parsed_config_with_openai();

        // Create LiteLLM config
        let litellm_config = LiteLLMConfig {
            port: 4001, // Use different port to avoid conflicts
            image: "ghcr.io/berriai/litellm:main-latest".to_string(),
            container_name: "hive-litellm-integration-test".to_string(),
            auto_remove: true,
            env_overrides: HashMap::new(),
        };

        info!(
            "Starting LiteLLM Docker container on port {}",
            litellm_config.port
        );

        // Start the LiteLLM container (runs in detached mode with -d flag)
        let mut litellm_manager = LiteLLMManager::start(&litellm_config, &parsed_config)
            .await
            .expect("Should start LiteLLM container");

        // Start streaming container logs in background
        let container_name = litellm_config.container_name.clone();
        tokio::spawn(async move {
            if let Err(e) = LiteLLMManager::stream_container_logs(&container_name).await {
                warn!("Container log streaming failed: {}", e);
            }
        });

        // Create LLM client pointing to our container
        let base_url = format!("http://localhost:{}", litellm_config.port);
        let client = LLMClient::new(base_url.clone());

        info!("Making API request to LiteLLM at {}", base_url);

        // Make a simple chat request
        let messages = vec![ChatMessage::user(
            "What's the weather like in San Francisco",
        )];

        let tools = vec![Tool {
            tool_type: "function".to_string(),
            function: crate::llm_client::ToolFunction {
                name: "get_current_weather".to_string(),
                description: "Get the current weather in a given location".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "location": {
                            "type": "string",
                            "description": "The city and state, e.g. San Francisco, CA",
                        },
                        "unit": {"type": "string", "enum": ["celsius", "fahrenheit"]},
                    },
                    "required": ["location"],
                }),
            },
        }];

        let response = client
            .chat(
                "gpt-4o",
                "Use your tools to answer the users questions",
                messages,
                Some(tools),
            )
            .await
            .expect("Should get response from LiteLLM");

        // Verify we got a response
        assert!(
            !response.choices.is_empty(),
            "Should have at least one choice"
        );

        let message = &response.choices[0].message;
        match message {
            ChatMessage::Assistant {
                content: Some(content),
                tool_calls: Some(tool_calls),
                ..
            } => {
                assert!(!content.is_empty(), "Response content should not be empty");
                assert!(
                    !tool_calls.is_empty(),
                    "Tool calls content should not be empty"
                );
            }
            _ => panic!("Expected assistant message with content"),
        }

        // Clean up
        info!("Cleaning up Docker container");
        litellm_manager.stop().await.expect("Should stop container");

        info!("Integration test completed successfully");
    }

    fn create_test_parsed_config_with_openai() -> ParsedConfig {
        let mut openai_params = HashMap::new();
        openai_params.insert("model".to_string(), json!("openai/gpt-4o"));
        openai_params.insert("api_key".to_string(), json!("os.environ/OPENAI_API_KEY"));

        ParsedConfig {
            keys: crate::config::ParsedKeyConfig {
                bindings: HashMap::new(),
            },
            mcp_servers: HashMap::new(),
            whitelisted_commands: vec![],
            auto_approve_commands: false,
            hive: ParsedHiveConfig {
                main_manager_model: ParsedModelConfig {
                    model_name: "gpt-4o".to_string(),
                    system_prompt: "You are a helpful assistant.".to_string(),
                    litellm_params: openai_params.clone(),
                    base_url: "http://localhost:4001".to_string(),
                },
                sub_manager_model: ParsedModelConfig {
                    model_name: "gpt-4o".to_string(),
                    system_prompt: "You are a helpful assistant.".to_string(),
                    litellm_params: openai_params.clone(),
                    base_url: "http://localhost:4001".to_string(),
                },
                worker_model: ParsedModelConfig {
                    model_name: "gpt-4o".to_string(),
                    system_prompt: "You are a helpful assistant.".to_string(),
                    litellm_params: openai_params.clone(),
                    base_url: "http://localhost:4001".to_string(),
                },
                temporal: ParsedTemporalConfig { check_health: None },
                litellm: crate::config::LiteLLMConfig {
                    port: 4001,
                    image: "ghcr.io/berriai/litellm:main-latest".to_string(),
                    container_name: "hive-litellm-integration-test".to_string(),
                    auto_remove: true,
                    env_overrides: HashMap::new(),
                },
            },
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
}
