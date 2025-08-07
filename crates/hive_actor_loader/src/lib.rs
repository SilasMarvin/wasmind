//! # Hive Actor Loader
//!
//! Dynamic loading and dependency resolution system for Hive WASM actor components.
//! This crate handles downloading, building, caching, and loading actors from various
//! sources (local paths, Git repositories, etc.).

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

    #[snafu(display(
        "Failed to load actor '{actor_name}'. Package '{package_name}' not found in workspace at '{workspace_path}'."
    ))]
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

    #[snafu(display(
        "Failed to load actor '{actor_name}'. WASM file '{expected_wasm}' not found in target directory '{target_dir}'. Ensure the actor builds successfully."
    ))]
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

    #[snafu(display("Missing required dependency: {dependency}. {install_message}"))]
    MissingDependency {
        dependency: &'static str,
        install_message: String,
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
    pub id: String,   // This will be the actor_id from manifest
    pub name: String, // This is the logical name
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
                let cache_dir = hive_config::get_cache_dir()
                    .context(ConfigSnafu)?
                    .join("actors");
                Ok(Self { cache_dir })
            }
        }
    }

    pub async fn load_actors(
        &self,
        actors: Vec<Actor>,
        actor_overrides: Vec<hive_config::ActorOverride>,
    ) -> Result<Vec<LoadedActor>> {
        // Ensure cache directory exists
        fs::create_dir_all(&self.cache_dir)
            .await
            .context(IoSnafu { path: None })?;

        // Check for required tools
        self.check_required_tools().await?;

        // Phase 1: Resolve all dependencies
        #[cfg(feature = "progress-output")]
        println!("Resolving actor dependencies...");
        let resolver = dependency_resolver::DependencyResolver::new();
        let resolved_actors = resolver
            .resolve_all(actors, actor_overrides)
            .context(DependencyResolutionSnafu)?;

        // Phase 2: Load all resolved actors in parallel
        #[cfg(feature = "progress-output")]
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
                self.load_single_actor(
                    actor,
                    resolved.actor_id.clone(),
                    resolved.required_spawn_with,
                )
            })
            .collect();

        let results = join_all(tasks).await;

        // Collect results, propagating any errors
        let loaded_actors = results.into_iter().collect::<Result<Vec<_>>>()?;

        #[cfg(feature = "progress-output")]
        println!("✓ Actor loading complete");
        Ok(loaded_actors)
    }

    async fn check_required_tools(&self) -> Result<()> {
        // Check for git
        if which::which("git").is_err() {
            return Err(Error::MissingDependency {
                dependency: "git",
                install_message: "Please install git to clone remote actors.".to_string(),
                location: location!(),
            });
        }

        // Check for cargo
        if which::which("cargo").is_err() {
            return Err(Error::MissingDependency {
                dependency: "cargo",
                install_message: "Please install Rust and Cargo from https://rustup.rs/"
                    .to_string(),
                location: location!(),
            });
        }

        // Check for cargo-component
        if which::which("cargo-component").is_err() {
            return Err(Error::MissingDependency {
                dependency: "cargo-component",
                install_message:
                    "Please install cargo-component with: cargo install cargo-component".to_string(),
                location: location!(),
            });
        }

        Ok(())
    }

    async fn load_single_actor(
        &self,
        actor: Actor,
        actor_id: String,
        required_spawn_with: Vec<String>,
    ) -> Result<LoadedActor> {
        #[cfg(feature = "progress-output")]
        println!("  Loading {}", actor.name);
        info!("Loading actor: {} (id: {})", actor.name, actor_id);

        let is_dev_mode = std::env::var("DEV_MODE").is_ok();

        // Check if actor is already cached (skip cache in dev mode)
        if !is_dev_mode {
            if let Some(cached) = self
                .check_cache(&actor, &actor_id, &required_spawn_with)
                .await?
            {
                #[cfg(feature = "progress-output")]
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

        // Get package name from source
        let package_name = match &actor.source {
            ActorSource::Path(path_source) => path_source.package.as_deref(),
            ActorSource::Git(repository) => repository.package.as_deref(),
        };

        let (_build_path, wasm_path, version) = match &actor.source {
            ActorSource::Path(path_source) => {
                // For local path dependencies, build in-place using default target directory for speed
                info!("Building local actor in-place: {}", path_source.path);
                let source_path = Path::new(&path_source.path);

                // Verify source path exists
                ensure!(
                    source_path.exists(),
                    InvalidPathSnafu {
                        actor_name: actor.name.clone(),
                        path: path_source.path.clone()
                    }
                );

                // No dependency setup needed for in-place builds - they use their existing workspace dependencies

                // Build using default target directory (faster - reuses build artifacts)
                let wasm_path = self
                    .build_actor(source_path, package_name, &actor.name)
                    .await?;

                // Get version from original source
                let version = self
                    .get_actor_version(source_path, package_name, &actor.name)
                    .await?;

                (source_path.to_path_buf(), wasm_path, version)
            }
            ActorSource::Git(repository) => {
                // For Git dependencies, continue copying to temp for isolation
                info!("Copying Git actor to temp: {}", repository.url);
                let temp_dir = TempDir::new().context(TempDirSnafu)?;
                let build_path = temp_dir.path().join(&actor.name);

                self.clone_git_actor(&repository.url, &build_path, repository.git_ref.as_ref())
                    .await?;

                // Git dependencies use their original dependencies as-is

                // Build in temp directory
                let wasm_path = self
                    .build_actor(&build_path, package_name, &actor.name)
                    .await?;

                // Get version from temp copy
                let version = self
                    .get_actor_version(&build_path, package_name, &actor.name)
                    .await?;

                (build_path, wasm_path, version)
            }
        };

        // Read the built wasm
        let wasm = fs::read(&wasm_path).await.context(IoSnafu {
            path: Some(wasm_path),
        })?;

        // Cache the built actor (skip in dev mode)
        if !is_dev_mode {
            self.cache_actor(&actor, &actor_id, &version, &wasm).await?;
        }

        #[cfg(feature = "progress-output")]
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

    pub async fn check_cache(
        &self,
        actor: &Actor,
        actor_id: &str,
        required_spawn_with: &[String],
    ) -> Result<Option<LoadedActor>> {
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

    pub async fn cache_actor(
        &self,
        actor: &Actor,
        actor_id: &str,
        version: &str,
        wasm: &[u8],
    ) -> Result<()> {
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

    async fn build_actor(
        &self,
        actor_path: &Path,
        package_name: Option<&str>,
        actor_name: &str,
    ) -> Result<PathBuf> {
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
        cmd.current_dir(&build_dir).arg("build").arg("--release");

        if package_name.is_some() {
            info!("Building in package directory: {:?}", build_dir);
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

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

        // Get the expected WASM file name from the package manifest
        let expected_name = self.get_expected_wasm_name(&build_dir)?;
        let wasm_path = target_dir.join(&expected_name);

        if wasm_path.exists() {
            Ok(wasm_path)
        } else {
            Err(Error::WasmNotFound {
                actor_name: actor_name.to_string(),
                expected_wasm: expected_name,
                target_dir: target_dir.display().to_string(),
                location: location!(),
            })
        }
    }

    fn get_expected_wasm_name(&self, build_dir: &Path) -> Result<String> {
        let manifest_path = build_dir.join("Cargo.toml");

        let metadata = MetadataCommand::new()
            .manifest_path(&manifest_path)
            .exec()
            .context(CargoMetadataSnafu)?;

        let package = metadata
            .packages
            .iter()
            .find(|p| p.source.is_none())
            .ok_or_else(|| Error::MissingRequiredField {
                actor_name: "<package>".to_string(),
                field: "name".to_string(),
                location: location!(),
            })?;

        Ok(format!("{}.wasm", package.name.replace('-', "_")))
    }

    async fn get_actor_version(
        &self,
        actor_path: &Path,
        package_name: Option<&str>,
        actor_name: &str,
    ) -> Result<String> {
        // Determine the manifest path
        let manifest_path = if let Some(package) = package_name {
            actor_path.join(package).join("Cargo.toml")
        } else {
            actor_path.join("Cargo.toml")
        };

        // Use cargo metadata with explicit manifest path
        let metadata = MetadataCommand::new()
            .manifest_path(&manifest_path)
            .exec()
            .context(CargoMetadataSnafu)?;

        // Find the package (should be the root package of the manifest we specified)
        let package = metadata
            .packages
            .iter()
            .find(|p| p.source.is_none())
            .ok_or_else(|| Error::PackageNotFound {
                actor_name: actor_name.to_string(),
                package_name: package_name.unwrap_or("root").to_string(),
                workspace_path: manifest_path.display().to_string(),
                location: location!(),
            })?;

        Ok(package.version.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_actor_loader_creation() {
        let loader = ActorLoader::new(None);
        assert!(loader.is_ok());
    }

    #[tokio::test]
    async fn test_build_successful_actor() {
        let loader = ActorLoader::new(None).unwrap();
        let test_actor_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("test_actors")
            .join("buildable_simple");

        // Should build successfully
        let result = loader
            .build_actor(&test_actor_path, None, "buildable_simple")
            .await;
        assert!(
            result.is_ok(),
            "Failed to build buildable_simple: {:?}",
            result.err()
        );

        let wasm_path = result.unwrap();
        assert!(
            wasm_path.exists(),
            "Built wasm file should exist at {:?}",
            wasm_path
        );
        assert_eq!(wasm_path.extension().unwrap(), "wasm");
    }

    #[tokio::test]
    async fn test_build_failing_actor() {
        let loader = ActorLoader::new(None).unwrap();
        let test_actor_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("test_actors")
            .join("buildable_fail");

        // Should fail to build
        let result = loader
            .build_actor(&test_actor_path, None, "buildable_fail")
            .await;
        assert!(
            result.is_err(),
            "buildable_fail should have failed to build"
        );

        match result.err().unwrap() {
            Error::CommandFailed {
                actor_name, stderr, ..
            } => {
                assert_eq!(actor_name, "buildable_fail");
                assert!(!stderr.is_empty(), "Should have build error output");
            }
            other => panic!("Expected CommandFailed error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_get_actor_version_success() {
        let loader = ActorLoader::new(None).unwrap();
        let test_actor_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("test_actors")
            .join("buildable_simple");

        let result = loader
            .get_actor_version(&test_actor_path, None, "buildable_simple")
            .await;
        assert!(result.is_ok(), "Failed to get version: {:?}", result.err());
        assert_eq!(result.unwrap(), "0.1.0");
    }

    #[tokio::test]
    async fn test_get_actor_version_nonexistent() {
        let loader = ActorLoader::new(None).unwrap();
        let nonexistent_path = PathBuf::from("/nonexistent/path");

        let result = loader
            .get_actor_version(&nonexistent_path, None, "nonexistent")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cache_and_load_actor() {
        let temp_dir = TempDir::new().unwrap();
        let loader = ActorLoader::new(Some(temp_dir.path().to_path_buf())).unwrap();

        let test_wasm = b"fake wasm content";
        let actor = Actor {
            name: "test_actor".to_string(),
            source: ActorSource::Path(hive_config::PathSource {
                path: "/test/path".to_string(),
                package: None,
            }),
            config: None,
            auto_spawn: false,
            required_spawn_with: vec![],
        };

        // Cache the actor
        loader
            .cache_actor(&actor, "test:actor", "1.0.0", test_wasm)
            .await
            .unwrap();

        // Try to load from cache
        let cached = loader.check_cache(&actor, "test:actor", &[]).await.unwrap();
        assert!(cached.is_some());

        let loaded_actor = cached.unwrap();
        assert_eq!(loaded_actor.name, "test_actor");
        assert_eq!(loaded_actor.version, "1.0.0");
        assert_eq!(loaded_actor.id, "test:actor");
        assert_eq!(loaded_actor.wasm, test_wasm);
    }

    #[tokio::test]
    async fn test_wasm_file_discovery() {
        let loader = ActorLoader::new(None).unwrap();
        let test_actor_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("test_actors")
            .join("buildable_simple");

        // Build the actor first to ensure wasm file exists
        let wasm_path = loader
            .build_actor(&test_actor_path, None, "buildable_simple")
            .await;
        assert!(
            wasm_path.is_ok(),
            "Build should succeed for file discovery test"
        );

        let target_dir = test_actor_path
            .join("target")
            .join("wasm32-wasip1")
            .join("release");

        // Verify the discovered wasm file actually exists
        let wasm_file = wasm_path.unwrap();
        assert!(wasm_file.exists());
        assert!(wasm_file.starts_with(&target_dir));
    }

    #[tokio::test]
    async fn test_actor_hash_computation() {
        let loader = ActorLoader::new(None).unwrap();

        let actor1 = Actor {
            name: "test".to_string(),
            source: ActorSource::Path(hive_config::PathSource {
                path: "/path1".to_string(),
                package: None,
            }),
            config: None,
            auto_spawn: false,
            required_spawn_with: vec![],
        };

        let actor2 = Actor {
            name: "test".to_string(),
            source: ActorSource::Path(hive_config::PathSource {
                path: "/path2".to_string(),
                package: None,
            }),
            config: None,
            auto_spawn: false,
            required_spawn_with: vec![],
        };

        let hash1 = loader.compute_actor_hash(&actor1);
        let hash2 = loader.compute_actor_hash(&actor2);

        // Different paths should produce different hashes
        assert_ne!(hash1, hash2);

        // Same actor should produce same hash
        let hash1_again = loader.compute_actor_hash(&actor1);
        assert_eq!(hash1, hash1_again);
    }

    #[tokio::test]
    async fn test_load_local_path_actor_builds_in_place() {
        let temp_cache_dir = TempDir::new().unwrap();
        let loader = ActorLoader::new(Some(temp_cache_dir.path().to_path_buf())).unwrap();

        let test_actor_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("test_actors")
            .join("buildable_simple");

        let actor = Actor {
            name: "test_in_place".to_string(),
            source: ActorSource::Path(hive_config::PathSource {
                path: test_actor_path.to_str().unwrap().to_string(),
                package: None,
            }),
            config: None,
            auto_spawn: false,
            required_spawn_with: vec![],
        };

        // Load the actor (should build in-place)
        let result = loader
            .load_single_actor(actor, "test:in-place".to_string(), vec![])
            .await;

        assert!(
            result.is_ok(),
            "Failed to load local path actor: {:?}",
            result.err()
        );

        let loaded_actor = result.unwrap();
        assert_eq!(loaded_actor.name, "test_in_place");
        assert_eq!(loaded_actor.id, "test:in-place");
        assert!(!loaded_actor.wasm.is_empty());
    }

    #[tokio::test]
    async fn test_git_vs_path_source_behavior() {
        let temp_cache_dir = TempDir::new().unwrap();
        let loader = ActorLoader::new(Some(temp_cache_dir.path().to_path_buf())).unwrap();

        let test_actor_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("test_actors")
            .join("buildable_simple");

        // Test path source (should build in-place)
        let path_actor = Actor {
            name: "path_actor".to_string(),
            source: ActorSource::Path(hive_config::PathSource {
                path: test_actor_path.to_str().unwrap().to_string(),
                package: None,
            }),
            config: None,
            auto_spawn: false,
            required_spawn_with: vec![],
        };

        // For this test, we just verify the different code paths are taken
        // We can't easily test git without setting up a real git repo
        let result = loader
            .load_single_actor(path_actor, "test:path".to_string(), vec![])
            .await;

        assert!(result.is_ok(), "Path actor should load successfully");
    }

    #[tokio::test]
    async fn test_hash_differs_for_path_vs_git_sources() {
        use url::Url;

        let loader = ActorLoader::new(None).unwrap();

        let path_actor = Actor {
            name: "test".to_string(),
            source: ActorSource::Path(hive_config::PathSource {
                path: "/some/path".to_string(),
                package: None,
            }),
            config: None,
            auto_spawn: false,
            required_spawn_with: vec![],
        };

        let git_actor = Actor {
            name: "test".to_string(),
            source: ActorSource::Git(hive_config::Repository {
                url: Url::parse("https://github.com/example/repo").unwrap(),
                git_ref: None,
                package: None,
            }),
            config: None,
            auto_spawn: false,
            required_spawn_with: vec![],
        };

        let path_hash = loader.compute_actor_hash(&path_actor);
        let git_hash = loader.compute_actor_hash(&git_actor);

        // Different source types should produce different hashes
        assert_ne!(path_hash, git_hash);
    }
}
