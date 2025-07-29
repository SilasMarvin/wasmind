use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use snafu::{ResultExt, Snafu};

#[derive(Debug, Snafu)]
pub enum ConfigError {
    #[snafu(display("Error deserializing config. Double check all fields are valid"))]
    TomlDeserialize {
        #[snafu(source)]
        source: toml::de::Error,
    },
    
    #[snafu(display("Missing model '{model_name}' in litellm.models configuration"))]
    MissingModel {
        model_name: String,
    },
    
    #[snafu(display("Config parsing error: {}", source))]
    ConfigParse {
        #[snafu(source)]
        source: hive_config::Error,
    },
    
    #[snafu(display("LiteLLM configuration section is required"))]
    MissingLiteLLMConfig,
}

/// LiteLLM configuration
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LiteLLMConfig {
    /// Docker image to use
    #[serde(default = "default_litellm_image")]
    pub image: String,

    /// Port to expose LiteLLM on
    #[serde(default = "default_litellm_port")]
    pub port: u16,

    /// Container name (for easy management)
    #[serde(default = "default_container_name")]
    pub container_name: String,

    /// Additional env var overrides
    #[serde(default)]
    pub env_overrides: HashMap<String, String>,
    
    /// Model definitions (required)
    pub models: Vec<ModelDefinition>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ModelDefinition {
    /// Model name/identifier (required)
    pub model_name: String,
    
    /// LiteLLM parameters (API keys, base URLs, etc.)
    #[serde(default)]
    pub litellm_params: HashMap<String, toml::Value>,
}

fn default_litellm_image() -> String {
    "ghcr.io/berriai/litellm:main-latest".to_string()
}

fn default_litellm_port() -> u16 {
    4000
}

fn default_container_name() -> String {
    "hive-litellm".to_string()
}



impl LiteLLMConfig {
    pub fn from_config(config: &hive_config::Config) -> Result<Self, ConfigError> {
        config.parse_section::<LiteLLMConfig>("litellm")
            .context(ConfigParseSnafu)?
            .ok_or(ConfigError::MissingLiteLLMConfig)
    }
    
    pub fn get_base_url(&self) -> String {
        format!("http://localhost:{}", self.port)
    }
    
    pub fn get_model_definition(&self, model_name: &str) -> Result<&ModelDefinition, ConfigError> {
        self.models.iter()
            .find(|model| model.model_name == model_name)
            .ok_or_else(|| ConfigError::MissingModel { 
                model_name: model_name.to_string() 
            })
    }
    
    pub fn list_model_names(&self) -> Vec<&String> {
        self.models.iter().map(|model| &model.model_name).collect()
    }
}