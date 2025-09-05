use snafu::{Location, Snafu};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tracing::info;

use crate::ExternalDependencyCache;
use crate::utils::compute_source_hash;
use wasmind_config::{Actor, ActorManifest, ActorSource};

/// Detailed information for conflicting sources error
#[derive(Debug)]
pub struct ConflictingSourcesData {
    logical_name: String,
    parent_actor_id: String,
    source1: String,
    path1: String,
    source2: String,
    path2: String,
}

impl std::fmt::Display for ConflictingSourcesData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Conflicting sources for dependency '{}' required by '{}'.\n  - {} via '{}'\n  - {} via '{}'",
            self.logical_name,
            self.parent_actor_id,
            self.source1,
            self.path1,
            self.source2,
            self.path2
        )
    }
}

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display(
        "Circular dependency detected while resolving '{actor_id}'. Resolution path: {path}"
    ))]
    CircularDependency {
        actor_id: String,
        path: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("{conflicting_sources}"))]
    ConflictingSources {
        conflicting_sources: Box<ConflictingSourcesData>,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display(
        "Failed to load manifest for actor '{logical_name}' from source '{source_path}': {message}"
    ))]
    ManifestLoad {
        logical_name: String,
        source_path: String,
        message: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display(
        "Actor '{logical_name}' at '{source_path}' is missing required Wasmind.toml manifest file. All actors must have a Wasmind.toml file that declares their actor_id."
    ))]
    MissingManifest {
        logical_name: String,
        source_path: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display(
        "Invalid user configuration: Actor '{logical_name}' is defined in [actors] but already exists in the dependency chain. Use [actor_overrides.{logical_name}] instead to override dependency configuration."
    ))]
    ActorConflictsWithDependency {
        logical_name: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display(
        "Invalid user configuration: Actor override '{logical_name}' specified in [actor_overrides] but no actor with this name exists in any dependency chain. Remove this override or add the actor to [actors] if you want to define a new actor."
    ))]
    OverrideForNonExistentDependency {
        logical_name: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display(
        "Invalid user configuration: Actor '{logical_name}' is defined in both [actors] and [actor_overrides]. Use only [actors.{logical_name}] to define user actors, or only [actor_overrides.{logical_name}] to override dependencies."
    ))]
    ActorAndOverrideConflict {
        logical_name: String,
        #[snafu(implicit)]
        location: Location,
    },
}

type Result<T> = std::result::Result<T, Error>;

/// Represents a resolved actor with all its configuration
#[derive(Debug, Clone)]
pub struct ResolvedActor {
    pub logical_name: String,
    pub actor_id: String,
    pub source: ActorSource,
    pub config: Option<toml::Table>,
    pub auto_spawn: bool,
    pub required_spawn_with: Vec<String>,
    pub is_dependency: bool,
}

/// Handles dependency resolution and validation
pub struct DependencyResolver {
    /// Maps logical names to their resolved actors
    resolved: HashMap<String, ResolvedActor>,
    /// Tracks the resolution path for circular dependency detection
    resolution_stack: Vec<String>,
    /// Cache for external dependencies
    external_cache: Arc<ExternalDependencyCache>,
    /// Path to the actor build cache directory
    cache_dir: Option<PathBuf>,
}

impl Default for DependencyResolver {
    fn default() -> Self {
        let temp_dir =
            tempfile::TempDir::new().expect("Failed to create temporary directory for actor cache");
        let external_cache = Arc::new(
            ExternalDependencyCache::new(temp_dir)
                .expect("Failed to create external dependency cache"),
        );
        Self {
            resolved: HashMap::new(),
            resolution_stack: Vec::new(),
            external_cache,
            cache_dir: None,
        }
    }
}

impl DependencyResolver {
    /// Constructor for ActorLoader to reuse its external cache
    pub fn new(external_cache: Arc<ExternalDependencyCache>, cache_dir: Option<PathBuf>) -> Self {
        Self {
            resolved: HashMap::new(),
            resolution_stack: Vec::new(),
            external_cache,
            cache_dir,
        }
    }

    /// Create a resolver with persistent caching using specified cache directory
    pub fn with_persistent_cache(cache_dir: PathBuf) -> crate::Result<Self> {
        use crate::{ExternalDependencyCache, TempDirSnafu};
        use snafu::ResultExt;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().context(TempDirSnafu)?;
        let external_cache = Arc::new(ExternalDependencyCache::new(temp_dir)?);
        Ok(Self::new(external_cache, Some(cache_dir)))
    }

    /// Resolve all actors and their dependencies
    pub async fn resolve_all(
        mut self,
        user_actors: Vec<Actor>,
        actor_overrides: Vec<wasmind_config::ActorOverride>,
    ) -> Result<HashMap<String, ResolvedActor>> {
        // Build maps for validation and resolution
        let mut user_actor_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut global_overrides: HashMap<String, Actor> = HashMap::new();
        for actor in user_actors.iter() {
            user_actor_names.insert(actor.name.clone());
            global_overrides.insert(actor.name.clone(), actor.clone());
        }

        let mut override_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut overrides_map: HashMap<String, wasmind_config::ActorOverride> = HashMap::new();
        for override_entry in actor_overrides {
            override_names.insert(override_entry.name.clone());
            overrides_map.insert(override_entry.name.clone(), override_entry);
        }

        // Validation 1: Check for conflicts between [actors] and [actor_overrides]
        for user_actor_name in &user_actor_names {
            if override_names.contains(user_actor_name) {
                return Err(Error::ActorAndOverrideConflict {
                    logical_name: user_actor_name.clone(),
                    location: snafu::Location::default(),
                });
            }
        }

        // First pass: resolve all user actors and collect all dependency names
        for actor in user_actors {
            self.resolve_actor_internal(actor, false, &global_overrides, &overrides_map)
                .await?;
        }

        Ok(self.resolved)
    }

    /// Internal method for resolving actors with full context
    fn resolve_actor_internal<'a>(
        &'a mut self,
        actor: Actor,
        is_dependency: bool,
        global_overrides: &'a HashMap<String, Actor>,
        actor_overrides: &'a HashMap<String, wasmind_config::ActorOverride>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
        Box::pin(async move {
            let logical_name = actor.name.clone();

            // Check if already resolved or if there are conflicts/cycles
            if self.is_already_resolved_with_matching_source(&logical_name, &actor)? {
                return Ok(());
            }

            self.check_for_circular_dependency(&logical_name)?;

            // Push to resolution stack
            self.resolution_stack.push(logical_name.clone());

            // Load and process the actor manifest
            let manifest = self.load_actor_manifest(&logical_name, &actor).await?;
            let actor_id = manifest.actor_id.clone();

            // Apply configuration overrides
            let (final_source, final_config, final_auto_spawn, final_required_spawn_with) = self
                .apply_configuration_overrides(
                    &logical_name,
                    actor,
                    &manifest,
                    global_overrides,
                    actor_overrides,
                );

            // Create and store the resolved actor
            let resolved_actor = ResolvedActor {
                logical_name: logical_name.clone(),
                actor_id,
                source: final_source.clone(),
                config: final_config,
                auto_spawn: final_auto_spawn,
                required_spawn_with: final_required_spawn_with,
                is_dependency,
            };
            self.resolved.insert(logical_name, resolved_actor);

            // Resolve all dependencies
            self.resolve_actor_dependencies(
                &final_source,
                &manifest,
                global_overrides,
                actor_overrides,
            )
            .await?;

            // Pop from resolution stack
            self.resolution_stack.pop();

            Ok(())
        })
    }

    /// Check if actor is already resolved with matching source
    fn is_already_resolved_with_matching_source(
        &self,
        logical_name: &str,
        actor: &Actor,
    ) -> Result<bool> {
        if let Some(existing) = self.resolved.get(logical_name) {
            // Validate that sources match
            if !sources_match(&actor.source, &existing.source) {
                let parent_actor_id = if self.resolution_stack.is_empty() {
                    "<root>".to_string()
                } else {
                    self.resolved
                        .get(&self.resolution_stack[0])
                        .map(|a| a.actor_id.clone())
                        .unwrap_or_else(|| "<unknown>".to_string())
                };

                return Err(Error::ConflictingSources {
                    conflicting_sources: Box::new(ConflictingSourcesData {
                        logical_name: logical_name.to_string(),
                        parent_actor_id,
                        source1: source_to_string(&existing.source),
                        path1: format!("(previously resolved as '{}')", existing.logical_name),
                        source2: source_to_string(&actor.source),
                        path2: self.resolution_stack.join(" -> "),
                    }),
                    location: snafu::Location::default(),
                });
            }
            // Already resolved with matching source
            return Ok(true);
        }
        Ok(false)
    }

    /// Check for circular dependencies
    fn check_for_circular_dependency(&self, logical_name: &str) -> Result<()> {
        if self.resolution_stack.contains(&logical_name.to_string()) {
            let mut path = self.resolution_stack.clone();
            path.push(logical_name.to_string());

            // Get the actor_id of the first actor in the cycle for better error message
            let actor_id = if let Some(first_actor) = self.resolved.get(&path[0]) {
                first_actor.actor_id.clone()
            } else {
                logical_name.to_string()
            };

            return Err(Error::CircularDependency {
                actor_id,
                path: path.join(" -> "),
                location: snafu::Location::default(),
            });
        }
        Ok(())
    }

    /// Load and validate actor manifest
    async fn load_actor_manifest(
        &self,
        logical_name: &str,
        actor: &Actor,
    ) -> Result<ActorManifest> {
        // Check if DEV_MODE is enabled
        let is_dev_mode = std::env::var("DEV_MODE").is_ok();

        // Try to load from build cache first (unless in dev mode)
        if !is_dev_mode {
            if let Some(manifest) = self.check_build_cache_for_manifest(actor).await? {
                info!(
                    "Manifest cache HIT: Found cached manifest for '{}' from {}",
                    logical_name,
                    source_to_string(&actor.source)
                );
                return Ok(manifest);
            }
        }

        // Load from source if not cached
        info!(
            "Manifest cache MISS: Loading manifest for '{}' from source {}",
            logical_name,
            source_to_string(&actor.source)
        );
        let manifest = load_manifest_for_source(&actor.source, &self.external_cache)
            .await
            .map_err(|e| Error::ManifestLoad {
                logical_name: logical_name.to_string(),
                source_path: source_to_string(&actor.source),
                message: e.to_string(),
                location: snafu::Location::default(),
            })?
            .ok_or_else(|| Error::MissingManifest {
                logical_name: logical_name.to_string(),
                source_path: source_to_string(&actor.source),
                location: snafu::Location::default(),
            })?;

        Ok(manifest)
    }

    /// Check if manifest is available in the build cache
    async fn check_build_cache_for_manifest(&self, actor: &Actor) -> Result<Option<ActorManifest>> {
        // Only check cache if cache_dir is configured
        let cache_dir = match &self.cache_dir {
            Some(dir) => dir,
            None => return Ok(None),
        };

        let source_hash = compute_source_hash(&actor.source);
        let cached_manifest_path = cache_dir.join(&source_hash).join("Wasmind.toml");

        if cached_manifest_path.exists() {
            info!(
                "Build cache HIT: Found manifest for source {} at {}",
                source_to_string(&actor.source),
                cached_manifest_path.display()
            );
            // Load the cached manifest
            let manifest_content =
                fs::read_to_string(&cached_manifest_path)
                    .await
                    .map_err(|e| Error::ManifestLoad {
                        logical_name: actor.name.clone(),
                        source_path: format!("cached: {}", cached_manifest_path.display()),
                        message: e.to_string(),
                        location: snafu::Location::default(),
                    })?;

            let manifest: ActorManifest =
                toml::from_str(&manifest_content).map_err(|e| Error::ManifestLoad {
                    logical_name: actor.name.clone(),
                    source_path: format!("cached: {}", cached_manifest_path.display()),
                    message: e.to_string(),
                    location: snafu::Location::default(),
                })?;

            return Ok(Some(manifest));
        }

        info!(
            "Build cache MISS: No manifest found for source {} at {}",
            source_to_string(&actor.source),
            cache_dir.join(&source_hash).display()
        );
        Ok(None)
    }

    /// Apply configuration overrides in the correct precedence order
    fn apply_configuration_overrides(
        &self,
        logical_name: &str,
        actor: Actor,
        manifest: &ActorManifest,
        global_overrides: &HashMap<String, Actor>,
        actor_overrides: &HashMap<String, wasmind_config::ActorOverride>,
    ) -> (ActorSource, Option<toml::Table>, bool, Vec<String>) {
        // Start with base configuration from actor and manifest
        let mut final_source = actor.source;
        let mut final_config = actor.config;
        let mut final_auto_spawn = actor.auto_spawn;
        let mut final_required_spawn_with = manifest.required_spawn_with.clone();

        // Apply global override if it exists
        if let Some(global_override) = global_overrides.get(logical_name) {
            self.apply_global_override(
                &mut final_source,
                &mut final_config,
                &mut final_auto_spawn,
                &mut final_required_spawn_with,
                global_override,
            );
        }

        // Apply actor override if it exists (this takes precedence over global overrides)
        if let Some(actor_override) = actor_overrides.get(logical_name) {
            self.apply_actor_override(
                &mut final_source,
                &mut final_config,
                &mut final_auto_spawn,
                &mut final_required_spawn_with,
                actor_override,
            );
        }

        (
            final_source,
            final_config,
            final_auto_spawn,
            final_required_spawn_with,
        )
    }

    /// Resolve all dependencies declared in the manifest
    fn resolve_actor_dependencies<'a>(
        &'a mut self,
        final_source: &'a ActorSource,
        manifest: &'a ActorManifest,
        global_overrides: &'a HashMap<String, Actor>,
        actor_overrides: &'a HashMap<String, wasmind_config::ActorOverride>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
        Box::pin(async move {
            for (dep_name, dep_config) in &manifest.dependencies {
                // Start with manifest defaults
                let dep_source = resolve_relative_source(final_source, dep_config.source.clone());
                let dep_config_table = dep_config.config.clone();
                let dep_auto_spawn = dep_config.auto_spawn.unwrap_or(false);

                let dep_actor = Actor {
                    name: dep_name.clone(),
                    source: dep_source,
                    config: dep_config_table,
                    auto_spawn: dep_auto_spawn,
                    required_spawn_with: vec![], // Dependencies don't have required_spawn_with from manifest
                };
                self.resolve_actor_internal(dep_actor, true, global_overrides, actor_overrides)
                    .await?;
            }
            Ok(())
        })
    }

    /// Apply global override configuration to an actor
    fn apply_global_override(
        &self,
        final_source: &mut ActorSource,
        final_config: &mut Option<toml::Table>,
        final_auto_spawn: &mut bool,
        final_required_spawn_with: &mut Vec<String>,
        global_override: &Actor,
    ) {
        *final_source = global_override.source.clone();

        // Merge configs instead of replacing
        *final_config = merge_config_option(final_config.as_ref(), global_override.config.as_ref());

        *final_auto_spawn = global_override.auto_spawn;

        // Use user-provided required_spawn_with if not empty, otherwise keep current
        if !global_override.required_spawn_with.is_empty() {
            *final_required_spawn_with = global_override.required_spawn_with.clone();
        }
    }

    /// Apply actor override configuration to an actor
    fn apply_actor_override(
        &self,
        final_source: &mut ActorSource,
        final_config: &mut Option<toml::Table>,
        final_auto_spawn: &mut bool,
        final_required_spawn_with: &mut Vec<String>,
        actor_override: &wasmind_config::ActorOverride,
    ) {
        if let Some(override_source) = &actor_override.source {
            *final_source = override_source.clone();
        }

        if let Some(override_config) = &actor_override.config {
            // Merge configs instead of replacing
            *final_config = merge_config_option(final_config.as_ref(), Some(override_config));
        }

        if let Some(override_auto_spawn) = actor_override.auto_spawn {
            *final_auto_spawn = override_auto_spawn;
        }

        if let Some(override_required_spawn_with) = &actor_override.required_spawn_with {
            *final_required_spawn_with = override_required_spawn_with.clone();
        }
    }
}

async fn load_manifest_from_git(
    git_source: &wasmind_config::Repository,
    external_cache: &ExternalDependencyCache,
) -> std::result::Result<Option<ActorManifest>, wasmind_config::Error> {
    // Use external cache to get the cached clone path
    let clone_path = external_cache
        .load_external_dependency(git_source)
        .await
        .map_err(|e| wasmind_config::Error::Io {
            source: std::io::Error::other(e.to_string()),
            location: snafu::Location::default(),
        })?;

    // Determine manifest path based on package
    let manifest_path = if let Some(sub_dir) = &git_source.sub_dir {
        // sub_dir is where we cd before building, so manifest is there
        clone_path.join(sub_dir).join("Wasmind.toml")
    } else {
        // Look in root for Wasmind.toml
        clone_path.join("Wasmind.toml")
    };

    // Check if manifest exists and load it
    if manifest_path.exists() {
        Ok(Some(ActorManifest::from_path(
            manifest_path.parent().unwrap(),
        )?))
    } else {
        Ok(None)
    }
}

async fn load_manifest_for_source(
    source: &ActorSource,
    external_cache: &ExternalDependencyCache,
) -> std::result::Result<Option<ActorManifest>, wasmind_config::Error> {
    match source {
        ActorSource::Path(path_source) => {
            let base_path = Path::new(&path_source.path);

            // For path sources, the path now points directly to where Wasmind.toml should be
            let manifest_dir = base_path.to_path_buf();

            // Check if Wasmind.toml exists in the determined directory
            let manifest_path = manifest_dir.join("Wasmind.toml");
            if manifest_path.exists() {
                Ok(Some(ActorManifest::from_path(&manifest_dir)?))
            } else {
                // No manifest found
                Ok(None)
            }
        }
        ActorSource::Git(git_source) => {
            // Use cached git clone to read Wasmind.toml
            load_manifest_from_git(git_source, external_cache).await
        }
    }
}

fn sources_match(source1: &ActorSource, source2: &ActorSource) -> bool {
    match (source1, source2) {
        (ActorSource::Path(p1), ActorSource::Path(p2)) => p1.path == p2.path,
        (ActorSource::Git(g1), ActorSource::Git(g2)) => {
            g1.git == g2.git && git_refs_match(&g1.git_ref, &g2.git_ref) && g1.sub_dir == g2.sub_dir
        }
        _ => false,
    }
}

fn git_refs_match(
    ref1: &Option<wasmind_config::GitRef>,
    ref2: &Option<wasmind_config::GitRef>,
) -> bool {
    match (ref1, ref2) {
        (None, None) => true,
        (Some(r1), Some(r2)) => match (r1, r2) {
            (wasmind_config::GitRef::Branch(b1), wasmind_config::GitRef::Branch(b2)) => b1 == b2,
            (wasmind_config::GitRef::Tag(t1), wasmind_config::GitRef::Tag(t2)) => t1 == t2,
            (wasmind_config::GitRef::Rev(r1), wasmind_config::GitRef::Rev(r2)) => r1 == r2,
            _ => false,
        },
        _ => false,
    }
}

/// Recursively merges two TOML tables, with values from `override_table` taking precedence
fn merge_toml_tables(base: &toml::Table, override_table: &toml::Table) -> toml::Table {
    let mut merged = base.clone();

    for (key, override_value) in override_table {
        match (merged.get(key), override_value) {
            // If both are tables, merge recursively
            (Some(toml::Value::Table(base_table)), toml::Value::Table(override_table)) => {
                let merged_subtable = merge_toml_tables(base_table, override_table);
                merged.insert(key.clone(), toml::Value::Table(merged_subtable));
            }
            // Otherwise, override takes precedence
            _ => {
                merged.insert(key.clone(), override_value.clone());
            }
        }
    }

    merged
}

/// Helper function to merge optional TOML configurations
fn merge_config_option(
    base: Option<&toml::Table>,
    override_config: Option<&toml::Table>,
) -> Option<toml::Table> {
    match (base, override_config) {
        (Some(base), Some(override_cfg)) => Some(merge_toml_tables(base, override_cfg)),
        (None, Some(override_cfg)) => Some(override_cfg.clone()),
        (base, None) => base.cloned(),
    }
}

fn source_to_string(source: &ActorSource) -> String {
    match source {
        ActorSource::Path(p) => format!("path: {}", p.path),
        ActorSource::Git(g) => format!("git: {}", g.git),
    }
}

fn resolve_relative_source(parent_source: &ActorSource, dep_source: ActorSource) -> ActorSource {
    match (&parent_source, &dep_source) {
        (ActorSource::Path(parent_path), ActorSource::Path(dep_path)) => {
            // If dependency path is relative, resolve it relative to parent
            let dep_path_buf = Path::new(&dep_path.path);
            if dep_path_buf.is_relative() {
                let parent_base_path = Path::new(&parent_path.path);

                // For path sources, the path points directly to the directory containing the manifest
                let parent_manifest_dir = parent_base_path.to_path_buf();

                // Resolve the dependency path relative to where the parent's manifest actually is
                let resolved = parent_manifest_dir.join(&dep_path.path);
                ActorSource::Path(wasmind_config::PathSource {
                    path: resolved.to_string_lossy().into_owned(),
                })
            } else {
                dep_source
            }
        }
        _ => dep_source, // Git sources or mixed types - return as is
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmind_config::PathSource;

    #[tokio::test]
    async fn test_missing_manifest_fails() {
        let actors = vec![Actor {
            name: "actor_a".to_string(),
            source: ActorSource::Path(PathSource {
                path: "test_data/simple_actor".to_string(),
            }),
            config: None,
            auto_spawn: false,
            required_spawn_with: vec![],
        }];

        let resolver = DependencyResolver::default();
        let result = resolver.resolve_all(actors, vec![]).await;

        // Should fail because Wasmind.toml is required for ALL actors
        assert!(result.is_err());
        let error = result.unwrap_err();
        let error_msg = error.to_string();

        // Should be a MissingManifest error
        assert!(error_msg.contains("missing required Wasmind.toml manifest file"));
        assert!(error_msg.contains("actor_a"));
    }

    // Circular dependency testing is handled in integration tests (tests/circular_dependency_test.rs)

    #[test]
    fn test_merge_toml_tables() {
        let mut base = toml::Table::new();
        base.insert(
            "model_name".to_string(),
            toml::Value::String("gpt-3.5".to_string()),
        );
        base.insert("require_tool_call".to_string(), toml::Value::Boolean(true));

        let mut system_prompt_base = toml::Table::new();
        let mut defaults_base = toml::Table::new();
        defaults_base.insert(
            "identity".to_string(),
            toml::Value::String("You are a helpful assistant".to_string()),
        );
        defaults_base.insert(
            "context".to_string(),
            toml::Value::String("Some context".to_string()),
        );
        system_prompt_base.insert("defaults".to_string(), toml::Value::Table(defaults_base));
        base.insert(
            "system_prompt".to_string(),
            toml::Value::Table(system_prompt_base),
        );

        let mut override_table = toml::Table::new();
        override_table.insert(
            "model_name".to_string(),
            toml::Value::String("gpt-4o".to_string()),
        );

        let merged = merge_toml_tables(&base, &override_table);

        // Check that model_name was overridden
        assert_eq!(
            merged.get("model_name").unwrap().as_str().unwrap(),
            "gpt-4o"
        );

        // Check that require_tool_call was preserved
        assert!(merged.get("require_tool_call").unwrap().as_bool().unwrap());

        // Check that system_prompt.defaults was preserved
        let system_prompt = merged.get("system_prompt").unwrap().as_table().unwrap();
        let defaults = system_prompt.get("defaults").unwrap().as_table().unwrap();
        assert_eq!(
            defaults.get("identity").unwrap().as_str().unwrap(),
            "You are a helpful assistant"
        );
        assert_eq!(
            defaults.get("context").unwrap().as_str().unwrap(),
            "Some context"
        );
    }

    #[test]
    fn test_merge_toml_tables_nested_override() {
        let mut base = toml::Table::new();
        base.insert(
            "model_name".to_string(),
            toml::Value::String("gpt-3.5".to_string()),
        );

        let mut system_prompt_base = toml::Table::new();
        let mut defaults_base = toml::Table::new();
        defaults_base.insert(
            "identity".to_string(),
            toml::Value::String("You are a helpful assistant".to_string()),
        );
        defaults_base.insert(
            "context".to_string(),
            toml::Value::String("Some context".to_string()),
        );
        system_prompt_base.insert("defaults".to_string(), toml::Value::Table(defaults_base));
        base.insert(
            "system_prompt".to_string(),
            toml::Value::Table(system_prompt_base),
        );

        let mut override_table = toml::Table::new();
        let mut system_prompt_override = toml::Table::new();
        let mut defaults_override = toml::Table::new();
        defaults_override.insert(
            "identity".to_string(),
            toml::Value::String("You are a specialized assistant".to_string()),
        );
        system_prompt_override.insert(
            "defaults".to_string(),
            toml::Value::Table(defaults_override),
        );
        override_table.insert(
            "system_prompt".to_string(),
            toml::Value::Table(system_prompt_override),
        );

        let merged = merge_toml_tables(&base, &override_table);

        // Check that model_name was preserved
        assert_eq!(
            merged.get("model_name").unwrap().as_str().unwrap(),
            "gpt-3.5"
        );

        // Check that system_prompt.defaults.identity was overridden
        let system_prompt = merged.get("system_prompt").unwrap().as_table().unwrap();
        let defaults = system_prompt.get("defaults").unwrap().as_table().unwrap();
        assert_eq!(
            defaults.get("identity").unwrap().as_str().unwrap(),
            "You are a specialized assistant"
        );

        // Check that system_prompt.defaults.context was preserved
        assert_eq!(
            defaults.get("context").unwrap().as_str().unwrap(),
            "Some context"
        );
    }

    #[tokio::test]
    async fn test_actor_override_config_merging_integration() {
        use tempfile::TempDir;

        // Create a temporary test directory structure
        let temp_dir = TempDir::new().unwrap();
        let test_root = temp_dir.path();

        // Create dependency actor directory with Wasmind.toml
        let dep_actor_dir = test_root.join("dep_actor");
        std::fs::create_dir_all(&dep_actor_dir).unwrap();

        let wasmind_toml_content = r#"
actor_id = "test:dependency_actor"
required_spawn_with = []

[dependencies.test_assistant]
source = { path = "../assistant_actor" }

[dependencies.test_assistant.config]
model_name = "gpt-3.5-turbo"
require_tool_call = true

[dependencies.test_assistant.config.system_prompt.defaults]
identity = "You are a helpful assistant"
context = "You operate in a test environment"
guidelines = "Follow all instructions carefully"
"#;
        std::fs::write(dep_actor_dir.join("Wasmind.toml"), wasmind_toml_content).unwrap();

        // Create assistant actor directory with Wasmind.toml
        let assistant_actor_dir = test_root.join("assistant_actor");
        std::fs::create_dir_all(&assistant_actor_dir).unwrap();

        let assistant_wasmind_toml = r#"
actor_id = "test:assistant"
required_spawn_with = []
"#;
        std::fs::write(
            assistant_actor_dir.join("Wasmind.toml"),
            assistant_wasmind_toml,
        )
        .unwrap();

        // Create user actor that depends on the dependency
        let user_actors = vec![Actor {
            name: "dependency_actor".to_string(),
            source: ActorSource::Path(wasmind_config::PathSource {
                path: dep_actor_dir.to_string_lossy().to_string(),
            }),
            config: None,
            auto_spawn: false,
            required_spawn_with: vec![],
        }];

        // Create actor override that only specifies model_name
        let actor_overrides = vec![wasmind_config::ActorOverride {
            name: "test_assistant".to_string(),
            source: None,
            config: Some({
                let mut override_config = toml::Table::new();
                override_config.insert(
                    "model_name".to_string(),
                    toml::Value::String("gpt-4o".to_string()),
                );
                override_config
            }),
            auto_spawn: None,
            required_spawn_with: None,
        }];

        // Resolve all actors
        let resolver = DependencyResolver::default();
        let resolved = resolver
            .resolve_all(user_actors, actor_overrides)
            .await
            .unwrap();

        // Verify that test_assistant was resolved with merged config
        let assistant = resolved.get("test_assistant").unwrap();
        let config = assistant.config.as_ref().unwrap();

        // Check that model_name was overridden
        assert_eq!(
            config.get("model_name").unwrap().as_str().unwrap(),
            "gpt-4o"
        );

        // Check that require_tool_call was preserved from dependency config
        assert!(config.get("require_tool_call").unwrap().as_bool().unwrap());

        // Check that system_prompt.defaults were preserved
        let system_prompt = config.get("system_prompt").unwrap().as_table().unwrap();
        let defaults = system_prompt.get("defaults").unwrap().as_table().unwrap();
        assert_eq!(
            defaults.get("identity").unwrap().as_str().unwrap(),
            "You are a helpful assistant"
        );
        assert_eq!(
            defaults.get("context").unwrap().as_str().unwrap(),
            "You operate in a test environment"
        );
        assert_eq!(
            defaults.get("guidelines").unwrap().as_str().unwrap(),
            "Follow all instructions carefully"
        );

        // Verify that the assistant has the correct actor_id
        assert_eq!(assistant.actor_id, "test:assistant");
        assert_eq!(assistant.logical_name, "test_assistant");
        assert!(assistant.is_dependency);
    }
}
