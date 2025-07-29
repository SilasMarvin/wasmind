use etcetera::{AppStrategy, AppStrategyArgs, choose_app_strategy};
use serde::{Deserialize, de::DeserializeOwned};
use snafu::{Location, ResultExt, Snafu};
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
pub enum GitRef {
    Branch(String),
    Tag(String),
    Rev(String),
}

#[derive(Clone, Debug, Deserialize)]
pub struct Repository {
    pub url: Url,
    pub git_ref: Option<GitRef>,
}

#[derive(Clone, Debug, Deserialize)]
pub enum ActorSource {
    Path(String),
    Git(Repository),
}

#[derive(Clone, Debug, Deserialize)]
pub struct Actor {
    pub name: String,
    pub source: ActorSource,
    #[serde(default)]
    pub config: Option<toml::Table>,
}

#[derive(Clone, Debug)]
pub struct Config {
    raw_config: toml::Table,
    pub actors: Vec<Actor>,
    pub starting_actors: Vec<String>,
}

impl Config {
    pub fn parse_section<T: DeserializeOwned>(&self, section_name: &str) -> Result<Option<T>, Error> {
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
        actors_section.clone().try_into().context(TomlParseSnafu)?
    } else {
        Vec::new()
    };
    
    let starting_actors = if let Some(starting_actors_section) = raw_config.get("starting_actors") {
        starting_actors_section.clone().try_into().context(TomlParseSnafu)?
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
