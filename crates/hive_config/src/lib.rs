use etcetera::{AppStrategy, AppStrategyArgs, choose_app_strategy};
use serde::{Deserialize, de::DeserializeOwned};
use snafu::{Location, ResultExt, Snafu};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Config error"))]
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

    #[snafu(display("Error reading file: `{:?}` {}", file, source))]
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
    #[serde(skip)]
    pub name: String,
    pub source: ActorSource,
    #[serde(default)]
    pub config: Option<toml::Table>,
    #[serde(default)]
    pub auto_spawn: bool,
}

#[derive(Clone, Debug)]
pub struct Config {
    raw_config: toml::Table,
    pub actors: Vec<Actor>,
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
        if let Some(actors_table) = actors_section.as_table() {
            let mut actors_vec = Vec::new();
            for (name, value) in actors_table {
                let mut actor: Actor = value.clone().try_into().context(TomlParseSnafu)?;
                actor.name.clone_from(name);
                actors_vec.push(actor);
            }
            actors_vec
        } else {
            // actors_section exists but is not a table
            Vec::new()
        }
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

    Ok(Config {
        raw_config,
        actors,
        starting_actors,
    })
}

pub fn load_default_config() -> Result<Config, Error> {
    let config_path = get_config_file_path()?;
    load_from_path(config_path)
}

pub fn get_config_dir() -> Result<PathBuf, Error> {
    // Create an instance of Etcetera for your application "hive".
    // The etcetera crate will determine the correct base config directory depending on the OS.
    Ok(choose_app_strategy(AppStrategyArgs {
        top_level_domain: "com".to_string(),
        author: "hive".to_string(),
        app_name: "hive".to_string(),
    })
    .context(ConfigSnafu)?
    .config_dir())
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
}

#[derive(Clone, Debug, Deserialize)]
pub struct ActorManifest {
    pub actor_id: String,
    #[serde(default)]
    pub dependencies: HashMap<String, DependencyConfig>,
}

impl ActorManifest {
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let manifest_path = path.as_ref().join("Hive.toml");
        if !manifest_path.exists() {
            return Err(Error::Io {
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Hive.toml not found at {manifest_path:?}"),
                ),
                location: snafu::Location::default(),
            });
        }
        
        let content = std::fs::read_to_string(&manifest_path).context(ReadingFileSnafu {
            file: manifest_path.clone(),
        })?;
        
        let manifest: ActorManifest = toml::from_str(&content).context(TomlParseSnafu)?;
        Ok(manifest)
    }
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
        let assistant = config.actors.iter().find(|a| a.name == "assistant").unwrap();
        assert!(assistant.auto_spawn);
        assert!(matches!(assistant.source, ActorSource::Path(_)));
        assert!(assistant.config.is_some());
        
        let coordinator = config.actors.iter().find(|a| a.name == "coordinator").unwrap();
        assert!(matches!(coordinator.source, ActorSource::Git(_)));
        assert!(!coordinator.auto_spawn); // defaults to false
    }
}
