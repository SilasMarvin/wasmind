//! # Hive Configuration Management
//!
//! This crate provides configuration management for the Hive actor orchestration system.

use etcetera::{AppStrategy, AppStrategyArgs, choose_app_strategy};
use serde::{Deserialize, de::DeserializeOwned};
use snafu::{Location, OptionExt, ResultExt, Snafu};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Config directory error"))]
    Config {
        #[snafu(source)]
        source: etcetera::HomeDirError,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("IO error: {}", source))]
    Io {
        #[snafu(source)]
        source: std::io::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Error reading file: `{:?}` - {}", file, source))]
    ReadingFile {
        file: PathBuf,
        #[snafu(source)]
        source: std::io::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("TOML parsing error: {}", source))]
    TomlParse {
        #[snafu(source)]
        source: toml::de::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Invalid configuration section '[{}]': {}", section, reason))]
    InvalidSection {
        section: String,
        reason: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Actor manifest not found: {}", path.display()))]
    InvalidManifestLocation { 
        path: PathBuf,
        #[snafu(implicit)]
        location: Location,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GitRef {
    Branch(String),
    Tag(String),
    Rev(String),
}

#[derive(Clone, Debug, Deserialize)]
pub struct PathSource {
    pub path: String,
    pub package: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Repository {
    pub url: Url,
    pub git_ref: Option<GitRef>,
    pub package: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum ActorSource {
    Path(PathSource),
    Git(Repository),
}

#[derive(Clone, Debug, Deserialize)]
pub struct Actor {
    /// The logical name of the actor. This is populated from the TOML key, not the value.
    #[serde(skip)]
    pub name: String,
    pub source: ActorSource,
    #[serde(default)]
    pub config: Option<toml::Table>,
    #[serde(default)]
    pub auto_spawn: bool,
    #[serde(default)]
    pub required_spawn_with: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ActorOverride {
    /// The logical name of the actor override. This is populated from the TOML key, not the value.
    #[serde(skip)]
    pub name: String,
    #[serde(default)]
    pub source: Option<ActorSource>,
    #[serde(default)]
    pub config: Option<toml::Table>,
    #[serde(default)]
    pub auto_spawn: Option<bool>,
    #[serde(default)]
    pub required_spawn_with: Option<Vec<String>>,
}

#[derive(Clone, Debug)]
pub struct Config {
    raw_config: toml::Table,
    pub actors: Vec<Actor>,
    pub actor_overrides: Vec<ActorOverride>,
    pub starting_actors: Vec<String>,
}

impl Config {
    pub fn parse_section<T: DeserializeOwned>(
        &self,
        section_name: &str,
    ) -> Result<Option<T>, Error> {
        if let Some(section) = self.raw_config.get(section_name) {
            let value: T = section.clone().try_into().context(TomlParseSnafu)?;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    pub fn get_raw_table(&self, section_name: &str) -> Option<&toml::Table> {
        self.raw_config.get(section_name)?.as_table()
    }
}

pub fn load_from_path<P: AsRef<Path> + ToOwned<Owned = PathBuf>>(path: P) -> Result<Config, Error> {
    let content = std::fs::read_to_string(&path).context(ReadingFileSnafu {
        file: path.to_owned(),
    })?;
    let raw_config: toml::Table = toml::from_str(&content).context(TomlParseSnafu)?;

    let actors = if let Some(actors_section) = raw_config.get("actors") {
        let actors_table = actors_section.as_table().context(InvalidSectionSnafu {
            section: "actors",
            reason: "must be a table, not a different type",
        })?;

        let mut actors_vec = Vec::new();
        for (name, value) in actors_table {
            let mut actor: Actor = value.clone().try_into().context(TomlParseSnafu)?;
            actor.name.clone_from(name);
            actors_vec.push(actor);
        }
        actors_vec
    } else {
        Vec::new()
    };

    let starting_actors = if let Some(starting_actors_section) = raw_config.get("starting_actors") {
        starting_actors_section
            .clone()
            .try_into()
            .context(TomlParseSnafu)?
    } else {
        Vec::new()
    };

    // Parse actor_overrides section
    let actor_overrides = if let Some(overrides_section) = raw_config.get("actor_overrides") {
        let overrides_table = overrides_section.as_table().context(InvalidSectionSnafu {
            section: "actor_overrides",
            reason: "must be a table, not a different type",
        })?;

        let mut overrides_vec = Vec::new();
        for (name, value) in overrides_table {
            // TOML naturally handles dotted keys like [actor_overrides.logger.config]
            // They become nested tables, so we can just deserialize normally
            let mut actor_override: ActorOverride =
                value.clone().try_into().context(TomlParseSnafu)?;
            actor_override.name.clone_from(name);
            overrides_vec.push(actor_override);
        }
        overrides_vec
    } else {
        Vec::new()
    };

    Ok(Config {
        raw_config,
        actors,
        actor_overrides,
        starting_actors,
    })
}

pub fn load_default_config() -> Result<Config, Error> {
    let config_path = get_config_file_path()?;
    load_from_path(config_path)
}

fn get_app_strategy() -> Result<impl AppStrategy, Error> {
    choose_app_strategy(AppStrategyArgs {
        top_level_domain: "com".to_string(),
        author: "hive".to_string(),
        app_name: "hive".to_string(),
    })
    .context(ConfigSnafu)
}

pub fn get_config_dir() -> Result<PathBuf, Error> {
    // Create an instance of Etcetera for your application "hive".
    // The etcetera crate will determine the correct base config directory depending on the OS.
    Ok(get_app_strategy()?.config_dir())
}

pub fn get_cache_dir() -> Result<PathBuf, Error> {
    // On Linux/macOS, this will be: $HOME/.cache/hive/
    // On Windows, this will typically be: %LOCALAPPDATA%\hive\
    Ok(get_app_strategy()?.cache_dir())
}

pub fn get_config_file_path() -> Result<PathBuf, Error> {
    // This returns the complete path to the config file "config.toml".
    // On Linux/macOS, this will be: $HOME/.config/hive/config.toml
    // On Windows, this will typically be: %APPDATA%\hive\config.toml
    Ok(get_config_dir()?.join("config.toml"))
}

#[derive(Clone, Debug, Deserialize)]
pub struct DependencyConfig {
    pub source: ActorSource,
    #[serde(default)]
    pub auto_spawn: Option<bool>,
    #[serde(default)]
    pub config: Option<toml::Table>,
    #[serde(default)]
    pub required_spawn_with: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ActorManifest {
    pub actor_id: String,
    #[serde(default)]
    pub dependencies: HashMap<String, DependencyConfig>,
    #[serde(default)]
    pub required_spawn_with: Vec<String>,
}

impl ActorManifest {
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let manifest_path = path.as_ref().join("Hive.toml");
        if !manifest_path.exists() {
            return InvalidManifestLocationSnafu {
                path: manifest_path,
            }.fail();
        }

        let content = std::fs::read_to_string(&manifest_path).context(ReadingFileSnafu {
            file: manifest_path.clone(),
        })?;

        let manifest: ActorManifest = toml::from_str(&content).context(TomlParseSnafu)?;
        Ok(manifest)
    }
}

pub fn get_actors_cache_dir() -> Result<PathBuf, Error> {
    Ok(get_cache_dir()?.join("actors"))
}

pub fn get_log_file_path() -> Result<PathBuf, Error> {
    // Log file goes in the data directory
    let data_dir = get_data_dir()?;
    Ok(data_dir.join("hive.log"))
}

pub fn get_data_dir() -> Result<PathBuf, Error> {
    // On Linux/macOS, this will be: $HOME/.local/share/hive/
    // On Windows, this will typically be: %APPDATA%\hive\
    Ok(get_app_strategy()?.data_dir())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_actor_manifest_parsing() {
        let temp_dir = TempDir::new().unwrap();
        let manifest_content = r#"
actor_id = "test-company:test-actor"

[dependencies.logger]
source = { path = "../logger" }
auto_spawn = true

[dependencies.logger.config]
level = "info"
format = "json"

[dependencies.helper]
source = { url = "https://github.com/test/helper", git_ref = { branch = "main" } }
"#;

        fs::write(temp_dir.path().join("Hive.toml"), manifest_content).unwrap();

        let manifest = ActorManifest::from_path(temp_dir.path()).unwrap();
        assert_eq!(manifest.actor_id, "test-company:test-actor");
        assert_eq!(manifest.dependencies.len(), 2);

        let logger_dep = &manifest.dependencies["logger"];
        assert!(matches!(logger_dep.source, ActorSource::Path(_)));
        assert_eq!(logger_dep.auto_spawn, Some(true));
        assert!(logger_dep.config.is_some());

        let helper_dep = &manifest.dependencies["helper"];
        assert!(matches!(helper_dep.source, ActorSource::Git(_)));
    }

    #[test]
    fn test_config_parsing_with_table_format() {
        let temp_dir = TempDir::new().unwrap();
        let config_content = r#"
starting_actors = ["assistant", "coordinator"]

[actors.assistant]
source = { path = "./actors/assistant" }
auto_spawn = true

[actors.assistant.config]
model = "gpt-4"
temperature = 0.7

[actors.coordinator]
source = { url = "https://github.com/test/coordinator", git_ref = { tag = "v1.0.0" } }

[actors.bash_executor]
source = { path = "./actors/bash" }
auto_spawn = false
"#;

        let config_path = temp_dir.path().join("config.toml");
        fs::write(&config_path, config_content).unwrap();

        let config = load_from_path(config_path).unwrap();
        assert_eq!(config.starting_actors, vec!["assistant", "coordinator"]);
        assert_eq!(config.actors.len(), 3);

        // Find actors by name
        let assistant = config
            .actors
            .iter()
            .find(|a| a.name == "assistant")
            .unwrap();
        assert!(assistant.auto_spawn);
        assert!(matches!(assistant.source, ActorSource::Path(_)));
        assert!(assistant.config.is_some());

        let coordinator = config
            .actors
            .iter()
            .find(|a| a.name == "coordinator")
            .unwrap();
        assert!(matches!(coordinator.source, ActorSource::Git(_)));
        assert!(!coordinator.auto_spawn); // defaults to false
    }

    #[test]
    fn test_invalid_actors_section_type() {
        let temp_dir = TempDir::new().unwrap();

        // actors section as a string instead of a table
        let invalid_config = r#"
actors = "this should be a table, not a string"
"#;

        let config_path = temp_dir.path().join("config.toml");
        fs::write(&config_path, invalid_config).unwrap();

        let result = load_from_path(config_path);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Invalid configuration section '[actors]'")
        );
    }

    #[test]
    fn test_invalid_actor_overrides_section_type() {
        let temp_dir = TempDir::new().unwrap();

        // actor_overrides section as an array instead of a table
        let invalid_config = r#"
actor_overrides = ["should", "be", "a", "table"]
"#;

        let config_path = temp_dir.path().join("config.toml");
        fs::write(&config_path, invalid_config).unwrap();

        let result = load_from_path(config_path);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Invalid configuration section '[actor_overrides]'")
        );
    }

    #[test]
    fn test_actor_overrides_parsing() {
        let temp_dir = TempDir::new().unwrap();
        let config_content = r#"
[actors.my_assistant]
source = { path = "./actors/assistant" }
auto_spawn = true

[actor_overrides.logger]
source = { path = "./custom_logger" }
auto_spawn = false

[actor_overrides.logger.config]
level = "debug"
format = "json"

[actor_overrides.database.config]
connection_string = "postgres://localhost/test"
"#;

        let config_path = temp_dir.path().join("config.toml");
        fs::write(&config_path, config_content).unwrap();

        let config = load_from_path(config_path).unwrap();

        // Check actors
        assert_eq!(config.actors.len(), 1);
        assert!(config.actors.iter().any(|a| a.name == "my_assistant"));

        // Check overrides
        assert_eq!(config.actor_overrides.len(), 2);

        // Find logger override
        let logger_override = config
            .actor_overrides
            .iter()
            .find(|o| o.name == "logger")
            .unwrap();
        assert!(matches!(logger_override.source, Some(ActorSource::Path(_))));
        assert_eq!(logger_override.auto_spawn, Some(false));
        assert!(logger_override.config.is_some());
        let logger_config = logger_override.config.as_ref().unwrap();
        assert_eq!(
            logger_config.get("level").unwrap().as_str().unwrap(),
            "debug"
        );
        assert_eq!(
            logger_config.get("format").unwrap().as_str().unwrap(),
            "json"
        );

        // Find database override (config-only)
        let db_override = config
            .actor_overrides
            .iter()
            .find(|o| o.name == "database")
            .unwrap();
        assert!(db_override.source.is_none());
        assert!(db_override.auto_spawn.is_none());
        assert!(db_override.config.is_some());
        let db_config = db_override.config.as_ref().unwrap();
        assert_eq!(
            db_config
                .get("connection_string")
                .unwrap()
                .as_str()
                .unwrap(),
            "postgres://localhost/test"
        );
    }

    #[test]
    fn test_mixed_actor_and_override_definitions() {
        let temp_dir = TempDir::new().unwrap();
        let config_content = r#"
starting_actors = ["assistant", "executor"]

[actors.assistant]
source = { path = "./actors/assistant" }
auto_spawn = true
required_spawn_with = ["logger"]

[actors.executor]
source = { path = "./actors/bash" }

[actors.executor.config]
shell = "/bin/zsh"
timeout = 30

[actor_overrides.logger]
auto_spawn = true

[actor_overrides.logger.config]
level = "info"
"#;

        let config_path = temp_dir.path().join("config.toml");
        fs::write(&config_path, config_content).unwrap();

        let config = load_from_path(config_path).unwrap();

        // Check starting actors
        assert_eq!(config.starting_actors, vec!["assistant", "executor"]);

        // Check actors
        assert_eq!(config.actors.len(), 2);

        let assistant = config
            .actors
            .iter()
            .find(|a| a.name == "assistant")
            .unwrap();
        assert_eq!(assistant.required_spawn_with, vec!["logger"]);

        let executor = config.actors.iter().find(|a| a.name == "executor").unwrap();
        assert!(executor.config.is_some());
        let exec_config = executor.config.as_ref().unwrap();
        assert_eq!(
            exec_config.get("shell").unwrap().as_str().unwrap(),
            "/bin/zsh"
        );
        assert_eq!(
            exec_config.get("timeout").unwrap().as_integer().unwrap(),
            30
        );

        // Check overrides
        assert_eq!(config.actor_overrides.len(), 1);
        let logger_override = config
            .actor_overrides
            .iter()
            .find(|o| o.name == "logger")
            .unwrap();
        assert_eq!(logger_override.auto_spawn, Some(true));
    }

    #[test]
    fn test_invalid_starting_actors_type() {
        let temp_dir = TempDir::new().unwrap();

        // starting_actors as a string instead of an array
        let invalid_config = r#"
starting_actors = "should be an array"
"#;

        let config_path = temp_dir.path().join("config.toml");
        fs::write(&config_path, invalid_config).unwrap();

        let result = load_from_path(config_path);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error.to_string().contains("TOML parsing error"));
    }

    #[test]
    fn test_invalid_actor_configuration() {
        let temp_dir = TempDir::new().unwrap();

        // Actor with invalid source field
        let invalid_config = r#"
[actors.bad_actor]
source = "should be a table with path or url"
"#;

        let config_path = temp_dir.path().join("config.toml");
        fs::write(&config_path, invalid_config).unwrap();

        let result = load_from_path(config_path);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error.to_string().contains("TOML parsing error"));
    }

    #[test]
    fn test_invalid_manifest_file() {
        let temp_dir = TempDir::new().unwrap();

        // Create an invalid manifest file
        let invalid_manifest = r#"
# Missing required actor_id field
dependencies = {}
"#;
        fs::write(temp_dir.path().join("Hive.toml"), invalid_manifest).unwrap();

        let result = ActorManifest::from_path(temp_dir.path());
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error.to_string().contains("TOML parsing error"));
    }

    #[test]
    fn test_missing_manifest_file() {
        let temp_dir = TempDir::new().unwrap();

        // Don't create any Hive.toml file
        let result = ActorManifest::from_path(temp_dir.path());
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error.to_string().contains("Actor manifest not found"));
    }

    #[test]
    fn test_toml_dotted_key_parsing() {
        // Test how TOML handles dotted keys vs nested tables
        let dotted_toml = r#"
[actor_overrides.logger.config]
level = "debug"
format = "json"
"#;

        let nested_toml = r#"
[actor_overrides.logger]
config = { level = "debug", format = "json" }
"#;

        let dotted_parsed: toml::Table = toml::from_str(dotted_toml).unwrap();
        let nested_parsed: toml::Table = toml::from_str(nested_toml).unwrap();

        // Both should create the same structure
        let dotted_overrides = dotted_parsed
            .get("actor_overrides")
            .unwrap()
            .as_table()
            .unwrap();
        let nested_overrides = nested_parsed
            .get("actor_overrides")
            .unwrap()
            .as_table()
            .unwrap();

        // Both should have "logger" as a key
        assert!(dotted_overrides.contains_key("logger"));
        assert!(nested_overrides.contains_key("logger"));

        // Both logger values should be tables with a "config" key
        let dotted_logger = dotted_overrides.get("logger").unwrap().as_table().unwrap();
        let nested_logger = nested_overrides.get("logger").unwrap().as_table().unwrap();

        assert!(dotted_logger.contains_key("config"));
        assert!(nested_logger.contains_key("config"));

        // The config values should be identical
        let dotted_config = dotted_logger.get("config").unwrap();
        let nested_config = nested_logger.get("config").unwrap();

        assert_eq!(dotted_config, nested_config);
    }
}
