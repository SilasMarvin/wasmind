use etcetera::{AppStrategy, AppStrategyArgs, choose_app_strategy};
use serde::{Deserialize, Serialize};
use serde_json;
use snafu::{ResultExt, Snafu};
use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
};
use toml::Table;
use tuirealm::event::KeyEvent;

use crate::{
    actors::tui::components::{
        chat::ChatUserAction, dashboard::DashboardUserAction, graph::GraphUserAction,
    },
    utils::parse_key_combination,
};

/// Errors while getting the config
#[derive(Debug, Snafu)]
pub enum ConfigError {
    #[snafu(display("Invalid binding '{binding}' - are you sure this is a valid binding?"))]
    InvalidBinding { binding: String },

    #[snafu(display(
        "Invalid action: '{action}' for binding '{binding}' - are you sure this is a valid action?"
    ))]
    InvalidActionForBinding { action: String, binding: String },

    #[snafu(transparent)]
    IO { source: io::Error },

    #[snafu(display("No model config / invalid model config specified for: {model_for}"))]
    MissingModel { model_for: String },

    #[snafu(display("Error deserializing config. Double check all fields are valid"))]
    TomlDeserialize {
        #[snafu(source)]
        source: toml::de::Error,
    },
}

fn get_config_file_path() -> PathBuf {
    // Create an instance of Etcetera for your application "hive".
    // The etcetera crate will determine the correct base config directory depending on the OS.
    let strategy = choose_app_strategy(AppStrategyArgs {
        top_level_domain: "com".to_string(),
        author: "hive".to_string(),
        app_name: "hive".to_string(),
    })
    .unwrap();

    // This returns the complete path to the config file "config.toml".
    // On Linux/macOS, this will be: $HOME/.config/hive/config.toml
    // On Windows, this will typically be: %APPDATA%\hive\config.toml
    strategy.config_dir().join("config.toml")
}

#[derive(Deserialize, Default, Debug)]
pub struct DashboardConfig {
    #[serde(default)]
    clear_defaults: bool,
    #[serde(default)]
    key_bindings: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ParsedDashboardConfig {
    pub key_bindings: HashMap<KeyEvent, DashboardUserAction>,
}

#[derive(Deserialize, Default, Debug)]
pub struct GraphConfig {
    #[serde(default)]
    clear_defaults: bool,
    #[serde(default)]
    key_bindings: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ParsedGraphConfig {
    pub key_bindings: HashMap<KeyEvent, GraphUserAction>,
}

#[derive(Deserialize, Default, Debug)]
pub struct ChatConfig {
    #[serde(default)]
    clear_defaults: bool,
    #[serde(default)]
    key_bindings: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ParsedChatConfig {
    pub key_bindings: HashMap<KeyEvent, ChatUserAction>,
}

/// The config we deserialize directly from toml
#[derive(Deserialize)]
pub struct Config {
    #[serde(default)]
    tui: TuiConfig,
    #[serde(default)]
    mcp_servers: HashMap<String, McpServerConfig>,
    #[serde(default)]
    whitelisted_commands: Option<Vec<String>>,
    #[serde(default)]
    pub auto_approve_commands: bool,
    #[serde(default)]
    pub hive: HiveConfig,
}

impl Config {
    pub fn new(is_headless: bool) -> Result<Self, ConfigError> {
        // Check for environment variable first
        let config_file_path = if let Ok(env_path) = std::env::var("HIVE_CONFIG_PATH") {
            tracing::debug!(
                "Using config path from HIVE_CONFIG_PATH env var: {}",
                env_path
            );
            PathBuf::from(env_path)
        } else {
            get_config_file_path()
        };

        Config::from_file(config_file_path, is_headless)
    }

    pub fn from_file<P: AsRef<Path>>(path: P, is_headless: bool) -> Result<Self, ConfigError> {
        // tracing::debug!("Looking for config file at: {:?}", path);
        let user_config: Option<Config> = if fs::exists(&path)? {
            tracing::debug!("Found config file");
            let contents = fs::read_to_string(&path)?;
            Some(toml::from_str(&contents).context(TomlDeserializeSnafu)?)
        } else {
            tracing::debug!("No config file found, using defaults");
            None
        };

        let mut default = Config::load_default(is_headless)?;

        if let Some(mut user_config) = user_config {
            // Combine hive pieces

            if !user_config.tui.dashboard.clear_defaults {
                default
                    .tui
                    .dashboard
                    .key_bindings
                    .extend(user_config.tui.dashboard.key_bindings.into_iter());
                user_config.tui.dashboard.key_bindings = default.tui.dashboard.key_bindings;
            }

            if !user_config.tui.graph.clear_defaults {
                default
                    .tui
                    .graph
                    .key_bindings
                    .extend(user_config.tui.graph.key_bindings.into_iter());
                user_config.tui.graph.key_bindings = default.tui.graph.key_bindings;
            }

            if !user_config.tui.chat.clear_defaults {
                default
                    .tui
                    .chat
                    .key_bindings
                    .extend(user_config.tui.chat.key_bindings.into_iter());
                user_config.tui.chat.key_bindings = default.tui.chat.key_bindings;
            }

            Ok(user_config)
        } else {
            Ok(default)
        }
    }

    pub fn load_default(is_headless: bool) -> Result<Self, ConfigError> {
        let default_contents = if is_headless {
            include_str!("../../../headless_config.toml")
        } else {
            include_str!("../../../default_config.toml")
        };

        let config: Config = toml::from_str(default_contents).context(TomlDeserializeSnafu)?;
        Ok(config)
    }
}

/// The key bindings we deserialize directly from toml
#[derive(Deserialize, Default, Debug)]
struct TuiConfig {
    #[serde(default)]
    dashboard: DashboardConfig,
    #[serde(default)]
    chat: ChatConfig,
    #[serde(default)]
    graph: GraphConfig,
}

/// The model configuration we deserialize directly from toml
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct ModelConfig {
    pub model_name: Option<String>,
    pub system_prompt: Option<String>,
    pub litellm_params: Option<Table>,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct DefaultModelConfig {
    pub model_name: Option<String>,
    pub litellm_params: Option<Table>,
}

/// LiteLLM configuration
#[derive(Deserialize, Debug, Clone)]
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

impl Default for LiteLLMConfig {
    fn default() -> Self {
        Self {
            image: default_litellm_image(),
            port: default_litellm_port(),
            container_name: default_container_name(),
            env_overrides: HashMap::new(),
        }
    }
}

/// Temporal worker configuration
#[derive(Deserialize, Default, Debug, Clone)]
pub struct TemporalConfig {
    /// Model configuration for check_health temporal worker
    #[serde(default)]
    pub check_health: ModelConfig,
}

/// HIVE multi-agent configuration
#[derive(Deserialize, Default, Debug, Clone)]
pub struct HiveConfig {
    /// Default model configuration to use as fallback
    #[serde(default)]
    pub default_model: Option<DefaultModelConfig>,
    /// Model configuration for the main manager agent
    #[serde(default)]
    pub main_manager_model: ModelConfig,
    /// Model configuration for sub-manager agents
    #[serde(default)]
    pub sub_manager_model: ModelConfig,
    /// Model configuration for worker agents
    #[serde(default)]
    pub worker_model: ModelConfig,
    /// Temporal worker configurations
    #[serde(default)]
    pub temporal: TemporalConfig,
    /// LiteLLM configuration
    #[serde(default)]
    pub litellm: LiteLLMConfig,
}

/// An MCP Config
#[derive(Deserialize, Default, Debug, Clone)]
pub struct McpServerConfig {
    pub command: String,
    pub args: Vec<String>,
}

impl TryFrom<Config> for ParsedConfig {
    type Error = ConfigError;

    fn try_from(value: Config) -> Result<Self, Self::Error> {
        let tui = ParsedTuiConfig {
            dashboard: ParsedDashboardConfig {
                key_bindings: value
                    .tui
                    .dashboard
                    .key_bindings
                    .into_iter()
                    .map(|(binding, action)| {
                        let Some(parsed_binding) = parse_key_combination(&binding) else {
                            return Err(ConfigError::InvalidBinding { binding });
                        };

                        let Ok(action) = DashboardUserAction::try_from(action.as_str()) else {
                            return Err(ConfigError::InvalidActionForBinding { action, binding });
                        };

                        Ok((parsed_binding, action))
                    })
                    .collect::<Result<_, _>>()?,
            },
            chat: ParsedChatConfig {
                key_bindings: value
                    .tui
                    .chat
                    .key_bindings
                    .into_iter()
                    .map(|(binding, action)| {
                        let Some(parsed_binding) = parse_key_combination(&binding) else {
                            return Err(ConfigError::InvalidBinding { binding });
                        };

                        let Ok(action) = ChatUserAction::try_from(action.as_str()) else {
                            return Err(ConfigError::InvalidActionForBinding { action, binding });
                        };

                        Ok((parsed_binding, action))
                    })
                    .collect::<Result<_, _>>()?,
            },
            graph: ParsedGraphConfig {
                key_bindings: value
                    .tui
                    .graph
                    .key_bindings
                    .into_iter()
                    .map(|(binding, action)| {
                        let Some(parsed_binding) = parse_key_combination(&binding) else {
                            return Err(ConfigError::InvalidBinding { binding });
                        };

                        let Ok(action) = GraphUserAction::try_from(action.as_str()) else {
                            return Err(ConfigError::InvalidActionForBinding { action, binding });
                        };

                        Ok((parsed_binding, action))
                    })
                    .collect::<Result<_, _>>()?,
            },
        };

        let mcp_servers = value.mcp_servers;

        let whitelisted_commands = value.whitelisted_commands.unwrap_or(vec![]);
        let auto_approve_commands = value.auto_approve_commands;

        let base_url = format!("http://localhost:{}", value.hive.litellm.port);
        let hive = ParsedHiveConfig {
            main_manager_model: parse_model_config(
                value.hive.main_manager_model,
                &value.hive.default_model,
                &base_url,
            )
            .ok_or(ConfigError::MissingModel {
                model_for: "main_manager_model".to_string(),
            })?,
            sub_manager_model: parse_model_config(
                value.hive.sub_manager_model,
                &value.hive.default_model,
                &base_url,
            )
            .ok_or(ConfigError::MissingModel {
                model_for: "sub_manager_model".to_string(),
            })?,
            worker_model: parse_model_config(
                value.hive.worker_model,
                &value.hive.default_model,
                &base_url,
            )
            .ok_or(ConfigError::MissingModel {
                model_for: "worker_model".to_string(),
            })?,
            temporal: ParsedTemporalConfig {
                check_health: parse_model_config(
                    value.hive.temporal.check_health,
                    &value.hive.default_model,
                    &base_url,
                )
                .ok_or(ConfigError::MissingModel {
                    model_for: "temporal.check_health".to_string(),
                })?,
            },
            litellm: value.hive.litellm,
        };

        Ok(Self {
            tui,
            mcp_servers,
            whitelisted_commands,
            auto_approve_commands,
            hive,
        })
    }
}

/// The parsed and verified config
#[derive(Debug, Clone)]
pub struct ParsedConfig {
    pub tui: ParsedTuiConfig,
    pub mcp_servers: HashMap<String, McpServerConfig>,
    pub whitelisted_commands: Vec<String>,
    pub auto_approve_commands: bool,
    pub hive: ParsedHiveConfig,
}

/// The parsed and verified tui config
#[derive(Debug, Clone)]
pub struct ParsedTuiConfig {
    pub dashboard: ParsedDashboardConfig,
    pub chat: ParsedChatConfig,
    pub graph: ParsedGraphConfig,
}

/// The parsed and verified model config
#[derive(Debug, Clone)]
pub struct ParsedModelConfig {
    pub model_name: String,
    pub system_prompt: String,
    pub litellm_params: HashMap<String, serde_json::Value>,
    pub base_url: String,
}

/// The parsed and verified temporal config
#[derive(Debug, Clone)]
pub struct ParsedTemporalConfig {
    pub check_health: ParsedModelConfig,
}

/// The parsed and verified HIVE config
#[derive(Debug, Clone)]
pub struct ParsedHiveConfig {
    pub main_manager_model: ParsedModelConfig,
    pub sub_manager_model: ParsedModelConfig,
    pub worker_model: ParsedModelConfig,
    pub temporal: ParsedTemporalConfig,
    pub litellm: LiteLLMConfig,
}

fn convert_toml_to_json(toml: toml::Value) -> serde_json::Value {
    match toml {
        toml::Value::String(s) => serde_json::Value::String(s),
        toml::Value::Integer(i) => serde_json::Value::Number(i.into()),
        toml::Value::Float(f) => {
            let n = serde_json::Number::from_f64(f).expect("float infinite and nan not allowed");
            serde_json::Value::Number(n)
        }
        toml::Value::Boolean(b) => serde_json::Value::Bool(b),
        toml::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(convert_toml_to_json).collect())
        }
        toml::Value::Table(table) => serde_json::Value::Object(
            table
                .into_iter()
                .map(|(k, v)| (k, convert_toml_to_json(v)))
                .collect(),
        ),
        toml::Value::Datetime(dt) => serde_json::Value::String(dt.to_string()),
    }
}

fn parse_model_config(
    model_config: ModelConfig,
    default_model: &Option<DefaultModelConfig>,
    base_url: &str,
) -> Option<ParsedModelConfig> {
    if let Some(model_name) = model_config.model_name.or(default_model
        .as_ref()
        .map(|dm| dm.model_name.clone())
        .flatten())
        && let Some(system_prompt) = model_config.system_prompt
        && let Some(litellm_params) = model_config.litellm_params.or(default_model
            .as_ref()
            .map(|dm| dm.litellm_params.clone())
            .flatten())
    {
        Some(ParsedModelConfig {
            model_name,
            system_prompt: system_prompt,
            litellm_params: litellm_params
                .into_iter()
                .map(|(key, value)| (key, convert_toml_to_json(value)))
                .collect(),
            base_url: base_url.to_string(),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_deserialize() {}
}
