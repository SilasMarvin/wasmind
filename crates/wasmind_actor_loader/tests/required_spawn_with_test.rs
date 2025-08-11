use wasmind_actor_loader::dependency_resolver::DependencyResolver;
use wasmind_config::{Actor, ActorOverride, ActorSource, PathSource};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

#[tokio::test]
async fn test_required_spawn_with_basic_functionality() {
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_actors");

    let actors = vec![Actor {
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
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, vec![]).await;

    assert!(result.is_ok(), "Resolution should succeed: {result:?}");
    let resolved = result.unwrap();

    // Should have coordinator + logger dependency
    assert_eq!(resolved.len(), 2);
    assert!(resolved.contains_key("coordinator_instance"));
    assert!(resolved.contains_key("logger"));

    let coordinator = &resolved["coordinator_instance"];
    let logger = &resolved["logger"];

    // Check that required_spawn_with is properly set from manifests
    // Coordinator should have no required_spawn_with (empty in test manifest)
    assert_eq!(coordinator.required_spawn_with, Vec::<String>::new());

    // Logger should have its required_spawn_with from manifest (if any)
    // Based on test_actors/logger/Wasmind.toml structure
    assert_eq!(logger.required_spawn_with, Vec::<String>::new());
}

#[tokio::test]
async fn test_required_spawn_with_global_override() {
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_actors");

    let actors = vec![
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
        Actor {
            name: "logger".to_string(),
            source: ActorSource::Path(PathSource {
                path: test_actors_path
                    .join("logger")
                    .to_str()
                    .unwrap()
                    .to_string(),
                package: None,
            }),
            config: None,
            auto_spawn: true,
            required_spawn_with: vec!["coordinator_instance".to_string()],
        },
    ];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, vec![]).await;

    assert!(result.is_ok());
    let resolved = result.unwrap();

    let logger = &resolved["logger"];

    // required_spawn_with should be from global override
    assert_eq!(logger.required_spawn_with, vec!["coordinator_instance"]);
}

#[tokio::test]
async fn test_required_spawn_with_actor_override() {
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_actors");

    let actors = vec![Actor {
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
    }];

    let actor_overrides = vec![ActorOverride {
        name: "logger".to_string(),
        source: None,
        config: None,
        auto_spawn: None,
        required_spawn_with: Some(vec![
            "coordinator_instance".to_string(),
            "some_other_actor".to_string(),
        ]),
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, actor_overrides).await;

    assert!(result.is_ok());
    let resolved = result.unwrap();

    let logger = &resolved["logger"];

    // required_spawn_with should be from actor_override
    assert_eq!(
        logger.required_spawn_with,
        vec!["coordinator_instance", "some_other_actor"]
    );
}

#[tokio::test]
async fn test_required_spawn_with_empty_override() {
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_actors");

    let actors = vec![Actor {
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
    }];

    let actor_overrides = vec![ActorOverride {
        name: "logger".to_string(),
        source: None,
        config: None,
        auto_spawn: None,
        required_spawn_with: Some(vec![]),
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, actor_overrides).await;

    assert!(result.is_ok());
    let resolved = result.unwrap();

    let logger = &resolved["logger"];

    // required_spawn_with should be empty from actor_override
    assert_eq!(logger.required_spawn_with, Vec::<String>::new());
}

#[tokio::test]
async fn test_required_spawn_with_precedence() {
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_actors");

    let actors = vec![Actor {
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
        required_spawn_with: vec!["from_main".to_string()],
    }];

    let actor_overrides = vec![ActorOverride {
        name: "logger".to_string(),
        source: None,
        config: None,
        auto_spawn: None,
        required_spawn_with: Some(vec!["from_override".to_string()]),
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, actor_overrides).await;

    assert!(result.is_ok());
    let resolved = result.unwrap();

    let coordinator = &resolved["coordinator_instance"];
    let logger = &resolved["logger"];

    // Coordinator should use its own required_spawn_with
    assert_eq!(coordinator.required_spawn_with, vec!["from_main"]);

    // Logger should use actor_override value (highest precedence)
    assert_eq!(logger.required_spawn_with, vec!["from_override"]);
}

#[tokio::test]
async fn test_required_spawn_with_with_complex_actor() {
    let temp_dir = TempDir::new().unwrap();
    let workspace_path = temp_dir.path();

    let package_dir = workspace_path.join("crates").join("test_package");
    fs::create_dir_all(&package_dir).unwrap();

    let manifest_content = r#"
actor_id = "test:package-actor"
required_spawn_with = ["helper", "monitor"]

[dependencies.helper]
source = { path = "../helper_actor" }
auto_spawn = true

[dependencies.monitor]
source = { path = "../monitor_actor" }
auto_spawn = false
"#;

    fs::write(package_dir.join("Wasmind.toml"), manifest_content).unwrap();

    let helper_dir = workspace_path.join("crates").join("helper_actor");
    fs::create_dir_all(&helper_dir).unwrap();
    fs::write(helper_dir.join("Wasmind.toml"), r#"actor_id = "test:helper""#).unwrap();

    let monitor_dir = workspace_path.join("crates").join("monitor_actor");
    fs::create_dir_all(&monitor_dir).unwrap();
    fs::write(
        monitor_dir.join("Wasmind.toml"),
        r#"actor_id = "test:monitor""#,
    )
    .unwrap();

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
        "Failed to resolve actor with manifest required_spawn_with: {result:?}"
    );
    let resolved = result.unwrap();

    // Should have main actor + 2 dependencies
    assert_eq!(resolved.len(), 3);
    assert!(resolved.contains_key("package_actor_instance"));
    assert!(resolved.contains_key("helper"));
    assert!(resolved.contains_key("monitor"));

    let package_actor = &resolved["package_actor_instance"];

    // Should have required_spawn_with from manifest
    assert_eq!(package_actor.required_spawn_with, vec!["helper", "monitor"]);
    assert_eq!(package_actor.actor_id, "test:package-actor");
}

#[tokio::test]
async fn test_required_spawn_with_override_precedence_chain() {
    let temp_dir = TempDir::new().unwrap();
    let workspace_path = temp_dir.path();

    let package_dir = workspace_path.join("crates").join("main_actor");
    fs::create_dir_all(&package_dir).unwrap();

    let manifest_content = r#"
actor_id = "test:main-actor"
required_spawn_with = ["from_manifest"]

[dependencies.dep]
source = { path = "../dep_actor" }
"#;

    fs::write(package_dir.join("Wasmind.toml"), manifest_content).unwrap();

    let dep_dir = workspace_path.join("crates").join("dep_actor");
    fs::create_dir_all(&dep_dir).unwrap();
    fs::write(dep_dir.join("Wasmind.toml"), r#"actor_id = "test:dep""#).unwrap();

    let actors = vec![Actor {
        name: "main_instance".to_string(),
        source: ActorSource::Path(PathSource {
            path: workspace_path.to_str().unwrap().to_string(),
            package: Some("crates/main_actor".to_string()),
        }),
        config: None,
        auto_spawn: false,
        required_spawn_with: vec!["user_specified".to_string()],
    }];

    let actor_overrides = vec![ActorOverride {
        name: "dep".to_string(),
        source: None,
        config: None,
        auto_spawn: None,
        required_spawn_with: Some(vec!["from_actor_override".to_string()]),
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, actor_overrides).await;

    assert!(result.is_ok());
    let resolved = result.unwrap();

    let main_instance = &resolved["main_instance"];
    let dep = &resolved["dep"];

    // Main actor should use user-specified required_spawn_with (beats manifest)
    assert_eq!(main_instance.required_spawn_with, vec!["user_specified"]);

    // Dependency should use actor_override (beats global and manifest)
    assert_eq!(dep.required_spawn_with, vec!["from_actor_override"]);
}
