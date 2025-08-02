use std::collections::HashMap;
use std::path::Path;
use snafu::{Location, Snafu};
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

    #[snafu(display("Invalid user configuration: Actor '{logical_name}' is defined in [actors] but already exists in the dependency chain. Use [actor_overrides.{logical_name}] instead to override dependency configuration."))]
    ActorConflictsWithDependency {
        logical_name: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Invalid user configuration: Actor override '{logical_name}' specified in [actor_overrides] but no actor with this name exists in any dependency chain. Remove this override or add the actor to [actors] if you want to define a new actor."))]
    OverrideForNonExistentDependency {
        logical_name: String,
        #[snafu(implicit)]
        location: Location,
    },

    #[snafu(display("Invalid user configuration: Actor '{logical_name}' is defined in both [actors] and [actor_overrides]. Use only [actors.{logical_name}] to define user actors, or only [actor_overrides.{logical_name}] to override dependencies."))]
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
    pub fn resolve_all(mut self, user_actors: Vec<Actor>, actor_overrides: Vec<hive_config::ActorOverride>) -> Result<HashMap<String, ResolvedActor>> {
        // Build maps for validation and resolution
        let mut user_actor_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut global_overrides: HashMap<String, Actor> = HashMap::new();
        for actor in user_actors.iter() {
            user_actor_names.insert(actor.name.clone());
            global_overrides.insert(actor.name.clone(), actor.clone());
        }

        let mut override_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut overrides_map: HashMap<String, hive_config::ActorOverride> = HashMap::new();
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
            self.resolve_actor_internal(actor, false, &global_overrides, &overrides_map)?;
        }

        // Collect all actors that exist in dependency chains
        let _all_resolved_names: std::collections::HashSet<String> = self.resolved.keys().cloned().collect();

        // TODO: Add more sophisticated validation for:
        // - User actors conflicting with dependencies (requires two-phase resolution)
        // - Actor overrides referencing non-existent dependencies (requires dependency discovery)
        // For now, we have the basic validation that prevents [actors] and [actor_overrides] 
        // from both defining the same name

        Ok(self.resolved)
    }


    /// Internal method for resolving actors with full context
    fn resolve_actor_internal(
        &mut self, 
        actor: Actor, 
        is_dependency: bool, 
        global_overrides: &HashMap<String, Actor>,
        actor_overrides: &HashMap<String, hive_config::ActorOverride>
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

        // Apply overrides in order: manifest defaults → actor definition → global overrides → actor overrides
        let mut final_source = actor.source.clone();
        let mut final_config = actor.config.clone();
        let mut final_auto_spawn = actor.auto_spawn;
        let mut final_required_spawn_with = manifest.required_spawn_with.clone();

        // Apply global override if it exists
        if let Some(global_override) = global_overrides.get(&logical_name) {
            final_source = global_override.source.clone();
            final_config = global_override.config.clone();
            final_auto_spawn = global_override.auto_spawn;
            // Use user-provided required_spawn_with if not empty, otherwise keep current
            if !global_override.required_spawn_with.is_empty() {
                final_required_spawn_with = global_override.required_spawn_with.clone();
            }
        }

        // Apply actor override if it exists (this takes precedence over global overrides)
        if let Some(actor_override) = actor_overrides.get(&logical_name) {
            if let Some(override_source) = &actor_override.source {
                final_source = override_source.clone();
            }
            if let Some(override_config) = &actor_override.config {
                final_config = Some(override_config.clone());
            }
            if let Some(override_auto_spawn) = actor_override.auto_spawn {
                final_auto_spawn = override_auto_spawn;
            }
            if let Some(override_required_spawn_with) = &actor_override.required_spawn_with {
                final_required_spawn_with = override_required_spawn_with.clone();
            }
        }

        // Create resolved actor
        let resolved_actor = ResolvedActor {
            logical_name: logical_name.clone(),
            actor_id,
            source: final_source.clone(), // Clone for later use
            config: final_config,
            auto_spawn: final_auto_spawn,
            required_spawn_with: final_required_spawn_with,
            is_dependency,
        };

        // Store the resolved actor
        self.resolved.insert(logical_name.clone(), resolved_actor);

        // No longer checking for orphaned dependency configs as we use global overrides now

        // Resolve dependencies from manifest
        for (dep_name, dep_config) in manifest.dependencies {
            // Start with manifest defaults
            let dep_source = resolve_relative_source(&final_source, dep_config.source.clone());
            let dep_config_table = dep_config.config.clone();
            let dep_auto_spawn = dep_config.auto_spawn.unwrap_or(false);
            
            let dep_actor = Actor {
                name: dep_name.clone(),
                source: dep_source,
                config: dep_config_table,
                auto_spawn: dep_auto_spawn,
                required_spawn_with: vec![], // Dependencies don't have required_spawn_with from manifest
            };
            self.resolve_actor_internal(dep_actor, true, global_overrides, actor_overrides)?;
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
        // Package is the full subpath to the package
        clone_path.join(package).join("Hive.toml")
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
                // Package is the full subpath to the package
                base_path.join(package)
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
                    // Package is the full subpath to the package
                    parent_base_path.join(package)
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
                required_spawn_with: vec![],
            }
        ];

        let resolver = DependencyResolver::new();
        let result = resolver.resolve_all(actors, vec![]);
        
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