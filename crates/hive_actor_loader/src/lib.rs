pub mod dependency_resolver;

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

    #[snafu(display("Failed to deserialize TOML content: {text}"))]
    TomlDeserialize {
        source: toml::de::Error,
        text: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to deserialize JSON content: {text}"))]
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

    #[snafu(display("Build failed for actor '{actor_name}' with exit code {status}. {stderr}"))]
    CommandFailed {
        actor_name: String,
        status: std::process::ExitStatus,
        stderr: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to parse cargo metadata"))]
    CargoMetadata {
        source: cargo_metadata::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to load actor '{actor_name}'. Package '{package_name}' not found in workspace at '{workspace_path}'."))]
    PackageNotFound {
        actor_name: String,
        package_name: String,
        workspace_path: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to load actor '{actor_name}'. WASM file '{expected_wasm}' not found in target directory '{target_dir}'. Ensure the actor builds successfully."))]
    WasmNotFound {
        actor_name: String,
        expected_wasm: String,
        target_dir: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Config error"))]
    Config {
        source: hive_config::Error,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to load actor '{actor_name}'. Source path '{path}' not found."))]
    InvalidPath {
        actor_name: String,
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

    #[snafu(display("Dependency resolution failed"))]
    DependencyResolution {
        #[snafu(source)]
        source: dependency_resolver::Error,
        #[snafu(implicit)]
        location: Location,
    },
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct LoadedActor {
    pub id: String,  // This will be the actor_id from manifest
    pub name: String,  // This is the logical name
    pub version: String,
    pub wasm: Vec<u8>,
    pub config: Option<toml::Table>,
    pub auto_spawn: bool,
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

        // Phase 1: Resolve all dependencies
        println!("Resolving actor dependencies...");
        let resolver = dependency_resolver::DependencyResolver::new();
        let resolved_actors = resolver
            .resolve_all(actors)
            .context(DependencyResolutionSnafu)?;

        // Phase 2: Load all resolved actors in parallel
        println!("Loading {} actors...", resolved_actors.len());
        let tasks: Vec<_> = resolved_actors
            .into_iter()
            .map(|(logical_name, resolved)| {
                let actor = Actor {
                    name: logical_name,
                    source: resolved.source,
                    config: resolved.config,
                    auto_spawn: resolved.auto_spawn,
                };
                self.load_single_actor(actor, resolved.actor_id.clone())
            })
            .collect();

        let results = join_all(tasks).await;

        // Collect results, propagating any errors
        let loaded_actors = results.into_iter().collect::<Result<Vec<_>>>()?;
        
        println!("✓ Actor loading complete");
        Ok(loaded_actors)
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

    async fn load_single_actor(&self, actor: Actor, actor_id: String) -> Result<LoadedActor> {
        println!("  Loading {}", actor.name);
        info!("Loading actor: {} (id: {})", actor.name, actor_id);

        let is_dev_mode = std::env::var("DEV_MODE").is_ok();

        // Check if actor is already cached (skip cache in dev mode)
        if !is_dev_mode {
            if let Some(cached) = self.check_cache(&actor, &actor_id).await? {
                println!("  ✓ {} (cached)", actor.name);
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

        // Get package name from source
        let package_name = match &actor.source {
            ActorSource::Path(path_source) => path_source.package.as_deref(),
            ActorSource::Git(repository) => repository.package.as_deref(),
        };

        // Clone or copy the actor source
        match &actor.source {
            ActorSource::Path(path_source) => {
                self.copy_local_actor(&path_source.path, &build_path, &actor.name).await?;
            }
            ActorSource::Git(repository) => {
                self.clone_git_actor(&repository.url, &build_path, repository.git_ref.as_ref())
                    .await?;
            }
        }

        // Handle local development mode
        if is_dev_mode {
            self.setup_local_dependencies(&build_path, package_name, &actor.name).await?;
        }

        // Build the actor
        let wasm_path = self.build_actor(&build_path, package_name, &actor.name).await?;

        // Read the built wasm
        let wasm = fs::read(&wasm_path).await.context(IoSnafu {
            path: Some(wasm_path),
        })?;

        // Get metadata
        let (_, version) = self.get_actor_metadata(&build_path, package_name, &actor.name).await?;

        // Cache the built actor (skip in dev mode)
        if !is_dev_mode {
            self.cache_actor(&actor, &actor_id, &version, &wasm).await?;
        }

        println!("  ✓ {} (built)", actor.name);
        
        Ok(LoadedActor {
            name: actor.name.clone(),
            version,
            id: actor_id,
            wasm,
            config: actor.config,
            auto_spawn: actor.auto_spawn,
        })
    }

    async fn check_cache(&self, actor: &Actor, actor_id: &str) -> Result<Option<LoadedActor>> {
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

            let cached_actor_id = metadata["actor_id"]
                .as_str()
                .unwrap_or(actor_id)
                .to_string();
            let version = metadata["version"].as_str().unwrap_or("0.0.0").to_string();

            // Read wasm
            let wasm = fs::read(&wasm_path).await.context(IoSnafu {
                path: Some(wasm_path),
            })?;

            return Ok(Some(LoadedActor {
                name: actor.name.clone(),
                version,
                id: cached_actor_id,
                wasm,
                config: actor.config.clone(),
                auto_spawn: actor.auto_spawn,
            }));
        }

        Ok(None)
    }

    async fn cache_actor(&self, actor: &Actor, actor_id: &str, version: &str, wasm: &[u8]) -> Result<()> {
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
            "actor_id": actor_id,
            "logical_name": &actor.name,
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
            ActorSource::Path(path_source) => {
                hasher.update("path:");
                hasher.update(&path_source.path);
                if let Some(package) = &path_source.package {
                    hasher.update("package:");
                    hasher.update(package);
                }
            }
            ActorSource::Git(repo) => {
                hasher.update("git:");
                hasher.update(repo.url.as_str());
                if let Some(git_ref) = &repo.git_ref {
                    match git_ref {
                        GitRef::Branch(branch) => hasher.update(format!("branch:{branch}")),
                        GitRef::Tag(tag) => hasher.update(format!("tag:{tag}")),
                        GitRef::Rev(rev) => hasher.update(format!("rev:{rev}")),
                    }
                }
                if let Some(package) = &repo.package {
                    hasher.update("package:");
                    hasher.update(package);
                }
            }
        }
        hex::encode(hasher.finalize())
    }

    async fn copy_local_actor(&self, source: &str, dest: &Path, actor_name: &str) -> Result<()> {
        info!("Copying local actor from {} to {:?}", source, dest);

        let source_path = Path::new(source);
        ensure!(
            source_path.exists(),
            InvalidPathSnafu {
                actor_name: actor_name.to_string(),
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
        if !status.success() {
            return Err(Error::CommandFailed {
                actor_name: "<git-clone>".to_string(),
                status,
                stderr: "Git clone failed".to_string(),
                location: location!(),
            });
        }

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
            if !checkout_status.success() {
                return Err(Error::CommandFailed {
                    actor_name: "<git-checkout>".to_string(),
                    status: checkout_status,
                    stderr: "Git checkout failed".to_string(),
                    location: location!(),
                });
            }
        }

        Ok(())
    }

    async fn setup_local_dependencies(&self, actor_path: &Path, package_name: Option<&str>, actor_name: &str) -> Result<()> {
        info!("Setting up local dependencies for development mode");

        // Find the workspace root and construct absolute paths
        let workspace_root = std::env::current_dir().context(IoSnafu { path: None })?;
        let hive_actor_utils_path = workspace_root.join("crates").join("hive_actor_utils");
        let hive_llm_types = workspace_root.join("crates").join("hive_llm_types");
        let hive_actor_bindings_path = workspace_root.join("crates").join("hive_actor_bindings");

        // Verify both paths exist
        ensure!(
            hive_actor_utils_path.exists(),
            InvalidPathSnafu {
                actor_name: actor_name.to_string(),
                path: hive_actor_utils_path.display().to_string()
            }
        );
        ensure!(
            hive_actor_bindings_path.exists(),
            InvalidPathSnafu {
                actor_name: actor_name.to_string(),
                path: hive_actor_bindings_path.display().to_string()
            }
        );

        // Determine the correct Cargo.toml path
        let cargo_toml_path = match package_name {
            Some(package) => {
                // For workspace packages, we only look under the crates/ directory
                let package_path = actor_path.join("crates").join(package).join("Cargo.toml");
                if package_path.exists() {
                    package_path
                } else {
                    return Err(Error::PackageNotFound {
                        actor_name: actor_name.to_string(),
                        package_name: package.to_string(),
                        workspace_path: actor_path.display().to_string(),
                        location: location!(),
                    });
                }
            }
            None => actor_path.join("Cargo.toml"),
        };
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

        for dependency_type in ["dependencies", "dev-dependencies"] {
            // Update dependencies to use absolute paths
            if let Some(dependencies) = cargo_toml
                .get_mut(dependency_type)
                .and_then(|d| d.as_table_mut())
            {
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

                if dependencies.contains_key("hive_llm_types") {
                    dependencies.insert(
                        "hive_llm_types".to_string(),
                        toml::Value::Table({
                            let mut table = toml::Table::new();
                            table.insert(
                                "path".to_string(),
                                toml::Value::String(hive_llm_types.display().to_string()),
                            );
                            table
                        }),
                    );
                }
            }
        }

        // Update the component WIT
        if let Some(dependencies) = cargo_toml
            .get_mut("package")
            .and_then(|x| x.get_mut("metadata"))
            .and_then(|x| x.get_mut("component"))
            .and_then(|x| x.get_mut("target"))
            .and_then(|x| x.get_mut("dependencies"))
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

    async fn build_actor(&self, actor_path: &Path, package_name: Option<&str>, actor_name: &str) -> Result<PathBuf> {
        info!("Building actor at {:?}", actor_path);

        let mut cmd = Command::new("cargo-component");
        cmd.current_dir(actor_path)
            .arg("build")
            .arg("--release");
        
        // If package name is specified, build only that package
        if let Some(package) = package_name {
            cmd.arg("-p").arg(package);
            info!("Building specific package: {}", package);
        }
        
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = cmd.output().await.context(CommandSnafu)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("Build failed with stderr: {}", stderr);
            return Err(Error::CommandFailed {
                actor_name: actor_name.to_string(),
                status: output.status,
                stderr: stderr.into_owned(),
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
        
        let expected_wasm_name = match package_name {
            Some(package) => format!("{}.wasm", package.replace('-', "_")),
            None => {
                // For single package, find any .wasm file
                while let Some(entry) = entries.next_entry().await.context(IoSnafu { path: None })? {
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("wasm") {
                        return Ok(path);
                    }
                }
                return Err(Error::WasmNotFound {
                    actor_name: actor_name.to_string(),
                    expected_wasm: "<any .wasm file>".to_string(),
                    target_dir: target_dir.display().to_string(),
                    location: location!(),
                });
            }
        };

        // Look for the specific package's wasm file
        let wasm_path = target_dir.join(&expected_wasm_name);
        if wasm_path.exists() {
            Ok(wasm_path)
        } else {
            Err(Error::WasmNotFound {
                actor_name: actor_name.to_string(),
                expected_wasm: expected_wasm_name,
                target_dir: target_dir.display().to_string(),
                location: location!(),
            })
        }
    }

    async fn get_actor_metadata(&self, actor_path: &Path, package_name: Option<&str>, actor_name: &str) -> Result<(String, String)> {
        let metadata = MetadataCommand::new()
            .current_dir(actor_path)
            .exec()
            .context(CargoMetadataSnafu)?;

        let package = match package_name {
            Some(name) => {
                // Find the specific package in the workspace
                metadata
                    .packages
                    .iter()
                    .find(|p| p.name == name)
                    .ok_or_else(|| Error::PackageNotFound {
                        actor_name: actor_name.to_string(),
                        package_name: name.to_string(),
                        workspace_path: actor_path.display().to_string(),
                        location: location!(),
                    })?
            }
            None => {
                // Find the main package (no workspace or single package)
                metadata
                    .packages
                    .iter()
                    .find(|p| p.source.is_none())
                    .ok_or_else(|| Error::PackageNotFound {
                        actor_name: actor_name.to_string(),
                        package_name: "root package".to_string(),
                        workspace_path: actor_path.display().to_string(),
                        location: location!(),
                    })?
            }
        };

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
