use etcetera::{AppStrategy, AppStrategyArgs, choose_app_strategy};
use snafu::{Location, ResultExt, Snafu};
use std::path::PathBuf;
use url::Url;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Config Error"))]
    Config {
        #[snafu(source)]
        source: etcetera::HomeDirError,
        #[snafu(implicit)]
        location: Location,
    },
}

#[derive(Clone, Debug)]
pub enum GitSourceSpecifier {
    Branch(String),
    Tag(String),
    Rev(String),
}

#[derive(Clone, Debug)]
pub struct GitSource {
    url: Url,
    specified: GitSourceSpecifier,
}

#[derive(Clone, Debug)]
pub enum ActorSource {
    Path(String),
    Git(Url),
}

#[derive(Clone, Debug)]
pub struct Actors {
    pub name: String,
    pub source: ActorSource,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub actors: Vec<Actors>,
}

// TODO: Implement config parsing

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
