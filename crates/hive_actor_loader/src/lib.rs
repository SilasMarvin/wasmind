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

    #[snafu(display("Missing required field '{field}' in Cargo.toml for actor '{actor_name}'."))]
    MissingRequiredField {
        actor_name: String,
        field: String,
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
    pub required_spawn_with: Vec<String>,
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

    pub async fn load_actors(&self, actors: Vec<Actor>, actor_overrides: Vec<hive_config::ActorOverride>) -> Result<Vec<LoadedActor>> {
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
            .resolve_all(actors, actor_overrides)
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
                    required_spawn_with: resolved.required_spawn_with.clone(),
                };
                self.load_single_actor(actor, resolved.actor_id.clone(), resolved.required_spawn_with)
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

    async fn load_single_actor(&self, actor: Actor, actor_id: String, required_spawn_with: Vec<String>) -> Result<LoadedActor> {
        println!("  Loading {}", actor.name);
        info!("Loading actor: {} (id: {})", actor.name, actor_id);

        let is_dev_mode = std::env::var("DEV_MODE").is_ok();

        // Check if actor is already cached (skip cache in dev mode)
        if !is_dev_mode {
            if let Some(cached) = self.check_cache(&actor, &actor_id, &required_spawn_with).await? {
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

        // Get version
        let version = self.get_actor_version(&build_path, package_name, &actor.name).await?;

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
            required_spawn_with,
        })
    }

    async fn check_cache(&self, actor: &Actor, actor_id: &str, required_spawn_with: &[String]) -> Result<Option<LoadedActor>> {
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
                required_spawn_with: required_spawn_with.to_vec(),
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

        // Helper function to update hive dependencies in a TOML table
        let update_hive_deps = |deps: &mut toml::Table| {
            if deps.contains_key("hive_actor_utils") {
                let mut table = toml::Table::new();
                table.insert("path".to_string(), toml::Value::String(hive_actor_utils_path.display().to_string()));
                table.insert("features".to_string(), toml::Value::Array(vec![toml::Value::String("macros".to_string())]));
                deps.insert("hive_actor_utils".to_string(), toml::Value::Table(table));
            }

            if deps.contains_key("hive_llm_types") {
                let mut table = toml::Table::new();
                table.insert("path".to_string(), toml::Value::String(hive_llm_types.display().to_string()));
                deps.insert("hive_llm_types".to_string(), toml::Value::Table(table));
            }
        };

        // Fix workspace-level dependencies first if this is a workspace
        let workspace_cargo_toml = actor_path.join("Cargo.toml");
        if workspace_cargo_toml.exists() {
            let workspace_content = fs::read_to_string(&workspace_cargo_toml)
                .await
                .context(IoSnafu { path: Some(workspace_cargo_toml.clone()) })?;

            let mut workspace_toml: toml::Value = toml::from_str(&workspace_content)
                .context(TomlDeserializeSnafu { text: workspace_content })?;

            // Update workspace dependencies
            if let Some(workspace_deps) = workspace_toml
                .get_mut("workspace")
                .and_then(|w| w.get_mut("dependencies"))
                .and_then(|d| d.as_table_mut())
            {
                update_hive_deps(workspace_deps);
            }

            // Write the updated workspace Cargo.toml
            let updated_workspace_content = toml::to_string_pretty(&workspace_toml).unwrap();
            fs::write(&workspace_cargo_toml, updated_workspace_content)
                .await
                .context(IoSnafu { path: Some(workspace_cargo_toml) })?;
        }

        // Determine the correct package Cargo.toml path
        let cargo_toml_path = match package_name {
            Some(package) => {
                // Package is the full subpath to the package
                let package_path = actor_path.join(package).join("Cargo.toml");
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
            .context(IoSnafu { path: Some(cargo_toml_path.clone()) })?;

        let mut cargo_toml: toml::Value = toml::from_str(&cargo_content)
            .context(TomlDeserializeSnafu { text: cargo_content })?;

        // Update package dependencies
        for dependency_type in ["dependencies", "dev-dependencies"] {
            if let Some(dependencies) = cargo_toml
                .get_mut(dependency_type)
                .and_then(|d| d.as_table_mut())
            {
                update_hive_deps(dependencies);
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

        // Determine the actual build directory
        let build_dir = if let Some(package) = package_name {
            // For packages, cd into the package directory
            actor_path.join(package)
        } else {
            // For single actors, build in the root
            actor_path.to_path_buf()
        };

        let mut cmd = Command::new("cargo-component");
        cmd.current_dir(&build_dir)
            .arg("build")
            .arg("--release");
        
        if package_name.is_some() {
            info!("Building in package directory: {:?}", build_dir);
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
        // When building in a package directory, the target directory is usually at the workspace root
        let target_dir = if package_name.is_some() {
            // For packages, check if there's a workspace target directory at the actor_path root
            let workspace_target = actor_path
                .join("target")
                .join("wasm32-wasip1")
                .join("release");
            
            if workspace_target.exists() {
                workspace_target
            } else {
                // Fallback to package target directory
                build_dir
                    .join("target")
                    .join("wasm32-wasip1")
                    .join("release")
            }
        } else {
            // For single actors, use the build directory target
            build_dir
                .join("target")
                .join("wasm32-wasip1")
                .join("release")
        };

        let mut entries = fs::read_dir(&target_dir).await.context(IoSnafu {
            path: Some(target_dir.to_path_buf()),
        })?;
        
        // When building in a package directory, we need to find the package name from Cargo.toml
        let expected_wasm_name = if package_name.is_some() {
            // Get the actual package name from the Cargo.toml in the package directory
            let package_cargo_toml = build_dir.join("Cargo.toml");
            if let Ok(content) = fs::read_to_string(&package_cargo_toml).await {
                if let Ok(cargo_toml) = toml::from_str::<toml::Value>(&content) {
                    if let Some(package_name) = cargo_toml.get("package")
                        .and_then(|p| p.get("name"))
                        .and_then(|n| n.as_str()) {
                        format!("{}.wasm", package_name.replace('-', "_"))
                    } else {
                        // Fallback - try to find any .wasm file
                        String::new()
                    }
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        if !expected_wasm_name.is_empty() {
            // Look for the specific wasm file
            let wasm_path = target_dir.join(&expected_wasm_name);
            if wasm_path.exists() {
                return Ok(wasm_path);
            }
        }

        // Fallback: find any .wasm file
        while let Some(entry) = entries.next_entry().await.context(IoSnafu { path: None })? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("wasm") {
                return Ok(path);
            }
        }
        
        Err(Error::WasmNotFound {
            actor_name: actor_name.to_string(),
            expected_wasm: if expected_wasm_name.is_empty() { 
                "<any .wasm file>".to_string() 
            } else { 
                expected_wasm_name 
            },
            target_dir: target_dir.display().to_string(),
            location: location!(),
        })
    }

    async fn get_actor_version(&self, actor_path: &Path, package_name: Option<&str>, actor_name: &str) -> Result<String> {
        // Since we only need the version (actor_id comes from Hive.toml now),
        // we can read the Cargo.toml directly for packages to avoid metadata complexity
        if let Some(package_path) = package_name {
            // For packages, read the package's Cargo.toml directly
            let package_cargo_toml = actor_path.join(package_path).join("Cargo.toml");
            let cargo_content = fs::read_to_string(&package_cargo_toml)
                .await
                .context(IoSnafu { path: Some(package_cargo_toml) })?;
            
            let cargo_toml: toml::Value = toml::from_str(&cargo_content)
                .context(TomlDeserializeSnafu { text: cargo_content })?;
            
            let version = cargo_toml
                .get("package")
                .and_then(|p| p.get("version"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::MissingRequiredField {
                    actor_name: actor_name.to_string(),
                    field: "version".to_string(),
                    location: location!(),
                })?;
                
            Ok(version.to_string())
        } else {
            // For single actors, use cargo metadata
            let metadata = MetadataCommand::new()
                .current_dir(actor_path)
                .exec()
                .context(CargoMetadataSnafu)?;

            let package = metadata
                .packages
                .iter()
                .find(|p| p.source.is_none())
                .ok_or_else(|| Error::PackageNotFound {
                    actor_name: actor_name.to_string(),
                    package_name: "root package".to_string(),
                    workspace_path: actor_path.display().to_string(),
                    location: location!(),
                })?;
                
            Ok(package.version.to_string())
        }
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
