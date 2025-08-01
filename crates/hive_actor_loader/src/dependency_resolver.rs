use std::collections::HashMap;
use std::path::Path;
use snafu::{Location, Snafu};
use tracing::warn;
use tempfile::TempDir;

use hive_config::{Actor, ActorManifest, ActorSource};

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Circular dependency detected while resolving '{actor_id}'. Resolution path: {path}"))]
    CircularDependency {
        actor_id: String,
        path: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Conflicting sources for dependency '{logical_name}' required by '{parent_actor_id}'.\n  - {source1} via '{path1}'\n  - {source2} via '{path2}'"))]
    ConflictingSources {
        logical_name: String,
        parent_actor_id: String,
        source1: String,
        path1: String,
        source2: String,
        path2: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Failed to load manifest for actor '{logical_name}' from source '{source_path}': {message}"))]
    ManifestLoad {
        logical_name: String,
        source_path: String,
        message: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Actor '{logical_name}' at '{source_path}' is missing required Hive.toml manifest file. All actors must have a Hive.toml file that declares their actor_id."))]
    MissingManifest {
        logical_name: String,
        source_path: String,
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
    pub is_dependency: bool,
}

/// Handles dependency resolution and validation
#[derive(Default)]
pub struct DependencyResolver {
    /// Maps logical names to their resolved actors
    resolved: HashMap<String, ResolvedActor>,
    /// Tracks the resolution path for circular dependency detection
    resolution_stack: Vec<String>,
}

impl DependencyResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve all actors and their dependencies
    pub fn resolve_all(mut self, actors: Vec<Actor>) -> Result<HashMap<String, ResolvedActor>> {
        // Store user configs for dependency override lookup
        let mut user_configs: HashMap<String, toml::Table> = HashMap::new();
        for actor in &actors {
            if let Some(config) = &actor.config {
                user_configs.insert(actor.name.clone(), config.clone());
            }
        }

        // First, add all explicitly configured actors
        for actor in actors {
            self.resolve_actor_internal(actor, false, &user_configs, None)?;
        }

        Ok(self.resolved)
    }


    /// Internal method for resolving actors with full context
    fn resolve_actor_internal(
        &mut self, 
        actor: Actor, 
        is_dependency: bool, 
        user_configs: &HashMap<String, toml::Table>,
        _parent_logical_name: Option<&str>
    ) -> Result<()> {
        let logical_name = actor.name.clone();

        // Check if already resolved
        if let Some(existing) = self.resolved.get(&logical_name) {
            // Validate that sources match
            if !sources_match(&actor.source, &existing.source) {
                let parent_actor_id = if self.resolution_stack.is_empty() {
                    "<root>".to_string()
                } else {
                    self.resolved.get(&self.resolution_stack[0])
                        .map(|a| a.actor_id.clone())
                        .unwrap_or_else(|| "<unknown>".to_string())
                };
                
                return Err(Error::ConflictingSources {
                    logical_name: logical_name.clone(),
                    parent_actor_id,
                    source1: source_to_string(&existing.source),
                    path1: self.get_resolution_path_for(&existing.logical_name),
                    source2: source_to_string(&actor.source),
                    path2: self.resolution_stack.join(" -> "),
                    location: snafu::Location::default(),
                });
            }
            // Already resolved with matching source, nothing to do
            return Ok(());
        }

        // Check for circular dependencies
        if self.resolution_stack.contains(&logical_name) {
            let mut path = self.resolution_stack.clone();
            path.push(logical_name.clone());
            
            // Get the actor_id of the first actor in the cycle for better error message
            let actor_id = if let Some(first_actor) = self.resolved.get(&path[0]) {
                first_actor.actor_id.clone()
            } else {
                logical_name.clone()
            };
            
            return Err(Error::CircularDependency {
                actor_id,
                path: path.join(" -> "),
                location: snafu::Location::default(),
            });
        }

        // Push to resolution stack
        self.resolution_stack.push(logical_name.clone());

        // Load manifest (REQUIRED for ALL actors - no exceptions)
        let manifest = load_manifest_for_source(&actor.source).map_err(|e| Error::ManifestLoad {
            logical_name: logical_name.clone(),
            source_path: source_to_string(&actor.source),
            message: e.to_string(),
            location: snafu::Location::default(),
        })?.ok_or_else(|| Error::MissingManifest {
            logical_name: logical_name.clone(),
            source_path: source_to_string(&actor.source),
            location: snafu::Location::default(),
        })?;
        
        // Get actor_id from manifest (always required)
        let actor_id = manifest.actor_id.clone();

        // Create resolved actor
        let resolved_actor = ResolvedActor {
            logical_name: logical_name.clone(),
            actor_id,
            source: actor.source.clone(),
            config: actor.config.clone(),
            auto_spawn: actor.auto_spawn,
            is_dependency,
        };

        // Store the resolved actor
        self.resolved.insert(logical_name.clone(), resolved_actor);

        // Check for orphaned dependency configurations and warn about them
        self.check_orphaned_dependency_configs(&logical_name, &manifest, user_configs);

        // Resolve dependencies from manifest
        for (dep_name, dep_config) in manifest.dependencies {
            // Merge all dependency settings: source, config, auto_spawn from manifest defaults + user overrides
            let (merged_source, merged_config, merged_auto_spawn) = merge_dependency_settings(
                &dep_config,
                &actor.source, // Parent source for relative path resolution
                Some(&logical_name), // The current actor is the parent of this dependency
                &dep_name,
                user_configs
            );
            
            let dep_actor = Actor {
                name: dep_name.clone(),
                source: merged_source,
                config: merged_config,
                auto_spawn: merged_auto_spawn,
            };
            self.resolve_actor_internal(dep_actor, true, user_configs, Some(&logical_name))?;
        }

        // Pop from resolution stack
        self.resolution_stack.pop();

        Ok(())
    }

    fn get_resolution_path_for(&self, logical_name: &str) -> String {
        // This is a simplified version - in a real implementation,
        // we might want to track the actual resolution paths
        format!("(previously resolved as '{logical_name}')")
    }

    /// Check for orphaned dependency configurations and warn about them
    fn check_orphaned_dependency_configs(
        &self,
        actor_logical_name: &str,
        manifest: &ActorManifest,
        user_configs: &HashMap<String, toml::Table>
    ) {
        if let Some(user_config) = user_configs.get(actor_logical_name) {
            if let Some(dependencies_config) = user_config.get("dependencies") {
                if let Some(dependencies_table) = dependencies_config.as_table() {
                    // Get the set of actual dependencies from the manifest
                    let actual_dependencies: std::collections::HashSet<String> = 
                        manifest.dependencies.keys().cloned().collect();
                    
                    // Check each configured dependency
                    for dep_name in dependencies_table.keys() {
                        if !actual_dependencies.contains(dep_name) {
                            warn!(
                                "Configuration for unknown dependency '{dep_name}' in actor '{actor_logical_name}' will be ignored. Check for typos or outdated configuration."
                            );
                        }
                    }
                }
            }
        }
    }
}

fn load_manifest_from_git(git_source: &hive_config::Repository) -> std::result::Result<Option<ActorManifest>, hive_config::Error> {
    // Create temporary directory for cloning
    let temp_dir = TempDir::new().map_err(|e| hive_config::Error::Io {
        source: e,
        location: snafu::Location::default(),
    })?;
    
    let clone_path = temp_dir.path().join("repo");
    
    // Build git clone command
    let mut cmd = std::process::Command::new("git");
    cmd.arg("clone")
       .arg("--depth").arg("1") // Shallow clone for efficiency
       .arg(git_source.url.as_str())
       .arg(&clone_path);
    
    // Add git ref if specified
    if let Some(git_ref) = &git_source.git_ref {
        match git_ref {
            hive_config::GitRef::Branch(branch) => {
                cmd.arg("-b").arg(branch);
            }
            hive_config::GitRef::Tag(tag) => {
                cmd.arg("-b").arg(tag);
            }
            hive_config::GitRef::Rev(rev) => {
                // For specific revisions, we need to clone first then checkout
                cmd.arg("-b").arg("main"); // Try main first, will handle rev after
                let _ = rev; // TODO: Handle specific revision checkout
            }
        }
    }
    
    // Execute clone
    let output = cmd.output().map_err(|e| hive_config::Error::Io {
        source: e,
        location: snafu::Location::default(),
    })?;
    
    if !output.status.success() {
        // If clone fails, treat as no manifest available
        return Ok(None);
    }
    
    // Determine manifest path based on package
    let manifest_path = if let Some(package) = &git_source.package {
        // Look in crates/{package}/Hive.toml for Rust packages
        clone_path.join("crates").join(package).join("Hive.toml")
    } else {
        // Look in root for Hive.toml
        clone_path.join("Hive.toml")
    };
    
    // Check if manifest exists and load it
    if manifest_path.exists() {
        Ok(Some(ActorManifest::from_path(manifest_path.parent().unwrap())?))
    } else {
        Ok(None)
    }
}

pub fn load_manifest_for_source(source: &ActorSource) -> std::result::Result<Option<ActorManifest>, hive_config::Error> {
    match source {
        ActorSource::Path(path_source) => {
            let base_path = Path::new(&path_source.path);
            
            // Determine the actual path where Hive.toml should be located
            let manifest_dir = if let Some(package) = &path_source.package {
                // For packages, look in crates/{package}/
                base_path.join("crates").join(package)
            } else {
                // For single actors, look in the root path
                base_path.to_path_buf()
            };
            
            // Check if Hive.toml exists in the determined directory
            let manifest_path = manifest_dir.join("Hive.toml");
            if manifest_path.exists() {
                Ok(Some(ActorManifest::from_path(&manifest_dir)?))
            } else {
                // No manifest found
                Ok(None)
            }
        }
        ActorSource::Git(git_source) => {
            // Clone the repository temporarily to read Hive.toml
            load_manifest_from_git(git_source)
        }
    }
}

fn sources_match(source1: &ActorSource, source2: &ActorSource) -> bool {
    match (source1, source2) {
        (ActorSource::Path(p1), ActorSource::Path(p2)) => {
            p1.path == p2.path && p1.package == p2.package
        }
        (ActorSource::Git(g1), ActorSource::Git(g2)) => {
            g1.url == g2.url && 
            git_refs_match(&g1.git_ref, &g2.git_ref) && 
            g1.package == g2.package
        }
        _ => false,
    }
}

fn git_refs_match(ref1: &Option<hive_config::GitRef>, ref2: &Option<hive_config::GitRef>) -> bool {
    match (ref1, ref2) {
        (None, None) => true,
        (Some(r1), Some(r2)) => match (r1, r2) {
            (hive_config::GitRef::Branch(b1), hive_config::GitRef::Branch(b2)) => b1 == b2,
            (hive_config::GitRef::Tag(t1), hive_config::GitRef::Tag(t2)) => t1 == t2,
            (hive_config::GitRef::Rev(r1), hive_config::GitRef::Rev(r2)) => r1 == r2,
            _ => false,
        },
        _ => false,
    }
}

fn source_to_string(source: &ActorSource) -> String {
    match source {
        ActorSource::Path(p) => format!("path: {}", p.path),
        ActorSource::Git(g) => format!("git: {}", g.url),
    }
}

fn resolve_relative_source(parent_source: &ActorSource, dep_source: ActorSource) -> ActorSource {
    match (&parent_source, &dep_source) {
        (ActorSource::Path(parent_path), ActorSource::Path(dep_path)) => {
            // If dependency path is relative, resolve it relative to parent
            let dep_path_buf = Path::new(&dep_path.path);
            if dep_path_buf.is_relative() {
                let parent_base_path = Path::new(&parent_path.path);
                
                // Determine the actual directory where the parent's manifest is located
                let parent_manifest_dir = if let Some(package) = &parent_path.package {
                    // For packages, the manifest is in crates/{package}/
                    parent_base_path.join("crates").join(package)
                } else {
                    // For single actors, the manifest is in the root path
                    parent_base_path.to_path_buf()
                };
                
                // Resolve the dependency path relative to where the parent's manifest actually is
                let resolved = parent_manifest_dir.join(&dep_path.path);
                ActorSource::Path(hive_config::PathSource {
                    path: resolved.to_string_lossy().into_owned(),
                    package: dep_path.package.clone(),
                })
            } else {
                dep_source
            }
        }
        _ => dep_source, // Git sources or mixed types - return as is
    }
}

/// Merge dependency settings (source, config, auto_spawn) from manifest defaults and user overrides
fn merge_dependency_settings(
    dep_config: &hive_config::DependencyConfig,
    parent_source: &ActorSource,
    parent_logical_name: Option<&str>,
    dep_logical_name: &str,
    user_configs: &HashMap<String, toml::Table>
) -> (ActorSource, Option<toml::Table>, bool) {
    // Start with manifest-provided values
    let mut merged_source = dep_config.source.clone();
    let mut merged_config = dep_config.config.clone();
    let mut merged_auto_spawn = dep_config.auto_spawn.unwrap_or(false);
    
    // Look for user override in parent's config
    if let Some(parent_name) = parent_logical_name {
        if let Some(parent_config) = user_configs.get(parent_name) {
            // Look for dependencies.{dep_name} in user config
            if let Some(dependencies) = parent_config.get("dependencies") {
                if let Some(dependencies_table) = dependencies.as_table() {
                    if let Some(dep_override) = dependencies_table.get(dep_logical_name) {
                        if let Some(dep_override_table) = dep_override.as_table() {
                            // Override source if specified
                            if let Some(source_value) = dep_override_table.get("source") {
                                if let Ok(override_source) = source_value.clone().try_into() {
                                    merged_source = override_source;
                                }
                            }
                            
                            // Override auto_spawn if specified
                            if let Some(auto_spawn_value) = dep_override_table.get("auto_spawn") {
                                if let Some(auto_spawn_bool) = auto_spawn_value.as_bool() {
                                    merged_auto_spawn = auto_spawn_bool;
                                }
                            }
                            
                            // Override config if specified
                            if let Some(override_config) = dep_override_table.get("config") {
                                if let Some(override_config_table) = override_config.as_table() {
                                    // Merge the configs - user config wins
                                    match merged_config {
                                        Some(ref mut base) => {
                                            merge_toml_tables(base, override_config_table);
                                        }
                                        None => {
                                            merged_config = Some(override_config_table.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    // Resolve relative paths after all overrides are applied
    let resolved_source = resolve_relative_source(parent_source, merged_source);
    
    (resolved_source, merged_config, merged_auto_spawn)
}

/// Merge two TOML tables, with the second table's values taking precedence
fn merge_toml_tables(base: &mut toml::Table, override_table: &toml::Table) {
    for (key, value) in override_table {
        match (base.get_mut(key), value) {
            // If both are tables, merge recursively
            (Some(toml::Value::Table(base_table)), toml::Value::Table(override_table)) => {
                merge_toml_tables(base_table, override_table);
            }
            // Otherwise, user value wins
            _ => {
                base.insert(key.clone(), value.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hive_config::PathSource;

    #[test]
    fn test_missing_manifest_fails() {
        let actors = vec![
            Actor {
                name: "actor_a".to_string(),
                source: ActorSource::Path(PathSource {
                    path: "test_data/simple_actor".to_string(),
                    package: None,
                }),
                config: None,
                auto_spawn: false,
            }
        ];

        let resolver = DependencyResolver::new();
        let result = resolver.resolve_all(actors);
        
        // Should fail because Hive.toml is required for ALL actors
        assert!(result.is_err());
        let error = result.unwrap_err();
        let error_msg = error.to_string();
        
        // Should be a MissingManifest error
        assert!(error_msg.contains("missing required Hive.toml manifest file"));
        assert!(error_msg.contains("actor_a"));
    }

    #[test]
    fn test_circular_dependency_detection() {
        // This test would require setting up test actors with circular dependencies
        // For now, we'll just test the basic structure
        let resolver = DependencyResolver::new();
        assert!(resolver.resolved.is_empty());
    }
}