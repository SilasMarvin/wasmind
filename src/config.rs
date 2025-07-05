use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use etcetera::{AppStrategy, AppStrategyArgs, choose_app_strategy};
use serde::{Deserialize, Serialize};
use serde_json;
use snafu::{ResultExt, Snafu};
use std::{collections::HashMap, fs, io, path::PathBuf};
use toml::Table;

use crate::actors::Action;

pub type KeyBinding = Vec<KeyEvent>;

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

/// The config we deserialize directly from toml
#[derive(Deserialize)]
pub struct Config {
    #[serde(default)]
    key_bindings: KeyConfig,
    #[serde(default)]
    mcp_servers: HashMap<String, McpServerConfig>,
    #[serde(default)]
    whitelisted_commands: Vec<String>,
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

        tracing::debug!("Looking for config file at: {:?}", config_file_path);
        let user_config: Option<Config> = if fs::exists(&config_file_path)? {
            tracing::debug!("Found user config file");
            let contents = fs::read_to_string(&config_file_path)?;
            Some(toml::from_str(&contents).context(TomlDeserializeSnafu)?)
        } else {
            tracing::debug!("No user config file found, using defaults");
            None
        };

        if let Some(mut user_config) = user_config {
            // Always load default config to get whitelisted commands
            let default = Config::load_default(is_headless)?;

            if user_config.key_bindings.clear_defaults {
                // Even with clear_defaults, use default whitelisted commands if user hasn't specified any
                if user_config.whitelisted_commands.is_empty() {
                    user_config.whitelisted_commands = default.whitelisted_commands;
                }
                Ok(user_config)
            } else {
                // Merge key bindings: add default bindings that don't conflict with user bindings
                for (binding, action) in default.key_bindings.bindings {
                    // Only add default bindings if the user hasn't defined this binding
                    if !user_config.key_bindings.bindings.contains_key(&binding) {
                        user_config.key_bindings.bindings.insert(binding, action);
                    }
                }

                // Merge whitelisted commands if user config doesn't have any
                // or extend the default list with user's additional commands
                if user_config.whitelisted_commands.is_empty() {
                    user_config.whitelisted_commands = default.whitelisted_commands;
                } else {
                    // Prepend default whitelisted commands to user's list
                    let mut merged_whitelist = default.whitelisted_commands;
                    merged_whitelist.extend(user_config.whitelisted_commands);
                    // Remove duplicates while preserving order
                    let mut seen = std::collections::HashSet::new();
                    user_config.whitelisted_commands = merged_whitelist
                        .into_iter()
                        .filter(|cmd| seen.insert(cmd.clone()))
                        .collect();
                }

                Ok(user_config)
            }
        } else {
            let config = Config::load_default(is_headless)?;
            Ok(config)
        }
    }

    pub fn from_file(path: &str, is_headless: bool) -> Result<Self, ConfigError> {
        let contents = fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&contents).context(TomlDeserializeSnafu)?;

        // Merge with default whitelisted commands if none specified
        if config.whitelisted_commands.is_empty() {
            let default = Config::load_default(is_headless)?;
            config.whitelisted_commands = default.whitelisted_commands;
        }

        Ok(config)
    }

    pub fn load_default(is_headless: bool) -> Result<Self, ConfigError> {
        let default_contents = if is_headless {
            include_str!("../headless_config.toml")
        } else {
            include_str!("../default_config.toml")
        };

        let config: Config = toml::from_str(default_contents).context(TomlDeserializeSnafu)?;
        Ok(config)
    }
}

/// The key bindings we deserialize directly from toml
#[derive(Deserialize, Default, Debug)]
struct KeyConfig {
    #[serde(default)]
    clear_defaults: bool,
    #[serde(default)]
    bindings: HashMap<String, String>,
}

/// The model configuration we deserialize directly from toml
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct ModelConfig {
    pub model_name: String,
    pub system_prompt: String,
    pub litellm_params: Table,
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

    /// Whether to remove container on exit
    #[serde(default = "default_auto_remove")]
    pub auto_remove: bool,

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

fn default_auto_remove() -> bool {
    false
}

impl Default for LiteLLMConfig {
    fn default() -> Self {
        Self {
            image: default_litellm_image(),
            port: default_litellm_port(),
            container_name: default_container_name(),
            auto_remove: default_auto_remove(),
            env_overrides: HashMap::new(),
        }
    }
}

/// Temporal worker configuration
#[derive(Deserialize, Default, Debug, Clone)]
pub struct TemporalConfig {
    /// Model configuration for check_health temporal worker
    #[serde(default)]
    pub check_health: Option<ModelConfig>,
}

/// HIVE multi-agent configuration
#[derive(Deserialize, Default, Debug, Clone)]
pub struct HiveConfig {
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
        let keys = {
            let bindings = value
                .key_bindings
                .bindings
                .into_iter()
                .map(|(binding, action)| {
                    let Some(parsed_binding) = parse_key_combination(&binding) else {
                        return Err(ConfigError::InvalidBinding { binding });
                    };

                    let Some(action) = Action::from_str(&action) else {
                        return Err(ConfigError::InvalidActionForBinding { action, binding });
                    };

                    Ok((parsed_binding, action))
                })
                .collect::<Result<_, _>>()?;

            ParsedKeyConfig { bindings }
        };

        let mcp_servers = value.mcp_servers;

        let whitelisted_commands = value.whitelisted_commands;
        let auto_approve_commands = value.auto_approve_commands;

        let base_url = format!("http://localhost:{}", value.hive.litellm.port);
        let hive = ParsedHiveConfig {
            main_manager_model: parse_model_config(value.hive.main_manager_model, &base_url),
            sub_manager_model: parse_model_config(value.hive.sub_manager_model, &base_url),
            worker_model: parse_model_config(value.hive.worker_model, &base_url),
            temporal: ParsedTemporalConfig {
                check_health: value
                    .hive
                    .temporal
                    .check_health
                    .map(|config| parse_model_config(config, &base_url)),
            },
            litellm: value.hive.litellm,
        };

        tracing::error!("THE PARSED HIVE CONFIG:\n{:?}", hive);

        tracing::error!(
            "THE PARSED HIVE CONFIG FOR THE WORKER_MODEL\n{:?}",
            hive.worker_model
        );

        Ok(Self {
            keys,
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
    pub keys: ParsedKeyConfig,
    pub mcp_servers: HashMap<String, McpServerConfig>,
    pub whitelisted_commands: Vec<String>,
    pub auto_approve_commands: bool,
    pub hive: ParsedHiveConfig,
}

/// The parsed and verified key bindings
/// For now we only allow mapping one event to an action but in the future we may allow creating vec![] of
/// key events
#[derive(Debug, Clone)]
pub struct ParsedKeyConfig {
    pub bindings: HashMap<KeyBinding, Action>,
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
    pub check_health: Option<ParsedModelConfig>,
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

fn parse_model_config(model_config: ModelConfig, base_url: &str) -> ParsedModelConfig {
    ParsedModelConfig {
        model_name: model_config.model_name,
        system_prompt: model_config.system_prompt,
        litellm_params: model_config
            .litellm_params
            .into_iter()
            .map(|(key, value)| (key, convert_toml_to_json(value)))
            .collect(),
        base_url: base_url.to_string(),
    }
}

pub fn parse_key_combination(input: &str) -> Option<KeyBinding> {
    let parts: Vec<&str> = input.split('-').collect();
    let mut modifiers = KeyModifiers::empty();
    let mut key_code = None;

    for part in parts {
        match part {
            // Modifiers
            "ctrl" => modifiers |= KeyModifiers::CONTROL,
            "alt" => modifiers |= KeyModifiers::ALT,
            "meta" | "cmd" | "super" | "win" => modifiers |= KeyModifiers::SUPER,
            "shift" => modifiers |= KeyModifiers::SHIFT,

            // Letters
            "a" => key_code = Some(KeyCode::Char('a')),
            "b" => key_code = Some(KeyCode::Char('b')),
            "c" => key_code = Some(KeyCode::Char('c')),
            "d" => key_code = Some(KeyCode::Char('d')),
            "e" => key_code = Some(KeyCode::Char('e')),
            "f" => key_code = Some(KeyCode::Char('f')),
            "g" => key_code = Some(KeyCode::Char('g')),
            "h" => key_code = Some(KeyCode::Char('h')),
            "i" => key_code = Some(KeyCode::Char('i')),
            "j" => key_code = Some(KeyCode::Char('j')),
            "k" => key_code = Some(KeyCode::Char('k')),
            "l" => key_code = Some(KeyCode::Char('l')),
            "m" => key_code = Some(KeyCode::Char('m')),
            "n" => key_code = Some(KeyCode::Char('n')),
            "o" => key_code = Some(KeyCode::Char('o')),
            "p" => key_code = Some(KeyCode::Char('p')),
            "q" => key_code = Some(KeyCode::Char('q')),
            "r" => key_code = Some(KeyCode::Char('r')),
            "s" => key_code = Some(KeyCode::Char('s')),
            "t" => key_code = Some(KeyCode::Char('t')),
            "u" => key_code = Some(KeyCode::Char('u')),
            "v" => key_code = Some(KeyCode::Char('v')),
            "w" => key_code = Some(KeyCode::Char('w')),
            "x" => key_code = Some(KeyCode::Char('x')),
            "y" => key_code = Some(KeyCode::Char('y')),
            "z" => key_code = Some(KeyCode::Char('z')),

            // Numbers
            "0" => key_code = Some(KeyCode::Char('0')),
            "1" => key_code = Some(KeyCode::Char('1')),
            "2" => key_code = Some(KeyCode::Char('2')),
            "3" => key_code = Some(KeyCode::Char('3')),
            "4" => key_code = Some(KeyCode::Char('4')),
            "5" => key_code = Some(KeyCode::Char('5')),
            "6" => key_code = Some(KeyCode::Char('6')),
            "7" => key_code = Some(KeyCode::Char('7')),
            "8" => key_code = Some(KeyCode::Char('8')),
            "9" => key_code = Some(KeyCode::Char('9')),

            // Special keys
            "enter" => key_code = Some(KeyCode::Enter),
            "escape" | "esc" => key_code = Some(KeyCode::Esc),
            "space" => key_code = Some(KeyCode::Char(' ')),
            "tab" => key_code = Some(KeyCode::Tab),
            "backspace" => key_code = Some(KeyCode::Backspace),
            "delete" | "del" => key_code = Some(KeyCode::Delete),
            "insert" => key_code = Some(KeyCode::Insert),
            "home" => key_code = Some(KeyCode::Home),
            "end" => key_code = Some(KeyCode::End),
            "pageup" => key_code = Some(KeyCode::PageUp),
            "pagedown" => key_code = Some(KeyCode::PageDown),
            "up" => key_code = Some(KeyCode::Up),
            "down" => key_code = Some(KeyCode::Down),
            "left" => key_code = Some(KeyCode::Left),
            "right" => key_code = Some(KeyCode::Right),

            // Function keys
            "f1" => key_code = Some(KeyCode::F(1)),
            "f2" => key_code = Some(KeyCode::F(2)),
            "f3" => key_code = Some(KeyCode::F(3)),
            "f4" => key_code = Some(KeyCode::F(4)),
            "f5" => key_code = Some(KeyCode::F(5)),
            "f6" => key_code = Some(KeyCode::F(6)),
            "f7" => key_code = Some(KeyCode::F(7)),
            "f8" => key_code = Some(KeyCode::F(8)),
            "f9" => key_code = Some(KeyCode::F(9)),
            "f10" => key_code = Some(KeyCode::F(10)),
            "f11" => key_code = Some(KeyCode::F(11)),
            "f12" => key_code = Some(KeyCode::F(12)),

            _ => return None,
        }
    }

    let key_code = key_code?;
    let key_event = KeyEvent::new(key_code, modifiers);
    Some(vec![key_event])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize() {}
}
