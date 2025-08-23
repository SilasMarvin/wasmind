use std::path::PathBuf;
use wasmind_actor_loader::dependency_resolver::DependencyResolver;
use wasmind_config::{Actor, ActorOverride, ActorSource, PathSource};

#[tokio::test]
async fn test_actor_overrides_config_only() {
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_actors");

    let actors = vec![Actor {
        name: "coordinator_instance".to_string(),
        source: ActorSource::Path(PathSource {
            path: test_actors_path
                .join("coordinator")
                .to_str()
                .unwrap()
                .to_string(),
        }),
        config: None,
        auto_spawn: false,
        required_spawn_with: vec![],
    }];

    let mut logger_config = toml::Table::new();
    logger_config.insert(
        "level".to_string(),
        toml::Value::String("debug".to_string()),
    );
    logger_config.insert(
        "format".to_string(),
        toml::Value::String("json".to_string()),
    );

    let actor_overrides = vec![ActorOverride {
        name: "logger".to_string(),
        source: None,
        config: Some(logger_config),
        auto_spawn: None,
        required_spawn_with: None,
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, actor_overrides).await;

    assert!(result.is_ok(), "Resolution should succeed: {result:?}");
    let resolved = result.unwrap();

    assert_eq!(resolved.len(), 2);
    assert!(resolved.contains_key("coordinator_instance"));
    assert!(resolved.contains_key("logger"));

    let logger = &resolved["logger"];

    let logger_config = logger.config.as_ref().unwrap();
    assert_eq!(
        logger_config.get("level").unwrap().as_str().unwrap(),
        "debug"
    );
    assert_eq!(
        logger_config.get("format").unwrap().as_str().unwrap(),
        "json"
    );

    assert!(logger.auto_spawn); // From manifest default
    assert_eq!(logger.actor_id, "test:logger"); // From manifest
}

#[tokio::test]
async fn test_actor_overrides_for_existing_dependency() {
    // Test that actor_overrides can modify existing dependencies
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_actors");

    let actors = vec![Actor {
        name: "coordinator_instance".to_string(),
        source: ActorSource::Path(PathSource {
            path: test_actors_path
                .join("coordinator")
                .to_str()
                .unwrap()
                .to_string(),
        }),
        config: None,
        auto_spawn: false,
        required_spawn_with: vec![],
    }];

    let mut override_config = toml::Table::new();
    override_config.insert(
        "level".to_string(),
        toml::Value::String("error".to_string()),
    );
    override_config.insert(
        "source".to_string(),
        toml::Value::String("override".to_string()),
    );

    let actor_overrides = vec![ActorOverride {
        name: "logger".to_string(),
        source: None,
        config: Some(override_config),
        auto_spawn: Some(false),
        required_spawn_with: None,
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, actor_overrides).await;

    assert!(result.is_ok());
    let resolved = result.unwrap();

    let logger = &resolved["logger"];

    match &logger.source {
        ActorSource::Path(path_source) => {
            assert!(path_source.path.contains("coordinator/../logger"));
        }
        _ => panic!("Expected path source"),
    }

    let logger_config = logger.config.as_ref().unwrap();
    assert_eq!(
        logger_config.get("level").unwrap().as_str().unwrap(),
        "error"
    );
    assert_eq!(
        logger_config.get("source").unwrap().as_str().unwrap(),
        "override"
    );

    assert!(!logger.auto_spawn);

    assert_eq!(logger.actor_id, "test:logger");
}

#[tokio::test]
async fn test_actor_overrides_all_fields() {
    // Test overriding source, config, auto_spawn, required_spawn_with via actor_overrides
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_actors");

    let actors = vec![Actor {
        name: "coordinator_instance".to_string(),
        source: ActorSource::Path(PathSource {
            path: test_actors_path
                .join("coordinator")
                .to_str()
                .unwrap()
                .to_string(),
        }),
        config: None,
        auto_spawn: false,
        required_spawn_with: vec![],
    }];

    // Override ALL fields in actor_overrides
    let mut override_config = toml::Table::new();
    override_config.insert(
        "overridden".to_string(),
        toml::Value::String("completely".to_string()),
    );

    let actor_overrides = vec![ActorOverride {
        name: "logger".to_string(),
        source: Some(ActorSource::Path(PathSource {
            path: test_actors_path
                .join("simple_actor")
                .to_str()
                .unwrap()
                .to_string(),
        })),
        config: Some(override_config),
        auto_spawn: Some(false), // Override to false
        required_spawn_with: Some(vec!["coordinator_instance".to_string()]), // Add required spawn
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, actor_overrides).await;

    assert!(result.is_ok());
    let resolved = result.unwrap();

    let logger = &resolved["logger"];

    // Source should be overridden to simple_actor
    match &logger.source {
        ActorSource::Path(path_source) => {
            assert!(path_source.path.ends_with("test_actors/simple_actor"));
        }
        _ => panic!("Expected path source"),
    }

    // Config should be completely overridden
    let logger_config = logger.config.as_ref().unwrap();
    assert_eq!(
        logger_config.get("overridden").unwrap().as_str().unwrap(),
        "completely"
    );

    // auto_spawn should be overridden to false
    assert!(!logger.auto_spawn);

    // required_spawn_with should be overridden
    assert_eq!(logger.required_spawn_with, vec!["coordinator_instance"]);

    // actor_id should remain from the original manifest (not the overridden source)
    // This is the current behavior: actor_id comes from original source,
    // even when source is overridden via actor_overrides
    assert_eq!(logger.actor_id, "test:logger");
}

#[tokio::test]
async fn test_actor_overrides_partial_fields() {
    // Test overriding only some fields (e.g., just auto_spawn)
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_actors");

    let actors = vec![Actor {
        name: "coordinator_instance".to_string(),
        source: ActorSource::Path(PathSource {
            path: test_actors_path
                .join("coordinator")
                .to_str()
                .unwrap()
                .to_string(),
        }),
        config: None,
        auto_spawn: false,
        required_spawn_with: vec![],
    }];

    // Override only auto_spawn
    let actor_overrides = vec![ActorOverride {
        name: "logger".to_string(),
        source: None,              // No override
        config: None,              // No override
        auto_spawn: Some(false),   // Override to false
        required_spawn_with: None, // No override
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, actor_overrides).await;

    assert!(result.is_ok());
    let resolved = result.unwrap();

    let logger = &resolved["logger"];

    // Source should remain from dependency manifest (via coordinator's Wasmind.toml)
    match &logger.source {
        ActorSource::Path(path_source) => {
            // The path should be the resolved dependency path: coordinator/../logger
            assert!(path_source.path.contains("coordinator/../logger"));
        }
        _ => panic!("Expected path source, got: {:?}", logger.source),
    }

    // Config should remain from manifest
    let logger_config = logger.config.as_ref().unwrap();
    assert_eq!(
        logger_config.get("format").unwrap().as_str().unwrap(),
        "json"
    );

    // auto_spawn should be overridden to false (manifest default is true)
    assert!(!logger.auto_spawn);

    // actor_id should be from original source
    assert_eq!(logger.actor_id, "test:logger");
}

#[tokio::test]
async fn test_user_defined_actor_separate_from_dependencies() {
    // Test that user can define separate actors that don't conflict with dependencies
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
            }),
            config: None,
            auto_spawn: false,
            required_spawn_with: vec![],
        },
        // User-defined actor that doesn't conflict with any dependency
        Actor {
            name: "my_custom_service".to_string(),
            source: ActorSource::Path(PathSource {
                path: test_actors_path
                    .join("simple_actor")
                    .to_str()
                    .unwrap()
                    .to_string(),
            }),
            config: {
                let mut config = toml::Table::new();
                config.insert(
                    "custom".to_string(),
                    toml::Value::String("service".to_string()),
                );
                Some(config)
            },
            auto_spawn: true,
            required_spawn_with: vec![],
        },
    ];

    // Override the logger dependency (separate from user-defined actor)
    let mut override_config = toml::Table::new();
    override_config.insert(
        "from_override".to_string(),
        toml::Value::String("override_value".to_string()),
    );

    let actor_overrides = vec![ActorOverride {
        name: "logger".to_string(), // This is a dependency override
        source: None,
        config: Some(override_config),
        auto_spawn: Some(false),
        required_spawn_with: None,
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, actor_overrides).await;

    assert!(result.is_ok());
    let resolved = result.unwrap();

    // Should have: coordinator + logger dependency + custom service
    assert_eq!(resolved.len(), 3);
    assert!(resolved.contains_key("coordinator_instance"));
    assert!(resolved.contains_key("logger"));
    assert!(resolved.contains_key("my_custom_service"));

    let logger = &resolved["logger"];
    let custom_service = &resolved["my_custom_service"];

    // Logger should be overridden by actor_override
    let logger_config = logger.config.as_ref().unwrap();
    assert_eq!(
        logger_config
            .get("from_override")
            .unwrap()
            .as_str()
            .unwrap(),
        "override_value"
    );
    assert!(!logger.auto_spawn);
    assert_eq!(logger.actor_id, "test:logger");

    // Custom service should use user definition
    let custom_config = custom_service.config.as_ref().unwrap();
    assert_eq!(
        custom_config.get("custom").unwrap().as_str().unwrap(),
        "service"
    );
    assert!(custom_service.auto_spawn);
    assert_eq!(custom_service.actor_id, "test:simple-actor");
}

#[tokio::test]
async fn test_actor_overrides_nonexistent_actor() {
    // Test graceful handling when actor_overrides reference unused actors
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_actors");

    let actors = vec![Actor {
        name: "simple_service".to_string(),
        source: ActorSource::Path(PathSource {
            path: test_actors_path
                .join("simple_actor")
                .to_str()
                .unwrap()
                .to_string(),
        }),
        config: None,
        auto_spawn: false,
        required_spawn_with: vec![],
    }];

    // Override for an actor that doesn't exist in the dependency tree
    let mut unused_config = toml::Table::new();
    unused_config.insert(
        "unused".to_string(),
        toml::Value::String("value".to_string()),
    );

    let actor_overrides = vec![ActorOverride {
        name: "nonexistent_actor".to_string(),
        source: None,
        config: Some(unused_config),
        auto_spawn: Some(true),
        required_spawn_with: None,
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, actor_overrides).await;

    // Should succeed - unused overrides are simply ignored
    assert!(result.is_ok());
    let resolved = result.unwrap();

    // Should only have the simple_service actor
    assert_eq!(resolved.len(), 1);
    assert!(resolved.contains_key("simple_service"));
    assert!(!resolved.contains_key("nonexistent_actor"));

    let simple_service = &resolved["simple_service"];
    assert_eq!(simple_service.actor_id, "test:simple-actor");
}

#[tokio::test]
async fn test_actor_and_override_conflict_error() {
    // Test that having both [actors.NAME] and [actor_overrides.NAME] causes an error
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_actors");

    let actors = vec![Actor {
        name: "simple_service".to_string(),
        source: ActorSource::Path(PathSource {
            path: test_actors_path
                .join("simple_actor")
                .to_str()
                .unwrap()
                .to_string(),
        }),
        config: None,
        auto_spawn: false,
        required_spawn_with: vec![],
    }];

    // Try to override the same actor that's defined in actors
    let actor_overrides = vec![ActorOverride {
        name: "simple_service".to_string(), // Same name as in actors!
        source: None,
        config: Some({
            let mut config = toml::Table::new();
            config.insert(
                "should_not_work".to_string(),
                toml::Value::String("error".to_string()),
            );
            config
        }),
        auto_spawn: Some(true),
        required_spawn_with: None,
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, actor_overrides).await;

    // Should fail with ActorAndOverrideConflict error
    assert!(result.is_err());
    let error = result.unwrap_err();
    let error_msg = error.to_string();

    assert!(error_msg.contains("defined in both [actors] and [actor_overrides]"));
    assert!(error_msg.contains("simple_service"));
}
