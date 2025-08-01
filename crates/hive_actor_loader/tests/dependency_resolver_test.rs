use hive_config::{Actor, ActorSource, PathSource};
use hive_actor_loader::dependency_resolver::DependencyResolver;
use std::path::PathBuf;

#[test]
fn test_simple_actor_resolution() {
    let test_actor_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test_actors/simple_actor");
    
    let actors = vec![
        Actor {
            name: "test_actor".to_string(),
            source: ActorSource::Path(PathSource {
                path: test_actor_path.to_str().unwrap().to_string(),
                package: None,
            }),
            config: None,
            auto_spawn: false,
        }
    ];

    let resolver = DependencyResolver::new();
    let result = resolver.resolve_all(actors);
    
    assert!(result.is_ok());
    let resolved = result.unwrap();
    assert_eq!(resolved.len(), 1);
    assert!(resolved.contains_key("test_actor"));
    
    let actor = &resolved["test_actor"];
    assert_eq!(actor.actor_id, "test:simple-actor");
    assert_eq!(actor.logical_name, "test_actor");
}

#[test]
fn test_actor_without_manifest_fails() {
    let actors = vec![
        Actor {
            name: "no_manifest_actor".to_string(),
            source: ActorSource::Path(PathSource {
                path: "/tmp/nonexistent".to_string(),
                package: None,
            }),
            config: None,
            auto_spawn: false,
        }
    ];

    let resolver = DependencyResolver::new();
    let result = resolver.resolve_all(actors);
    
    // Should fail because Hive.toml is required
    assert!(result.is_err());
    let error = result.unwrap_err();
    let error_msg = error.to_string();
    
    // Should be a MissingManifest error with clear message
    assert!(error_msg.contains("missing required Hive.toml manifest file"));
    assert!(error_msg.contains("no_manifest_actor"));
}

#[test]
fn test_dependency_resolution() {
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test_actors");
    
    let actors = vec![
        Actor {
            name: "my_coordinator".to_string(),
            source: ActorSource::Path(PathSource {
                path: test_actors_path.join("coordinator").to_str().unwrap().to_string(),
                package: None,
            }),
            config: None,
            auto_spawn: false,
        }
    ];

    let resolver = DependencyResolver::new();
    let result = resolver.resolve_all(actors);
    
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
    assert_eq!(logger.auto_spawn, true);
}

#[test]
fn test_configuration_merging() {
    use toml::Table;
    
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test_actors");
    
    // Create user config with dependency override
    let mut user_config = Table::new();
    let mut dependencies = Table::new();
    let mut logger_override = Table::new();
    let mut logger_config = Table::new();
    
    logger_config.insert("level".to_string(), toml::Value::String("debug".to_string()));
    logger_config.insert("output".to_string(), toml::Value::String("stdout".to_string()));
    logger_override.insert("config".to_string(), toml::Value::Table(logger_config));
    dependencies.insert("logger".to_string(), toml::Value::Table(logger_override));
    user_config.insert("dependencies".to_string(), toml::Value::Table(dependencies));
    
    let actors = vec![
        Actor {
            name: "my_coordinator".to_string(),
            source: ActorSource::Path(PathSource {
                path: test_actors_path.join("coordinator").to_str().unwrap().to_string(),
                package: None,
            }),
            config: Some(user_config),
            auto_spawn: false,
        }
    ];

    let resolver = DependencyResolver::new();
    let result = resolver.resolve_all(actors);
    
    assert!(result.is_ok());
    let resolved = result.unwrap();
    
    let logger = &resolved["logger"];
    let logger_config = logger.config.as_ref().unwrap();
    
    // Should have merged config: manifest default (format=json) + user override (level=debug, output=stdout)
    assert_eq!(logger_config.get("level").unwrap().as_str().unwrap(), "debug"); // User override
    assert_eq!(logger_config.get("format").unwrap().as_str().unwrap(), "json"); // Manifest default
    assert_eq!(logger_config.get("output").unwrap().as_str().unwrap(), "stdout"); // User addition
}

#[test]
fn test_dependency_source_and_auto_spawn_overrides() {
    use toml::Table;
    
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test_actors");
    
    // Create user config with dependency source and auto_spawn overrides
    let mut user_config = Table::new();
    let mut dependencies = Table::new();
    let mut logger_override = Table::new();
    
    // Override the source to use a different path
    let alternative_logger_path = test_actors_path.join("logger").to_str().unwrap().to_string();
    logger_override.insert("source".to_string(), toml::Value::Table({
        let mut source_table = Table::new();
        source_table.insert("path".to_string(), toml::Value::String(alternative_logger_path));
        source_table
    }));
    
    // Override auto_spawn to false (manifest has it as true)
    logger_override.insert("auto_spawn".to_string(), toml::Value::Boolean(false));
    
    dependencies.insert("logger".to_string(), toml::Value::Table(logger_override));
    user_config.insert("dependencies".to_string(), toml::Value::Table(dependencies));
    
    let actors = vec![
        Actor {
            name: "my_coordinator".to_string(),
            source: ActorSource::Path(PathSource {
                path: test_actors_path.join("coordinator").to_str().unwrap().to_string(),
                package: None,
            }),
            config: Some(user_config),
            auto_spawn: false,
        }
    ];

    let resolver = DependencyResolver::new();
    let result = resolver.resolve_all(actors);
    
    assert!(result.is_ok());
    let resolved = result.unwrap();
    
    let logger = &resolved["logger"];
    
    // Source should be overridden to the user-specified path
    match &logger.source {
        ActorSource::Path(path_source) => {
            assert!(path_source.path.ends_with("test_actors/logger"));
        }
        _ => panic!("Expected path source"),
    }
    
    // auto_spawn should be overridden to false (manifest default is true)
    assert_eq!(logger.auto_spawn, false);
    
    // Config should still have the manifest defaults since we didn't override them
    let logger_config = logger.config.as_ref().unwrap();
    assert_eq!(logger_config.get("level").unwrap().as_str().unwrap(), "info"); // Manifest default
    assert_eq!(logger_config.get("format").unwrap().as_str().unwrap(), "json"); // Manifest default
}

#[test]
fn test_orphaned_dependency_configuration_warning() {
    use toml::Table;
    
    let test_actors_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test_actors");
    
    // Create user config with an orphaned dependency (non-existent in manifest)
    let mut user_config = Table::new();
    let mut dependencies = Table::new();
    let mut orphaned_dep = Table::new();
    
    orphaned_dep.insert("auto_spawn".to_string(), toml::Value::Boolean(true));
    dependencies.insert("nonexistent_dependency".to_string(), toml::Value::Table(orphaned_dep));
    
    // Also add a valid dependency configuration
    let mut valid_dep = Table::new();
    valid_dep.insert("auto_spawn".to_string(), toml::Value::Boolean(false));
    dependencies.insert("logger".to_string(), toml::Value::Table(valid_dep));
    
    user_config.insert("dependencies".to_string(), toml::Value::Table(dependencies));
    
    let actors = vec![
        Actor {
            name: "my_coordinator".to_string(),
            source: ActorSource::Path(PathSource {
                path: test_actors_path.join("coordinator").to_str().unwrap().to_string(),
                package: None,
            }),
            config: Some(user_config),
            auto_spawn: false,
        }
    ];

    let resolver = DependencyResolver::new();
    let result = resolver.resolve_all(actors);
    
    // Resolution should still succeed despite orphaned config
    assert!(result.is_ok());
    let resolved = result.unwrap();
    
    // Should still resolve both coordinator and logger (ignoring orphaned config)
    assert_eq!(resolved.len(), 2);
    assert!(resolved.contains_key("my_coordinator"));
    assert!(resolved.contains_key("logger"));
    
    // Logger should use the valid override (auto_spawn = false instead of manifest default true)
    let logger = &resolved["logger"];
    assert_eq!(logger.auto_spawn, false);
    
    // Note: The warning for "nonexistent_dependency" would be logged but we can't easily test
    // the log output in a unit test. In a real scenario, users would see:
    // "Configuration for unknown dependency 'nonexistent_dependency' in actor 'my_coordinator' will be ignored."
}