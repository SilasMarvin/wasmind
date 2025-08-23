use std::path::PathBuf;
use wasmind_actor_loader::dependency_resolver::DependencyResolver;
use wasmind_config::{Actor, ActorSource, PathSource};

#[tokio::test]
async fn test_simple_actor_resolution() {
    let test_actor_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_actors/simple_actor");

    let actors = vec![Actor {
        name: "test_actor".to_string(),
        source: ActorSource::Path(PathSource {
            path: test_actor_path.to_str().unwrap().to_string(),
        }),
        config: None,
        auto_spawn: false,
        required_spawn_with: vec![],
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, vec![]).await;

    assert!(result.is_ok());
    let resolved = result.unwrap();
    assert_eq!(resolved.len(), 1);
    assert!(resolved.contains_key("test_actor"));

    let actor = &resolved["test_actor"];
    assert_eq!(actor.actor_id, "test:simple-actor");
    assert_eq!(actor.logical_name, "test_actor");
}

#[tokio::test]
async fn test_actor_without_manifest_fails() {
    let actors = vec![Actor {
        name: "no_manifest_actor".to_string(),
        source: ActorSource::Path(PathSource {
            path: "/tmp/nonexistent".to_string(),
        }),
        config: None,
        auto_spawn: false,
        required_spawn_with: vec![],
    }];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, vec![]).await;

    // Should fail because Wasmind.toml is required
    assert!(result.is_err());
    let error = result.unwrap_err();
    let error_msg = error.to_string();

    // Should be a MissingManifest error with clear message
    assert!(error_msg.contains("missing required Wasmind.toml manifest file"));
    assert!(error_msg.contains("no_manifest_actor"));
}

#[tokio::test]
async fn test_dependency_resolution() {
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_actors");

    let actors = vec![Actor {
        name: "my_coordinator".to_string(),
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

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, vec![]).await;

    assert!(result.is_ok());
    let resolved = result.unwrap();

    // Should have both coordinator and its logger dependency
    assert_eq!(resolved.len(), 2);
    assert!(resolved.contains_key("my_coordinator"));
    assert!(resolved.contains_key("logger"));

    let coordinator = &resolved["my_coordinator"];
    assert_eq!(coordinator.actor_id, "test:coordinator");
    assert!(!coordinator.is_dependency);

    let logger = &resolved["logger"];
    assert_eq!(logger.actor_id, "test:logger");
    assert!(logger.is_dependency);
    assert!(logger.auto_spawn);
}

#[tokio::test]
async fn test_global_configuration_override() {
    use toml::Table;

    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_actors");

    // Create global override for logger actor
    let mut logger_config = Table::new();
    logger_config.insert(
        "level".to_string(),
        toml::Value::String("debug".to_string()),
    );
    logger_config.insert(
        "output".to_string(),
        toml::Value::String("stdout".to_string()),
    );

    let actors = vec![
        Actor {
            name: "my_coordinator".to_string(),
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
        // Global override for logger
        Actor {
            name: "logger".to_string(),
            source: ActorSource::Path(PathSource {
                path: test_actors_path
                    .join("logger")
                    .to_str()
                    .unwrap()
                    .to_string(),
            }),
            config: Some(logger_config),
            auto_spawn: false,
            required_spawn_with: vec![],
        },
    ];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, vec![]).await;

    assert!(result.is_ok());
    let resolved = result.unwrap();

    let logger = &resolved["logger"];
    let logger_config = logger.config.as_ref().unwrap();

    // Should have global override config (level=debug, output=stdout) + manifest default (format=json)
    assert_eq!(
        logger_config.get("level").unwrap().as_str().unwrap(),
        "debug"
    ); // Global override
    assert_eq!(
        logger_config.get("output").unwrap().as_str().unwrap(),
        "stdout"
    ); // Global override
    // Note: The dependency's default config is not merged with global overrides
}

#[tokio::test]
async fn test_global_source_and_auto_spawn_overrides() {
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_actors");

    let actors = vec![
        Actor {
            name: "my_coordinator".to_string(),
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
        // Global override for logger with different source and auto_spawn
        Actor {
            name: "logger".to_string(),
            source: ActorSource::Path(PathSource {
                path: test_actors_path
                    .join("simple_actor")
                    .to_str()
                    .unwrap()
                    .to_string(), // Different path
            }),
            config: None,
            auto_spawn: false, // Override to false (manifest default is true)
            required_spawn_with: vec![],
        },
    ];

    let resolver = DependencyResolver::default();
    let result = resolver.resolve_all(actors, vec![]).await;

    assert!(result.is_ok());
    let resolved = result.unwrap();

    // Logger should be resolved with the global override
    let logger = &resolved["logger"];

    // Source should be the globally overridden path
    match &logger.source {
        ActorSource::Path(path_source) => {
            assert!(path_source.path.ends_with("test_actors/simple_actor"));
        }
        _ => panic!("Expected path source"),
    }

    // auto_spawn should be globally overridden to false
    assert!(!logger.auto_spawn);
}
