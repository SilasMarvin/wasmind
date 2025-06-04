use etcetera::{AppStrategy, AppStrategyArgs, choose_app_strategy};
use genai::{
    ServiceTarget,
    resolver::{AuthData, Endpoint, ServiceTargetResolver},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;
use snafu::{ResultExt, Snafu};
use std::{collections::HashMap, fs, io, path::PathBuf};

use crate::actors::Action;

const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant.";

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
    model: ModelConfig,
    #[serde(default)]
    key_bindings: KeyConfig,
    #[serde(default)]
    mcp_servers: HashMap<String, McpServerConfig>,
    #[serde(default)]
    whitelisted_commands: Vec<String>,
    #[serde(default)]
    auto_approve_commands: bool,
    #[serde(default)]
    hive: HiveConfig,
}

impl Config {
    pub fn new() -> Result<Self, ConfigError> {
        // Check for environment variable first
        let config_file_path = if let Ok(env_path) = std::env::var("HIVE_CONFIG_PATH") {
            tracing::debug!("Using config path from HIVE_CONFIG_PATH env var: {}", env_path);
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
            let default = Config::default()?;

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

                tracing::debug!(
                    "Final whitelisted commands: {:?}",
                    user_config.whitelisted_commands
                );
                Ok(user_config)
            }
        } else {
            let config = Config::default()?;
            tracing::debug!(
                "Using default config, whitelisted commands: {:?}",
                config.whitelisted_commands
            );
            Ok(config)
        }
    }

    pub fn from_file(path: &str) -> Result<Self, ConfigError> {
        let contents = fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&contents).context(TomlDeserializeSnafu)?;

        // Merge with default whitelisted commands if none specified
        if config.whitelisted_commands.is_empty() {
            let default = Config::default()?;
            config.whitelisted_commands = default.whitelisted_commands;
        }

        Ok(config)
    }

    pub fn default() -> Result<Self, ConfigError> {
        // Use headless config for builds without GUI features
        #[cfg(not(feature = "gui"))]
        let default_contents = include_str!("../headless_config.toml");
        
        #[cfg(feature = "gui")]
        let default_contents = include_str!("../default_config.toml");
        
        tracing::debug!("Default config contents:\n{}", default_contents);
        let config: Config = toml::from_str(default_contents).context(TomlDeserializeSnafu)?;
        tracing::debug!(
            "Parsed default config - whitelisted_commands: {:?}",
            config.whitelisted_commands
        );
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
#[derive(Deserialize, Default, Debug, Clone)]
pub struct ModelConfig {
    name: String,
    system_prompt: Option<String>,
    endpoint: Option<String>,
    auth: Option<String>,
    adapter: Option<String>,
}

/// HIVE multi-agent configuration
#[derive(Deserialize, Default, Debug, Clone)]
pub struct HiveConfig {
    /// Model configuration for the main manager agent
    #[serde(default)]
    pub main_manager_model: Option<ModelConfig>,
    /// Model configuration for sub-manager agents
    #[serde(default)]
    pub sub_manager_model: Option<ModelConfig>,
    /// Model configuration for worker agents
    #[serde(default)]
    pub worker_model: Option<ModelConfig>,
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

        let model = parse_model_config(value.model);

        let mcp_servers = value.mcp_servers;

        let whitelisted_commands = value.whitelisted_commands;
        let auto_approve_commands = value.auto_approve_commands;

        let hive = ParsedHiveConfig {
            main_manager_model: value.hive.main_manager_model.map(parse_model_config),
            sub_manager_model: value.hive.sub_manager_model.map(parse_model_config),
            worker_model: value.hive.worker_model.map(parse_model_config),
        };

        tracing::debug!("Loaded whitelisted commands: {:?}", whitelisted_commands);

        Ok(Self {
            keys,
            model,
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
    pub model: ParsedModelConfig,
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
    pub name: String,
    pub system_prompt: String,
    pub service_target_resolver: ServiceTargetResolver,
}

/// The parsed and verified HIVE config
#[derive(Debug, Clone)]
pub struct ParsedHiveConfig {
    pub main_manager_model: Option<ParsedModelConfig>,
    pub sub_manager_model: Option<ParsedModelConfig>,
    pub worker_model: Option<ParsedModelConfig>,
}

fn parse_model_config(model_config: ModelConfig) -> ParsedModelConfig {
    let model_name = model_config.name.clone();
    let service_target_resolver = ServiceTargetResolver::from_resolver_fn(
        move |service_target: ServiceTarget| -> Result<ServiceTarget, genai::resolver::Error> {
            let ServiceTarget {
                model,
                endpoint,
                auth,
            } = service_target;
            let model = model_config
                .adapter
                .map(|adapter| {
                    serde_json::from_value(serde_json::json!({
                        "adapter_kind": adapter,
                        "model_name": model_config.name,
                    }))
                    .unwrap()
                })
                .unwrap_or(model);
            let endpoint = model_config
                .endpoint
                .map(|endpoint| Endpoint::from_owned(endpoint))
                .unwrap_or(endpoint);
            let auth = match model_config.auth {
                None => auth,
                Some(s) => match std::env::var(&s) {
                    Ok(value) => {
                        tracing::debug!("Successfully loaded auth from environment variable");
                        AuthData::Key(value)
                    }
                    Err(_) => {
                        tracing::debug!(
                            "Environment variable not found, using FromEnv auth method"
                        );
                        AuthData::FromEnv(s)
                    }
                },
            };
            Ok(ServiceTarget {
                endpoint,
                auth,
                model,
            })
        },
    );

    ParsedModelConfig {
        name: model_name,
        service_target_resolver,
        system_prompt: model_config
            .system_prompt
            .unwrap_or(DEFAULT_SYSTEM_PROMPT.to_string()),
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
