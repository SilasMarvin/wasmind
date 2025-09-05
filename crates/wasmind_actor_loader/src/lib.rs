//! # Wasmind Actor Loader
//!
//! Dynamic loading and dependency resolution system for Wasmind WASM actor components.
//! This crate handles downloading, building, caching, and loading actors from various
//! sources (local paths, Git repositories, etc.).

pub mod dependency_resolver;
pub mod utils;

use snafu::{Location, ResultExt, Snafu, ensure, location};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tokio::fs;
use tokio::process::Command;
use tracing::{info, warn};

use crate::utils::{compute_git_source_hash, compute_source_hash};
use wasmind_config::{Actor, ActorSource, GitRef};

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
    Toml {
        source: toml::de::Error,
        text: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Error getting Cargo.toml {key}"))]
    CargoToml {
        key: String,
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

    #[snafu(transparent)]
    Config { source: wasmind_config::Error },

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

    #[snafu(display(
        "Actor '{actor_name}' has an unexpected WASM module file instead of a WebAssembly Component at '{wasm_path}'. This is a rare issue that may indicate mixed build tools or a cargo-component bug. Please file an issue with details about your build environment."
    ))]
    WasmModuleInsteadOfComponent {
        actor_name: String,
        wasm_path: String,
        #[snafu(implicit)]
        location: Location,
    },
}

type Result<T> = std::result::Result<T, Error>;

/// Cache for external dependencies (git repos, etc.) to avoid duplicate fetching
pub struct ExternalDependencyCache {
    temp_dir: TempDir,
    cache: Mutex<HashMap<String, PathBuf>>,
}

impl ExternalDependencyCache {
    /// Create a new external dependency cache with a temporary directory
    pub fn new(temp_dir: TempDir) -> Result<Self> {
        Ok(Self {
            temp_dir,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// Load external dependency (git repository) and return the path to the cloned repo
    async fn load_external_dependency(
        &self,
        git_source: &wasmind_config::Repository,
    ) -> Result<PathBuf> {
        let cache_key = compute_git_source_hash(git_source);

        // Check cache first
        {
            let cache = self.cache.lock().unwrap();
            if let Some(existing_path) = cache.get(&cache_key)
                && existing_path.exists()
            {
                info!(
                    "Git cache HIT: Using cached repository {} at {}",
                    git_source.git,
                    existing_path.display()
                );
                return Ok(existing_path.clone());
            }
        }

        info!("Git cache MISS: Cloning repository {}", git_source.git);

        // Clone and cache
        let clone_path = self.temp_dir.path().join(&cache_key);
        self.clone_git_source(git_source, &clone_path).await?;

        // Update cache
        {
            let mut cache = self.cache.lock().unwrap();
            cache.insert(cache_key, clone_path.clone());
        }

        Ok(clone_path)
    }

    /// Clone a git source to the specified path
    async fn clone_git_source(
        &self,
        git_source: &wasmind_config::Repository,
        dest: &Path,
    ) -> Result<()> {
        info!("Cloning git source {} to cache", git_source.git);

        let mut cmd = Command::new("git");
        cmd.arg("clone").arg("--depth").arg("1");

        // Add git ref if specified
        if let Some(git_ref) = &git_source.git_ref {
            match git_ref {
                GitRef::Branch(branch) => {
                    cmd.arg("-b").arg(branch);
                }
                GitRef::Tag(tag) => {
                    cmd.arg("-b").arg(tag);
                }
                GitRef::Rev(_rev) => {
                    // Note: Specific revision checkout requires two-step process
                    // Clone default branch first, then checkout specific commit
                }
            }
        }

        cmd.arg(git_source.git.as_str()).arg(dest);

        let output = cmd.output().await.context(CommandSnafu)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::CommandFailed {
                actor_name: "<git-clone>".to_string(),
                status: output.status,
                stderr: stderr.into_owned(),
                location: location!(),
            });
        }

        // Handle specific revision checkout if needed
        if let Some(GitRef::Rev(rev)) = &git_source.git_ref {
            let mut checkout_cmd = Command::new("git");
            checkout_cmd.current_dir(dest).arg("checkout").arg(rev);

            let checkout_output = checkout_cmd.output().await.context(CommandSnafu)?;
            if !checkout_output.status.success() {
                let stderr = String::from_utf8_lossy(&checkout_output.stderr);
                return Err(Error::CommandFailed {
                    actor_name: "<git-checkout>".to_string(),
                    status: checkout_output.status,
                    stderr: stderr.into_owned(),
                    location: location!(),
                });
            }
        }

        Ok(())
    }
}

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
    external_cache: Arc<ExternalDependencyCache>,
}

impl ActorLoader {
    pub fn new(cache_dir: Option<PathBuf>) -> Result<Self> {
        let cache_dir = match cache_dir {
            Some(cache_dir) => cache_dir,
            None => wasmind_config::get_cache_dir()?.join("actors"),
        };

        Ok(Self {
            cache_dir,
            external_cache: Arc::new(ExternalDependencyCache::new(
                TempDir::new().context(TempDirSnafu)?,
            )?),
        })
    }

    pub async fn load_actors(
        &self,
        actors: Vec<Actor>,
        actor_overrides: Vec<wasmind_config::ActorOverride>,
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
        let resolver = dependency_resolver::DependencyResolver::with_cache(
            self.external_cache.clone(),
            Some(self.cache_dir.clone()),
        );
        let resolved_actors = resolver
            .resolve_all(actors, actor_overrides)
            .await
            .context(DependencyResolutionSnafu)?;

        // Phase 2: Load all resolved actors
        #[cfg(feature = "progress-output")]
        println!("Loading {} actors...", resolved_actors.len());

        let mut loaded_actors = vec![];
        for (logical_name, resolved) in resolved_actors {
            let actor = Actor {
                name: logical_name,
                source: resolved.source,
                config: resolved.config,
                auto_spawn: resolved.auto_spawn,
                required_spawn_with: resolved.required_spawn_with.clone(),
            };
            let actor = self
                .load_single_actor(
                    actor,
                    resolved.actor_id.clone(),
                    resolved.required_spawn_with,
                )
                .await?;
            loaded_actors.push(actor);
        }

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

        // For git sources, sub_dir tells us where to cd before building
        // For path sources, the path already points to the build directory

        let (build_path, wasm_path, version) = match &actor.source {
            ActorSource::Path(path_source) => {
                // For local paths, the path points directly to the build directory
                info!("Building local actor: {}", path_source.path);
                let build_path = Path::new(&path_source.path);

                // Verify build path exists
                ensure!(
                    build_path.exists(),
                    InvalidPathSnafu {
                        actor_name: actor.name.clone(),
                        path: path_source.path.clone()
                    }
                );

                // Build in the specified directory
                // Search backwards up to the root. This is somewhat hacky but path should really
                // only be used for local development
                let wasm_path = self
                    .build_actor(build_path, Path::new("/"), &actor.name)
                    .await?;

                // Get version from build directory
                let version = self.get_actor_version(build_path).await?;

                (build_path.to_path_buf(), wasm_path, version)
            }
            ActorSource::Git(repository) => {
                // Use external cache to get the cloned repository
                info!("Using cached Git actor: {}", repository.git);
                let cached_repo_path = self
                    .external_cache
                    .load_external_dependency(repository)
                    .await?;

                // Determine build path based on sub_dir
                let build_path = if let Some(sub_dir) = &repository.sub_dir {
                    // sub_dir is where we cd before building
                    cached_repo_path.join(sub_dir)
                } else {
                    // For single actors, use the repo root
                    cached_repo_path.clone()
                };

                // Build using the build directory, but search from repo root
                let wasm_path = self
                    .build_actor(&build_path, &cached_repo_path, &actor.name)
                    .await?;

                // Get version from build directory
                let version = self.get_actor_version(&build_path).await?;

                (build_path, wasm_path, version)
            }
        };

        // Read the built wasm
        let wasm = fs::read(&wasm_path).await.context(IoSnafu {
            path: Some(wasm_path.clone()),
        })?;

        // Validate that this is a WebAssembly Component, not a WASM module
        if wasm.len() >= 8 {
            let magic = &wasm[0..8];
            // WASM module magic bytes: 00 61 73 6d 01 00 00 00
            if magic == [0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00] {
                return Err(Error::WasmModuleInsteadOfComponent {
                    actor_name: actor.name.clone(),
                    wasm_path: wasm_path.display().to_string(),
                    location: location!(),
                });
            }
        }

        // Cache the built actor (skip in dev mode)
        if !is_dev_mode {
            self.cache_actor(&actor, &actor_id, &version, &wasm, &build_path)
                .await?;
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

    async fn check_cache(
        &self,
        actor: &Actor,
        actor_id: &str,
        required_spawn_with: &[String],
    ) -> Result<Option<LoadedActor>> {
        let source_hash = compute_source_hash(&actor.source);
        let cache_path = self.cache_dir.join(&source_hash);
        let metadata_path = cache_path.join("metadata.json");
        let wasm_path = cache_path.join("actor.wasm");

        if metadata_path.exists() && wasm_path.exists() {
            info!(
                "Cache HIT: Found cached actor '{}' at {}",
                actor.name,
                cache_path.display()
            );
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

        info!(
            "Cache MISS: No cached actor '{}' found at {}",
            actor.name,
            cache_path.display()
        );
        Ok(None)
    }

    async fn cache_actor(
        &self,
        actor: &Actor,
        actor_id: &str,
        version: &str,
        wasm: &[u8],
        manifest_dir: &Path,
    ) -> Result<()> {
        let source_hash = compute_source_hash(&actor.source);
        let cache_path = self.cache_dir.join(&source_hash);

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
            "source_hash": &source_hash,
            "cached_at": chrono::Utc::now().to_rfc3339(),
        });
        let metadata_path = cache_path.join("metadata.json");
        fs::write(&metadata_path, metadata.to_string())
            .await
            .context(IoSnafu {
                path: Some(metadata_path),
            })?;

        // Copy Wasmind.toml to cache
        let source_manifest_path = manifest_dir.join("Wasmind.toml");
        if source_manifest_path.exists() {
            let manifest_content =
                fs::read_to_string(&source_manifest_path)
                    .await
                    .context(IoSnafu {
                        path: Some(source_manifest_path.clone()),
                    })?;
            let cached_manifest_path = cache_path.join("Wasmind.toml");
            fs::write(&cached_manifest_path, manifest_content)
                .await
                .context(IoSnafu {
                    path: Some(cached_manifest_path),
                })?;
        }

        info!(
            "Cached actor '{}' version {} at {}",
            actor.name,
            version,
            cache_path.display()
        );
        Ok(())
    }

    async fn build_actor(
        &self,
        build_path: &Path,
        search_root: &Path,
        actor_name: &str,
    ) -> Result<PathBuf> {
        info!("Building actor at {:?}", build_path);

        // Build in the specified build directory
        let mut cmd = Command::new("cargo-component");
        cmd.current_dir(build_path).arg("build").arg("--release");
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

        // Get the expected WASM file name from the build directory's Cargo.toml
        let expected_name = self.get_expected_wasm_name(build_path)?;

        // Search for target directory with our WASM file, starting from build_path and moving up to search_root
        self.find_target_with_wasm(build_path, &expected_name, search_root, actor_name)
            .await
    }

    /// Search for target directory containing the expected WASM file
    ///
    /// Searches upward from `build_path` toward `search_root` for a target directory
    /// containing the expected WASM file. Uses a "first found wins" strategy - returns
    /// the first matching file found, preferring locations closer to the build path.
    ///
    /// # Search Strategy
    /// - Starts at `build_path`
    /// - Checks each parent directory for `target/wasm32-wasip1/release/{expected_wasm}`
    /// - Returns immediately upon finding the first match
    /// - Stops searching when reaching `search_root`
    async fn find_target_with_wasm(
        &self,
        build_path: &Path,
        expected_wasm: &str,
        search_root: &Path,
        actor_name: &str,
    ) -> Result<PathBuf> {
        let mut current_dir = build_path;

        loop {
            let target_dir = current_dir
                .join("target")
                .join("wasm32-wasip1")
                .join("release");

            let wasm_path = target_dir.join(expected_wasm);

            if wasm_path.exists() {
                info!("Found WASM file at: {:?}", wasm_path);
                return Ok(wasm_path);
            }

            // Move up one directory
            if let Some(parent) = current_dir.parent() {
                // Stop if we've reached the search root
                if parent < search_root {
                    break;
                }
                current_dir = parent;
            } else {
                break;
            }
        }

        // If we get here, we didn't find the WASM file
        Err(Error::WasmNotFound {
            actor_name: actor_name.to_string(),
            expected_wasm: expected_wasm.to_string(),
            target_dir: format!("searched from {build_path:?} up to {search_root:?}"),
            location: location!(),
        })
    }

    fn get_expected_wasm_name(&self, build_dir: &Path) -> Result<String> {
        let manifest_path = build_dir.join("Cargo.toml");

        let file_contents = std::fs::read_to_string(&manifest_path).context(IoSnafu {
            path: Some(manifest_path.to_path_buf()),
        })?;
        let toml: toml::Value = toml::from_str(&file_contents).context(TomlSnafu {
            text: file_contents,
        })?;
        let package_name = toml["package"]["name"].as_str().ok_or(Error::CargoToml {
            location: location!(),
            key: "package.name".to_string(),
        })?;

        Ok(format!("{}.wasm", package_name.replace('-', "_")))
    }

    async fn get_actor_version(&self, build_path: &Path) -> Result<String> {
        // Get version from the build directory's Cargo.toml
        let manifest_path = build_path.join("Cargo.toml");

        let file_contents = std::fs::read_to_string(&manifest_path).context(IoSnafu {
            path: Some(manifest_path.to_path_buf()),
        })?;
        let toml: toml::Value = toml::from_str(&file_contents).context(TomlSnafu {
            text: file_contents,
        })?;
        let package_version = toml["package"]["version"]
            .as_str()
            .ok_or(Error::CargoToml {
                location: location!(),
                key: "package.version".to_string(),
            })?;

        Ok(package_version.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_build_successful_actor() {
        let loader = ActorLoader::new(None).unwrap();
        let test_actor_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("test_actors")
            .join("buildable_simple");

        // Should build successfully
        let result = loader
            .build_actor(&test_actor_path, &test_actor_path, "buildable_simple")
            .await;
        assert!(
            result.is_ok(),
            "Failed to build buildable_simple: {:?}",
            result.err()
        );

        let wasm_path = result.unwrap();
        assert!(
            wasm_path.exists(),
            "Built wasm file should exist at {wasm_path:?}"
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
            .build_actor(&test_actor_path, &test_actor_path, "buildable_fail")
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
            other => panic!("Expected CommandFailed error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_get_actor_version_success() {
        let loader = ActorLoader::new(None).unwrap();
        let test_actor_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("test_actors")
            .join("buildable_simple");

        let result = loader.get_actor_version(&test_actor_path).await;
        assert!(result.is_ok(), "Failed to get version: {:?}", result.err());
        assert_eq!(result.unwrap(), "0.1.0");
    }

    #[tokio::test]
    async fn test_get_actor_version_nonexistent() {
        let loader = ActorLoader::new(None).unwrap();
        let nonexistent_path = PathBuf::from("/nonexistent/path");

        let result = loader.get_actor_version(&nonexistent_path).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cache_and_load_actor() {
        let temp_dir = TempDir::new().unwrap();
        let loader = ActorLoader::new(Some(temp_dir.path().to_path_buf())).unwrap();

        let test_wasm = b"fake wasm content";
        let actor = Actor {
            name: "test_actor".to_string(),
            source: ActorSource::Path(wasmind_config::PathSource {
                path: "/test/path".to_string(),
            }),
            config: None,
            auto_spawn: false,
            required_spawn_with: vec![],
        };

        // Create a fake manifest directory
        let manifest_dir = temp_dir.path().join("manifest_dir");
        std::fs::create_dir_all(&manifest_dir).unwrap();

        // Cache the actor
        loader
            .cache_actor(&actor, "test:actor", "1.0.0", test_wasm, &manifest_dir)
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
            .build_actor(&test_actor_path, &test_actor_path, "buildable_simple")
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
        let _loader = ActorLoader::new(None).unwrap();

        let actor1 = Actor {
            name: "test".to_string(),
            source: ActorSource::Path(wasmind_config::PathSource {
                path: "/path1".to_string(),
            }),
            config: None,
            auto_spawn: false,
            required_spawn_with: vec![],
        };

        let actor2 = Actor {
            name: "test".to_string(),
            source: ActorSource::Path(wasmind_config::PathSource {
                path: "/path2".to_string(),
            }),
            config: None,
            auto_spawn: false,
            required_spawn_with: vec![],
        };

        let hash1 = compute_source_hash(&actor1.source);
        let hash2 = compute_source_hash(&actor2.source);

        // Different paths should produce different hashes
        assert_ne!(hash1, hash2);

        // Same actor should produce same hash
        let hash1_again = compute_source_hash(&actor1.source);
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
            source: ActorSource::Path(wasmind_config::PathSource {
                path: test_actor_path.to_str().unwrap().to_string(),
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
            source: ActorSource::Path(wasmind_config::PathSource {
                path: test_actor_path.to_str().unwrap().to_string(),
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

        let _loader = ActorLoader::new(None).unwrap();

        let path_actor = Actor {
            name: "test".to_string(),
            source: ActorSource::Path(wasmind_config::PathSource {
                path: "/some/path".to_string(),
            }),
            config: None,
            auto_spawn: false,
            required_spawn_with: vec![],
        };

        let git_actor = Actor {
            name: "test".to_string(),
            source: ActorSource::Git(wasmind_config::Repository {
                git: Url::parse("https://github.com/example/repo").unwrap(),
                git_ref: None,
                sub_dir: None,
            }),
            config: None,
            auto_spawn: false,
            required_spawn_with: vec![],
        };

        let path_hash = compute_source_hash(&path_actor.source);
        let git_hash = compute_source_hash(&git_actor.source);

        // Different source types should produce different hashes
        assert_ne!(path_hash, git_hash);
    }

    #[tokio::test]
    async fn test_multiple_target_directories_prefers_closest() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();

        // Create nested structure:
        // root/
        // ├── target/wasm32-wasip1/release/my_actor.wasm  (farther from build)
        // └── deep/
        //     ├── package/   <- build_path
        //     └── target/wasm32-wasip1/release/my_actor.wasm  (closer to build)

        let workspace_target = root.join("target/wasm32-wasip1/release");
        let deep_target = root.join("deep/target/wasm32-wasip1/release");
        let build_path = root.join("deep/package");

        fs::create_dir_all(&workspace_target).unwrap();
        fs::create_dir_all(&deep_target).unwrap();
        fs::create_dir_all(&build_path).unwrap();

        // Create WASM files in both locations
        fs::write(workspace_target.join("my_actor.wasm"), b"workspace_build").unwrap();
        fs::write(deep_target.join("my_actor.wasm"), b"package_build").unwrap();

        let loader = ActorLoader::new(None).unwrap();
        let result = loader
            .find_target_with_wasm(&build_path, "my_actor.wasm", root, "test_actor")
            .await;

        assert!(result.is_ok(), "Should find WASM file in nested structure");
        let wasm_path = result.unwrap();

        // Verify it found the closer one (deep/target, not root/target)
        assert!(
            wasm_path.to_string_lossy().contains("deep/target"),
            "Should prefer closer target directory, found: {}",
            wasm_path.display()
        );
        assert!(
            !wasm_path.to_string_lossy().contains("root/target"),
            "Should not use farther target directory"
        );
    }
}
