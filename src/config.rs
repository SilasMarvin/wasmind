use etcetera::{AppStrategy, AppStrategyArgs, choose_app_strategy};
use genai::{
    ServiceTarget,
    resolver::{AuthData, Endpoint, ServiceTargetResolver},
};
use rdev::Key;
use serde::Deserialize;
use snafu::{ResultExt, Snafu};
use std::{collections::HashMap, fs, io, path::PathBuf};
use toml::Value;

use crate::worker::Action;

const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant.";

pub type KeyBinding = Vec<Key>;

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
    // Create an instance of Etcetera for your application "giggle".
    // The etcetera crate will determine the correct base config directory depending on the OS.
    let strategy = choose_app_strategy(AppStrategyArgs {
        top_level_domain: "org".to_string(), // Change to "com" if that's more appropriate.
        author: "spilot".to_string(),
        app_name: "spilot".to_string(),
    })
    .unwrap();

    // This returns the complete path to the config file "config.toml".
    // On Linux/macOS, this will be: $HOME/.config/giggle/config.toml
    // On Windows, this will typically be: %APPDATA%\giggle\config.toml
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
}

impl Config {
    pub fn new() -> Result<Self, ConfigError> {
        let config_file_path = get_config_file_path();
        let user_config: Option<Config> = if fs::exists(&config_file_path)? {
            let contents = fs::read_to_string(&config_file_path)?;
            Some(toml::from_str(&contents).context(TomlDeserializeSnafu)?)
        } else {
            None
        };

        if let Some(mut user_config) = user_config {
            if user_config.key_bindings.clear_defaults {
                Ok(user_config)
            } else {
                let mut default = Config::default()?;
                default
                    .key_bindings
                    .bindings
                    .extend(user_config.key_bindings.bindings);
                user_config.key_bindings.bindings = default.key_bindings.bindings;

                Ok(user_config)
            }
        } else {
            Config::default()
        }
    }

    pub fn default() -> Result<Self, ConfigError> {
        let default_contents = include_str!("../default_config.toml");
        toml::from_str(default_contents).context(TomlDeserializeSnafu)
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
#[derive(Deserialize, Default, Debug)]
struct ModelConfig {
    name: String,
    system_prompt: Option<String>,
    endpoint: Option<String>,
    auth: Option<String>,
    chat_config: Option<Value>,
    adapater: Option<String>,
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

        Ok(Self {
            keys,
            model,
            mcp_servers,
        })
    }
}

/// The parsed and verified config
#[derive(Debug, Clone)]
pub struct ParsedConfig {
    pub keys: ParsedKeyConfig,
    pub model: ParsedModelConfig,
    pub mcp_servers: HashMap<String, McpServerConfig>,
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
                .adapater
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
                    Ok(value) => AuthData::Key(value),
                    Err(_) => AuthData::FromEnv(s),
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

fn parse_key_combination(input: &str) -> Option<KeyBinding> {
    let parts: Vec<&str> = input.split('-').collect();
    let mut binding = vec![];

    for key_str in parts {
        let key = match key_str {
            // Modifiers
            "ctrl" => Key::ControlLeft,
            "alt" => Key::Alt, // Option on MacOS
            "meta" | "cmd" | "super" | "win" => Key::MetaLeft,
            "shift" => Key::ShiftLeft,

            // Letters
            "a" => Key::KeyA,
            "b" => Key::KeyB,
            "c" => Key::KeyC,
            "d" => Key::KeyD,
            "e" => Key::KeyE,
            "f" => Key::KeyF,
            "g" => Key::KeyG,
            "h" => Key::KeyH,
            "i" => Key::KeyI,
            "j" => Key::KeyJ,
            "k" => Key::KeyK,
            "l" => Key::KeyL,
            "m" => Key::KeyM,
            "n" => Key::KeyN,
            "o" => Key::KeyO,
            "p" => Key::KeyP,
            "q" => Key::KeyQ,
            "r" => Key::KeyR,
            "s" => Key::KeyS,
            "t" => Key::KeyT,
            "u" => Key::KeyU,
            "v" => Key::KeyV,
            "w" => Key::KeyW,
            "x" => Key::KeyX,
            "y" => Key::KeyY,
            "z" => Key::KeyZ,

            // Numbers
            "0" => Key::Num0,
            "1" => Key::Num1,
            "2" => Key::Num2,
            "3" => Key::Num3,
            "4" => Key::Num4,
            "5" => Key::Num5,
            "6" => Key::Num6,
            "7" => Key::Num7,
            "8" => Key::Num8,
            "9" => Key::Num9,
            _ => return None,
        };
        binding.push(key);
    }
    Some(binding)
}
