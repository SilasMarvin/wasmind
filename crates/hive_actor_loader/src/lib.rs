use cargo_metadata::MetadataCommand;
use futures::future::join_all;
use sha2::{Digest, Sha256};
use snafu::{Location, ResultExt, Snafu, ensure, location};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tempfile::TempDir;
use tokio::fs;
use tokio::process::Command;
use tracing::{info, warn};
use url::Url;

use hive_config::{Actor, ActorSource, GitRef};

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("IO error: {} | Operation on path: {path:?}", source))]
    Io {
        source: std::io::Error,
        path: Option<PathBuf>,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to deserialize content: {text}"))]
    TomlDeserialize {
        source: toml::de::Error,
        text: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to deserialize content: {text}"))]
    Serde {
        source: serde_json::Error,
        text: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to create temp directory"))]
    TempDir {
        source: std::io::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to execute command: {}", source))]
    Command {
        source: std::io::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Command failed with status: {}", status))]
    CommandFailed {
        status: std::process::ExitStatus,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to parse cargo metadata"))]
    CargoMetadata {
        source: cargo_metadata::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to find package {} in cargo metadata", name))]
    PackageNotFound {
        name: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to find wasm file for actor {}", name))]
    WasmNotFound {
        name: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Config error"))]
    Config {
        source: hive_config::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Invalid path: {}", path))]
    InvalidPath {
        path: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Git command not found. Please install git."))]
    GitNotFound {
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Cargo component command not found. Please install cargo-component."))]
    CargoComponentNotFound {
        #[snafu(implicit)]
        location: Location,
    },
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct LoadedActor {
    pub id: String,
    pub name: String,
    pub version: String,
    pub wasm: Vec<u8>,
}

pub struct ActorLoader {
    cache_dir: PathBuf,
}

impl ActorLoader {
    pub fn new(cache_dir: Option<PathBuf>) -> Result<Self> {
        match cache_dir {
            Some(cache_dir) => Ok(Self { cache_dir }),
            None => {
                let cache_dir = hive_config::get_config_dir()
                    .context(ConfigSnafu)?
                    .join("actors");
                Ok(Self { cache_dir })
            }
        }
    }

    pub async fn load_actors(&self, actors: Vec<Actor>) -> Result<Vec<LoadedActor>> {
        // Ensure cache directory exists
        fs::create_dir_all(&self.cache_dir)
            .await
            .context(IoSnafu { path: None })?;

        // Check for required tools
        self.check_required_tools().await?;

        // Load all actors in parallel
        let tasks: Vec<_> = actors
            .into_iter()
            .map(|actor| self.load_single_actor(actor))
            .collect();

        let results = join_all(tasks).await;

        // Collect results, propagating any errors
        results.into_iter().collect::<Result<Vec<_>>>()
    }

    async fn check_required_tools(&self) -> Result<()> {
        // Check for git
        if which::which("git").is_err() {
            return Err(Error::GitNotFound {
                location: location!(),
            });
        }

        // Check for cargo-component
        if which::which("cargo-component").is_err() {
            return Err(Error::CargoComponentNotFound {
                location: location!(),
            });
        }

        Ok(())
    }

    async fn load_single_actor(&self, actor: Actor) -> Result<LoadedActor> {
        info!("Loading actor: {}", actor.name);

        let is_dev_mode = std::env::var("DEV_MODE").is_ok();

        // Check if actor is already cached (skip cache in dev mode)
        if !is_dev_mode {
            if let Some(cached) = self.check_cache(&actor).await? {
                info!("Using cached actor: {}", actor.name);
                return Ok(cached);
            }
        } else {
            info!(
                "DEV_MODE enabled - skipping cache for actor: {}",
                actor.name
            );
        }

        // Create temporary directory for building
        let temp_dir = TempDir::new().context(TempDirSnafu)?;
        let build_path = temp_dir.path().join(&actor.name);

        // Clone or copy the actor source
        match &actor.source {
            ActorSource::Path(path) => {
                self.copy_local_actor(path, &build_path).await?;
            }
            ActorSource::Git(repository) => {
                self.clone_git_actor(&repository.url, &build_path, repository.git_ref.as_ref())
                    .await?;
            }
        }

        // Handle local development mode
        if is_dev_mode {
            self.setup_local_dependencies(&build_path).await?;
        }

        // Build the actor
        let wasm_path = self.build_actor(&build_path).await?;

        // Read the built wasm
        let wasm = fs::read(&wasm_path).await.context(IoSnafu {
            path: Some(wasm_path),
        })?;

        // Get metadata
        let (crate_name, version) = self.get_actor_metadata(&build_path).await?;

        // Cache the built actor (skip in dev mode)
        if !is_dev_mode {
            self.cache_actor(&actor, &version, &wasm).await?;
        }

        Ok(LoadedActor {
            name: actor.name,
            version,
            id: crate_name,
            wasm,
        })
    }

    async fn check_cache(&self, actor: &Actor) -> Result<Option<LoadedActor>> {
        let actor_hash = self.compute_actor_hash(actor);
        let cache_path = self.cache_dir.join(&actor.name).join(&actor_hash);
        let metadata_path = cache_path.join("metadata.json");
        let wasm_path = cache_path.join("actor.wasm");

        if metadata_path.exists() && wasm_path.exists() {
            // Read metadata
            let metadata_content = fs::read_to_string(&metadata_path).await.context(IoSnafu {
                path: Some(metadata_path),
            })?;
            let metadata: serde_json::Value =
                serde_json::from_str(&metadata_content).context(SerdeSnafu {
                    text: metadata_content,
                })?;

            let crate_name = metadata["crate_name"]
                .as_str()
                .unwrap_or(&actor.name)
                .to_string();
            let version = metadata["version"].as_str().unwrap_or("0.0.0").to_string();

            // Read wasm
            let wasm = fs::read(&wasm_path).await.context(IoSnafu {
                path: Some(wasm_path),
            })?;

            return Ok(Some(LoadedActor {
                name: actor.name.clone(),
                version,
                id: crate_name,
                wasm,
            }));
        }

        Ok(None)
    }

    async fn cache_actor(&self, actor: &Actor, version: &str, wasm: &[u8]) -> Result<()> {
        let actor_hash = self.compute_actor_hash(actor);
        let cache_path = self.cache_dir.join(&actor.name).join(&actor_hash);

        // Create cache directory
        fs::create_dir_all(&cache_path).await.context(IoSnafu {
            path: Some(cache_path.clone()),
        })?;

        // Write wasm
        let wasm_path = cache_path.join("actor.wasm");
        fs::write(&wasm_path, wasm).await.context(IoSnafu {
            path: Some(wasm_path),
        })?;

        // Write metadata
        let metadata = serde_json::json!({
            "crate_name": &actor.name,
            "version": version,
            "cached_at": chrono::Utc::now().to_rfc3339(),
        });
        let metadata_path = cache_path.join("metadata.json");
        fs::write(&metadata_path, metadata.to_string())
            .await
            .context(IoSnafu {
                path: Some(metadata_path),
            })?;

        info!("Cached actor {} version {}", actor.name, version);
        Ok(())
    }

    fn compute_actor_hash(&self, actor: &Actor) -> String {
        let mut hasher = Sha256::new();
        hasher.update(&actor.name);
        match &actor.source {
            ActorSource::Path(path) => {
                hasher.update("path:");
                hasher.update(path);
            }
            ActorSource::Git(repo) => {
                hasher.update("git:");
                hasher.update(&repo.url.as_str());
                if let Some(git_ref) = &repo.git_ref {
                    match git_ref {
                        GitRef::Branch(branch) => hasher.update(&format!("branch:{branch}")),
                        GitRef::Tag(tag) => hasher.update(&format!("tag:{tag}")),
                        GitRef::Rev(rev) => hasher.update(&format!("rev:{rev}")),
                    }
                }
            }
        }
        hex::encode(hasher.finalize())
    }

    async fn copy_local_actor(&self, source: &str, dest: &Path) -> Result<()> {
        info!("Copying local actor from {} to {:?}", source, dest);

        let source_path = Path::new(source);
        ensure!(
            source_path.exists(),
            InvalidPathSnafu {
                path: source.to_string()
            }
        );

        // Copy directory recursively
        self.copy_dir_recursive(source_path, dest).await?;

        Ok(())
    }

    async fn copy_dir_recursive(&self, src: &Path, dst: &Path) -> Result<()> {
        // Create destination directory
        fs::create_dir_all(dst)
            .await
            .context(IoSnafu { path: None })?;

        let mut entries = fs::read_dir(src).await.context(IoSnafu {
            path: Some(dst.to_path_buf()),
        })?;

        while let Some(entry) = entries.next_entry().await.context(IoSnafu { path: None })? {
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());

            if entry
                .file_type()
                .await
                .context(IoSnafu {
                    path: Some(entry.path()),
                })?
                .is_dir()
            {
                fs::create_dir_all(&dst_path)
                    .await
                    .context(IoSnafu { path: None })?;
                Box::pin(self.copy_dir_recursive(&src_path, &dst_path)).await?;
            } else {
                fs::copy(&src_path, &dst_path).await.context(IoSnafu {
                    path: Some(src_path.to_path_buf()),
                })?;
            }
        }

        Ok(())
    }

    async fn clone_git_actor(
        &self,
        url: &Url,
        dest: &Path,
        specifier: Option<&GitRef>,
    ) -> Result<()> {
        info!("Cloning git actor from {} to {:?}", url, dest);

        let mut cmd = Command::new("git");
        cmd.arg("clone").arg(url.as_str()).arg(dest);

        let status = cmd.status().await.context(CommandSnafu)?;
        ensure!(status.success(), CommandFailedSnafu { status });

        // Checkout specific branch/tag/rev if specified
        if let Some(spec) = specifier {
            let mut checkout_cmd = Command::new("git");
            checkout_cmd.current_dir(dest).arg("checkout");

            match spec {
                GitRef::Branch(branch) => {
                    checkout_cmd.arg(branch);
                }
                GitRef::Tag(tag) => {
                    checkout_cmd.arg(tag);
                }
                GitRef::Rev(rev) => {
                    checkout_cmd.arg(rev);
                }
            }

            let checkout_status = checkout_cmd.status().await.context(CommandSnafu)?;
            ensure!(
                checkout_status.success(),
                CommandFailedSnafu {
                    status: checkout_status
                }
            );
        }

        Ok(())
    }

    async fn setup_local_dependencies(&self, actor_path: &Path) -> Result<()> {
        info!("Setting up local dependencies for development mode");

        // Find the workspace root and construct absolute paths
        let workspace_root = std::env::current_dir().context(IoSnafu { path: None })?;
        let hive_actor_utils_path = workspace_root.join("crates").join("hive_actor_utils");
        let hive_actor_bindings_path = workspace_root.join("crates").join("hive_actor_bindings");

        // Verify both paths exist
        ensure!(
            hive_actor_utils_path.exists(),
            InvalidPathSnafu {
                path: hive_actor_utils_path.display().to_string()
            }
        );
        ensure!(
            hive_actor_bindings_path.exists(),
            InvalidPathSnafu {
                path: hive_actor_bindings_path.display().to_string()
            }
        );

        // Update Cargo.toml to use absolute paths
        let cargo_toml_path = actor_path.join("Cargo.toml");
        let cargo_content = fs::read_to_string(&cargo_toml_path)
            .await
            .context(IoSnafu {
                path: Some(cargo_toml_path.clone()),
            })?;

        // Parse the Cargo.toml using toml crate
        let mut cargo_toml: toml::Value =
            toml::from_str(&cargo_content).context(TomlDeserializeSnafu {
                text: cargo_content,
            })?;

        // Update dependencies to use absolute paths
        if let Some(dependencies) = cargo_toml
            .get_mut("dependencies")
            .and_then(|d| d.as_table_mut())
        {
            // Update hive_actor_utils
            if dependencies.contains_key("hive_actor_utils") {
                dependencies.insert(
                    "hive_actor_utils".to_string(),
                    toml::Value::Table({
                        let mut table = toml::Table::new();
                        table.insert(
                            "path".to_string(),
                            toml::Value::String(hive_actor_utils_path.display().to_string()),
                        );
                        table.insert(
                            "features".to_string(),
                            toml::Value::Array(vec![toml::Value::String("macros".to_string())]),
                        );
                        table
                    }),
                );
            }
        }

        // Update the component WIT
        if let Some(dependencies) = cargo_toml
            .get_mut("package")
            .map(|x| x.get_mut("metadata"))
            .flatten()
            .map(|x| x.get_mut("component"))
            .flatten()
            .map(|x| x.get_mut("target"))
            .flatten()
            .map(|x| x.get_mut("dependencies"))
            .flatten()
            .and_then(|x| x.as_table_mut())
        {
            if dependencies.contains_key("hive:actor") {
                dependencies.insert(
                    "hive:actor".to_string(),
                    toml::Value::Table({
                        let mut table = toml::Table::new();
                        table.insert(
                            "path".to_string(),
                            toml::Value::String(
                                hive_actor_bindings_path.join("wit").display().to_string(),
                            ),
                        );
                        table
                    }),
                );
            }
        }

        // Write the updated Cargo.toml
        let updated_content = toml::to_string_pretty(&cargo_toml).unwrap();

        fs::write(&cargo_toml_path, updated_content)
            .await
            .context(IoSnafu {
                path: Some(cargo_toml_path),
            })?;

        Ok(())
    }

    async fn build_actor(&self, actor_path: &Path) -> Result<PathBuf> {
        info!("Building actor at {:?}", actor_path);

        let mut cmd = Command::new("cargo-component");
        cmd.current_dir(actor_path)
            .arg("build")
            .arg("--release")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = cmd.output().await.context(CommandSnafu)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("Build failed with stderr: {}", stderr);
            return Err(Error::CommandFailed {
                status: output.status,
                location: location!(),
            });
        }

        // Find the built wasm file
        let target_dir = actor_path
            .join("target")
            .join("wasm32-wasip1")
            .join("release");

        let mut entries = fs::read_dir(&target_dir).await.context(IoSnafu {
            path: Some(target_dir.to_path_buf()),
        })?;
        while let Some(entry) = entries.next_entry().await.context(IoSnafu { path: None })? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("wasm") {
                return Ok(path);
            }
        }

        Err(Error::WasmNotFound {
            name: actor_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            location: location!(),
        })
    }

    async fn get_actor_metadata(&self, actor_path: &Path) -> Result<(String, String)> {
        let metadata = MetadataCommand::new()
            .current_dir(actor_path)
            .exec()
            .context(CargoMetadataSnafu)?;

        // Find the main package
        let package = metadata
            .packages
            .iter()
            .find(|p| p.source.is_none())
            .ok_or_else(|| Error::PackageNotFound {
                name: "root package".to_string(),
                location: location!(),
            })?;

        Ok((package.name.clone(), package.version.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_actor_loader_creation() {
        let loader = ActorLoader::new(None);
        assert!(loader.is_ok());
    }
}
