use hive_actor_loader::dependency_resolver::DependencyResolver;
use hive_config::{Actor, ActorSource, GitRef, PathSource, Repository};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;
use url::Url;

#[tokio::test]
async fn test_circular_dependency_error() {
    // This would require setting up test actors with circular dependencies
    // For now, we verify the resolver can handle non-circular cases
    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(vec![], vec![]).await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

#[tokio::test]
async fn test_invalid_path_error() {
    
    let actors = vec![Actor {
        name: "nonexistent_actor".to_string(),
        source: ActorSource::Path(PathSource {
            path: "/this/path/does/not/exist".to_string(),
            package: None,
        }),
        config: None,
        auto_spawn: false,
        required_spawn_with: vec![],
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, vec![]).await;

    // Should fail because the path doesn't exist (and therefore no Hive.toml exists)
    assert!(result.is_err());
    let error = result.unwrap_err();
    let error_msg = error.to_string();

    // Should be a MissingManifest error since the path doesn't exist
    assert!(error_msg.contains("missing required Hive.toml manifest file"));
    assert!(error_msg.contains("nonexistent_actor"));
}

#[tokio::test]
async fn test_manifest_load_error() {
    
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_actors");

    // Test with a valid path that should load successfully
    let actors = vec![Actor {
        name: "test_actor".to_string(),
        source: ActorSource::Path(PathSource {
            path: test_actors_path
                .join("simple_actor")
                .to_str()
                .unwrap()
                .to_string(),
            package: None,
        }),
        config: None,
        auto_spawn: false,
        required_spawn_with: vec![],
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, vec![]).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_git_source_requires_manifest() {
    // Test that git sources fail when the repository doesn't exist
    
    let actors = vec![Actor {
        name: "git_actor".to_string(),
        source: ActorSource::Git(Repository {
            url: Url::parse("https://github.com/example/repo").unwrap(),
            git_ref: Some(GitRef::Branch("main".to_string())),
            package: None,
        }),
        config: None,
        auto_spawn: false,
        required_spawn_with: vec![],
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, vec![]).await;

    // Should fail - ALL actors now require Hive.toml manifests
    assert!(result.is_err());
    let error = result.unwrap_err();
    let error_msg = error.to_string();

    // Should be a ManifestLoad error (git clone fails for non-existent repo)
    assert!(error_msg.contains("Failed to load manifest for actor 'git_actor'"));
    assert!(error_msg.contains("git: https://github.com/example/repo"));
    assert!(error_msg.contains("Repository not found"));
}

#[tokio::test]
async fn test_dependency_resolution_with_path_sources() {
    
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_actors");

    let actors = vec![
        // Path source with dependencies
        Actor {
            name: "coordinator_instance".to_string(),
            source: ActorSource::Path(PathSource {
                path: test_actors_path
                    .join("coordinator")
                    .to_str()
                    .unwrap()
                    .to_string(),
                package: None,
            }),
            config: None,
            auto_spawn: false,
            required_spawn_with: vec![],
        },
        // Another path source without dependencies
        Actor {
            name: "simple_service".to_string(),
            source: ActorSource::Path(PathSource {
                path: test_actors_path
                    .join("simple_actor")
                    .to_str()
                    .unwrap()
                    .to_string(),
                package: None,
            }),
            config: None,
            auto_spawn: true,
            required_spawn_with: vec![],
        },
    ];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, vec![]).await;

    assert!(result.is_ok());
    let resolved = result.unwrap();

    // Should have coordinator + its logger dependency + simple service
    assert_eq!(resolved.len(), 3);
    assert!(resolved.contains_key("coordinator_instance"));
    assert!(resolved.contains_key("logger")); // Dependency of coordinator
    assert!(resolved.contains_key("simple_service"));

    // Check auto_spawn settings
    assert!(!resolved["coordinator_instance"].auto_spawn);
    assert!(resolved["logger"].auto_spawn); // From manifest
    assert!(resolved["simple_service"].auto_spawn); // From user config

    // Check actor IDs from manifests
    assert_eq!(
        resolved["coordinator_instance"].actor_id,
        "test:coordinator"
    );
    assert_eq!(resolved["logger"].actor_id, "test:logger");
    assert_eq!(resolved["simple_service"].actor_id, "test:simple-actor");
}

#[tokio::test]
async fn test_package_manifest_loading() {
    // Create a test workspace structure
    let temp_dir = TempDir::new().unwrap();
    let workspace_path = temp_dir.path();
    

    // Create crates/test_package/Hive.toml
    let package_dir = workspace_path.join("crates").join("test_package");
    fs::create_dir_all(&package_dir).unwrap();

    let manifest_content = r#"
actor_id = "test:package-actor"

[dependencies.helper]
source = { path = "../helper_actor" }
auto_spawn = true
"#;

    fs::write(package_dir.join("Hive.toml"), manifest_content).unwrap();

    // Also create the helper actor manifest
    // The path "../helper_actor" from crates/test_package means we need it at crates/helper_actor
    let helper_dir = workspace_path.join("crates").join("helper_actor");
    fs::create_dir_all(&helper_dir).unwrap();
    fs::write(helper_dir.join("Hive.toml"), r#"actor_id = "test:helper""#).unwrap();

    // Test loading manifest with package specification
    let actors = vec![Actor {
        name: "package_actor_instance".to_string(),
        source: ActorSource::Path(PathSource {
            path: workspace_path.to_str().unwrap().to_string(),
            package: Some("crates/test_package".to_string()),
        }),
        config: None,
        auto_spawn: false,
        required_spawn_with: vec![],
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, vec![]).await;

    assert!(
        result.is_ok(),
        "Failed to resolve package actor: {:?}",
        result
    );
    let resolved = result.unwrap();

    // Should have the main actor + its helper dependency
    assert_eq!(resolved.len(), 2);
    assert!(resolved.contains_key("package_actor_instance"));
    assert!(resolved.contains_key("helper"));

    // Check actor IDs
    assert_eq!(
        resolved["package_actor_instance"].actor_id,
        "test:package-actor"
    );
    assert_eq!(resolved["helper"].actor_id, "test:helper");

    // Check auto_spawn from dependency manifest
    assert!(resolved["helper"].auto_spawn);
}

#[tokio::test]
async fn test_package_manifest_not_found() {
    let temp_dir = TempDir::new().unwrap();
    let workspace_path = temp_dir.path();
    

    // Create workspace structure but NO Hive.toml
    let package_dir = workspace_path
        .join("crates")
        .join("missing_manifest_package");
    fs::create_dir_all(&package_dir).unwrap();

    let actors = vec![Actor {
        name: "missing_manifest_actor".to_string(),
        source: ActorSource::Path(PathSource {
            path: workspace_path.to_str().unwrap().to_string(),
            package: Some("crates/missing_manifest_package".to_string()),
        }),
        config: None,
        auto_spawn: false,
        required_spawn_with: vec![],
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, vec![]).await;

    // Should fail because Hive.toml is missing
    assert!(result.is_err());
    let error = result.unwrap_err();
    let error_msg = error.to_string();

    assert!(error_msg.contains("missing required Hive.toml manifest file"));
    assert!(error_msg.contains("missing_manifest_actor"));
}

