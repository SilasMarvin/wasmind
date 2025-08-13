use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};
use std::collections::HashMap;
use tuirealm::event::KeyEvent;

use crate::tui::components::{
    chat::ChatUserAction, dashboard::DashboardUserAction, graph::GraphUserAction,
};

#[derive(Debug, Snafu)]
pub enum ConfigError {
    #[snafu(display("Error deserializing config. Double check all fields are valid"))]
    TomlDeserialize {
        #[snafu(source)]
        source: toml::de::Error,
    },

    #[snafu(display("Missing model '{model_name}' in litellm.models configuration"))]
    MissingModel { model_name: String },

    #[snafu(display("Config parsing error: {}", source))]
    ConfigParse {
        #[snafu(source)]
        source: wasmind::wasmind_config::Error,
    },

    #[snafu(display("LiteLLM configuration section is required"))]
    MissingLiteLLMConfig,

    #[snafu(display("Invalid key binding '{binding}' - are you sure this is a valid binding?"))]
    InvalidBinding { binding: String },
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
    "wasmind-litellm".to_string()
}

fn default_chat_key_bindings() -> HashMap<String, String> {
    use crate::tui::components::chat::ChatUserAction;

    let mut bindings = HashMap::new();
    bindings.insert(
        "ctrl-a".to_string(),
        ChatUserAction::Assist.as_str().to_string(),
    );
    bindings.insert(
        "ctrl-t".to_string(),
        ChatUserAction::ToggleToolExpansion.as_str().to_string(),
    );
    bindings
}

fn default_dashboard_key_bindings() -> HashMap<String, String> {
    use crate::tui::components::dashboard::DashboardUserAction;

    let mut bindings = HashMap::new();
    bindings.insert(
        "ctrl-c".to_string(),
        DashboardUserAction::Exit.as_str().to_string(),
    );
    bindings.insert(
        "esc".to_string(),
        DashboardUserAction::InterruptAgent.as_str().to_string(),
    );
    bindings
}

fn default_graph_key_bindings() -> HashMap<String, String> {
    use crate::tui::components::graph::GraphUserAction;

    let mut bindings = HashMap::new();
    bindings.insert(
        "shift-up".to_string(),
        GraphUserAction::SelectUp.as_str().to_string(),
    );
    bindings.insert(
        "shift-down".to_string(),
        GraphUserAction::SelectDown.as_str().to_string(),
    );
    bindings
}

impl LiteLLMConfig {
    pub fn from_config(config: &wasmind::wasmind_config::Config) -> Result<Self, ConfigError> {
        config
            .parse_section::<LiteLLMConfig>("litellm")
            .context(ConfigParseSnafu)?
            .ok_or(ConfigError::MissingLiteLLMConfig)
    }

    pub fn get_base_url(&self) -> String {
        format!("http://localhost:{}", self.port)
    }

    pub fn get_model_definition(&self, model_name: &str) -> Result<&ModelDefinition, ConfigError> {
        self.models
            .iter()
            .find(|model| model.model_name == model_name)
            .ok_or_else(|| ConfigError::MissingModel {
                model_name: model_name.to_string(),
            })
    }

    pub fn list_model_names(&self) -> Vec<&String> {
        self.models.iter().map(|model| &model.model_name).collect()
    }
}

/// TUI configuration section
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct TuiConfig {
    /// Dashboard configuration
    #[serde(default)]
    pub dashboard: DashboardConfig,

    /// Chat configuration
    #[serde(default)]
    pub chat: ChatConfig,

    /// Graph configuration
    #[serde(default)]
    pub graph: GraphConfig,
}

/// Dashboard configuration
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DashboardConfig {
    /// Clear default key bindings
    #[serde(default)]
    pub clear_defaults: bool,

    /// Custom key bindings
    #[serde(default = "default_dashboard_key_bindings")]
    pub key_bindings: HashMap<String, String>,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        DashboardConfig {
            clear_defaults: false,
            key_bindings: default_dashboard_key_bindings(),
        }
    }
}

/// Chat configuration
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChatConfig {
    /// Clear default key bindings
    #[serde(default)]
    pub clear_defaults: bool,

    /// Custom key bindings
    #[serde(default = "default_chat_key_bindings")]
    pub key_bindings: HashMap<String, String>,
}

impl Default for ChatConfig {
    fn default() -> Self {
        ChatConfig {
            clear_defaults: false,
            key_bindings: default_chat_key_bindings(),
        }
    }
}

/// Graph configuration
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GraphConfig {
    /// Clear default key bindings
    #[serde(default)]
    pub clear_defaults: bool,

    /// Custom key bindings
    #[serde(default = "default_graph_key_bindings")]
    pub key_bindings: HashMap<String, String>,
}

impl Default for GraphConfig {
    fn default() -> Self {
        GraphConfig {
            clear_defaults: false,
            key_bindings: default_graph_key_bindings(),
        }
    }
}

/// Parsed TUI configuration
#[derive(Debug, Clone)]
pub struct ParsedTuiConfig {
    pub dashboard: ParsedDashboardConfig,
    pub chat: ParsedChatConfig,
    pub graph: ParsedGraphConfig,
}

/// Parsed dashboard configuration
#[derive(Debug, Clone)]
pub struct ParsedDashboardConfig {
    pub key_bindings: HashMap<KeyEvent, DashboardUserAction>,
}

/// Parsed chat configuration
#[derive(Debug, Clone)]
pub struct ParsedChatConfig {
    pub key_bindings: HashMap<KeyEvent, ChatUserAction>,
}

/// Parsed graph configuration
#[derive(Debug, Clone)]
pub struct ParsedGraphConfig {
    pub key_bindings: HashMap<KeyEvent, GraphUserAction>,
}

impl TuiConfig {
    pub fn from_config(config: &wasmind::wasmind_config::Config) -> Result<Self, ConfigError> {
        Ok(config
            .parse_section::<TuiConfig>("tui")
            .context(ConfigParseSnafu)?
            .unwrap_or_default())
    }

    pub fn parse(self) -> Result<ParsedTuiConfig, ConfigError> {
        use crate::utils::parse_key_combination;

        // Merge dashboard defaults with user config
        let mut dashboard_bindings = if self.dashboard.clear_defaults {
            HashMap::new()
        } else {
            default_dashboard_key_bindings()
        };
        dashboard_bindings.extend(self.dashboard.key_bindings);

        let dashboard_key_bindings = dashboard_bindings
            .into_iter()
            .map(|(binding, action)| {
                let Some(parsed_binding) = parse_key_combination(&binding) else {
                    return Err(ConfigError::InvalidBinding { binding });
                };
                let Ok(action) = DashboardUserAction::try_from(action.as_str()) else {
                    return Err(ConfigError::InvalidBinding { binding });
                };
                Ok((parsed_binding, action))
            })
            .collect::<Result<HashMap<_, _>, _>>()?;

        // Merge chat defaults with user config
        let mut chat_bindings = if self.chat.clear_defaults {
            HashMap::new()
        } else {
            default_chat_key_bindings()
        };
        chat_bindings.extend(self.chat.key_bindings);

        let chat_key_bindings = chat_bindings
            .into_iter()
            .map(|(binding, action)| {
                let Some(parsed_binding) = parse_key_combination(&binding) else {
                    return Err(ConfigError::InvalidBinding { binding });
                };
                let Ok(action) = ChatUserAction::try_from(action.as_str()) else {
                    return Err(ConfigError::InvalidBinding { binding });
                };
                Ok((parsed_binding, action))
            })
            .collect::<Result<HashMap<_, _>, _>>()?;

        // Merge graph defaults with user config
        let mut graph_bindings = if self.graph.clear_defaults {
            HashMap::new()
        } else {
            default_graph_key_bindings()
        };
        graph_bindings.extend(self.graph.key_bindings);

        let graph_key_bindings = graph_bindings
            .into_iter()
            .map(|(binding, action)| {
                let Some(parsed_binding) = parse_key_combination(&binding) else {
                    return Err(ConfigError::InvalidBinding { binding });
                };
                let Ok(action) = GraphUserAction::try_from(action.as_str()) else {
                    return Err(ConfigError::InvalidBinding { binding });
                };
                Ok((parsed_binding, action))
            })
            .collect::<Result<HashMap<_, _>, _>>()?;

        Ok(ParsedTuiConfig {
            dashboard: ParsedDashboardConfig {
                key_bindings: dashboard_key_bindings,
            },
            chat: ParsedChatConfig {
                key_bindings: chat_key_bindings,
            },
            graph: ParsedGraphConfig {
                key_bindings: graph_key_bindings,
            },
        })
    }
}
