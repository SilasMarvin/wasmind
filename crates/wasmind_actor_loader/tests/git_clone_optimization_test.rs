use std::fs;
use tempfile::TempDir;
use wasmind_actor_loader::dependency_resolver::DependencyResolver;
use wasmind_actor_loader::utils::compute_source_hash;
use wasmind_config::{Actor, ActorSource, Repository};

#[tokio::test]
async fn test_git_clone_avoided_when_manifest_cached() {
    // Create a cache directory with a pre-cached git manifest
    let cache_dir = TempDir::new().unwrap();

    // Create a git source that points to a non-existent repository
    // This will fail if we actually try to clone it
    let fake_git_source = Repository {
        git: url::Url::parse("https://github.com/fake/nonexistent-repo.git").unwrap(),
        git_ref: Some(wasmind_config::GitRef::Branch("main".to_string())),
        sub_dir: None,
    };

    let actor = Actor {
        name: "git_dependency".to_string(),
        source: ActorSource::Git(fake_git_source.clone()),
        config: None,
        auto_spawn: false,
        required_spawn_with: vec![],
    };

    // Pre-cache a manifest for this git source
    let source_hash = compute_source_hash(&actor.source);
    let cache_path = cache_dir.path().join(&source_hash);
    fs::create_dir_all(&cache_path).unwrap();

    let manifest_content = r#"
actor_id = "test:git_cached"
required_spawn_with = ["helper"]

[dependencies.helper]
source = { path = "./helper" }
"#;
    fs::write(cache_path.join("Wasmind.toml"), manifest_content).unwrap();

    // Create a resolver with the cache directory
    let resolver = DependencyResolver::new(
        std::sync::Arc::new(
            wasmind_actor_loader::ExternalDependencyCache::new(TempDir::new().unwrap()).unwrap(),
        ),
        Some(cache_dir.path().to_path_buf()),
    );

    // Try to resolve - this should succeed using the cached manifest
    // WITHOUT attempting to clone the fake repository
    let result = resolver.resolve_all(vec![actor], vec![]).await;

    match result {
        Ok(resolved) => {
            // Should successfully resolve using cached manifest
            assert_eq!(resolved.len(), 1);
            let resolved_actor = resolved.get("git_dependency").unwrap();
            assert_eq!(resolved_actor.actor_id, "test:git_cached");
            assert_eq!(resolved_actor.required_spawn_with, vec!["helper"]);
            println!("✅ Successfully resolved git dependency using cached manifest");
        }
        Err(e) => {
            let error_msg = e.to_string();

            // If it failed due to dependency resolution (like missing "helper"), that's expected
            // But it should NOT fail due to git clone errors
            if error_msg.contains("git")
                || error_msg.contains("clone")
                || error_msg.contains("Failed to execute command")
            {
                panic!(
                    "❌ Failed due to git clone attempt - optimization not working: {}",
                    error_msg
                );
            } else {
                // Other errors (like missing dependency) are acceptable for this test
                println!(
                    "ℹ️  Failed due to dependency issues (expected): {}",
                    error_msg
                );
            }
        }
    }
}

#[tokio::test]
async fn test_same_git_source_different_names_use_same_cache() {
    // Test that the same git repository used by different actor names
    // results in the same cache key (avoiding duplicate caching)

    let git_source = Repository {
        git: url::Url::parse("https://github.com/example/shared-dependency.git").unwrap(),
        git_ref: Some(wasmind_config::GitRef::Tag("v1.0.0".to_string())),
        sub_dir: None,
    };

    let actor1 = Actor {
        name: "app_a".to_string(),
        source: ActorSource::Git(git_source.clone()),
        config: None,
        auto_spawn: false,
        required_spawn_with: vec![],
    };

    let actor2 = Actor {
        name: "app_b".to_string(),
        source: ActorSource::Git(git_source.clone()),
        config: None,
        auto_spawn: false,
        required_spawn_with: vec![],
    };

    let hash1 = compute_source_hash(&actor1.source);
    let hash2 = compute_source_hash(&actor2.source);

    // Same git source should result in same cache key regardless of actor name
    assert_eq!(
        hash1, hash2,
        "Same git source should have same cache key regardless of actor name"
    );

    // Different actor names should NOT affect the source hash
    assert_ne!(
        &actor1.name, &actor2.name,
        "Sanity check: actor names are different"
    );

    println!("✅ Verified same git source produces same cache key for different actor names");
}
