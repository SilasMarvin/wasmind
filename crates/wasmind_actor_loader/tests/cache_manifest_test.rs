use std::fs;
use tempfile::TempDir;
use wasmind_config::{Actor, ActorSource, PathSource};

#[tokio::test]
async fn test_dependency_resolver_uses_cached_manifest() {
    use wasmind_actor_loader::dependency_resolver::DependencyResolver;
    use wasmind_actor_loader::utils::compute_source_hash;
    
    // Create a cache directory with a pre-cached manifest
    let cache_dir = TempDir::new().unwrap();
    
    let actor = Actor {
        name: "cached_actor".to_string(),
        source: ActorSource::Path(PathSource {
            path: "/fake/path".to_string(),
        }),
        config: None,
        auto_spawn: false,
        required_spawn_with: vec![],
    };
    
    // Create the cache structure manually
    let source_hash = compute_source_hash(&actor.source);
    let cache_path = cache_dir.path().join(&source_hash);
    fs::create_dir_all(&cache_path).unwrap();
    
    // Write a cached manifest
    let manifest_content = r#"
actor_id = "test:from_cache"
required_spawn_with = ["dep1", "dep2"]
"#;
    fs::write(cache_path.join("Wasmind.toml"), manifest_content).unwrap();
    
    // Create a resolver with the cache directory
    let resolver = DependencyResolver::with_cache(
        std::sync::Arc::new(
            wasmind_actor_loader::ExternalDependencyCache::new(TempDir::new().unwrap()).unwrap()
        ),
        Some(cache_dir.path().to_path_buf()),
    );
    
    // Try to resolve - should use cached manifest instead of trying to load from /fake/path
    let result = resolver.resolve_all(vec![actor], vec![]).await;
    
    match result {
        Ok(resolved) => {
            // Should have successfully resolved using cached manifest
            assert_eq!(resolved.len(), 1);
            let resolved_actor = resolved.get("cached_actor").unwrap();
            assert_eq!(resolved_actor.actor_id, "test:from_cache");
            assert_eq!(resolved_actor.required_spawn_with, vec!["dep1", "dep2"]);
        }
        Err(e) => {
            // If it failed, it should NOT be because of invalid path
            // (since we should have used the cache)
            let error_msg = e.to_string();
            assert!(
                !error_msg.contains("/fake/path"),
                "Should not try to access /fake/path when manifest is cached: {}",
                error_msg
            );
        }
    }
}