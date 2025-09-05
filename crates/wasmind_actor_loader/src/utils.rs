use sha2::{Digest, Sha256};
use wasmind_config::{ActorSource, GitRef, Repository};

/// Compute a hash for a git repository source
pub fn compute_git_source_hash(git_source: &Repository) -> String {
    let mut hasher = Sha256::new();
    hasher.update("git:");
    hasher.update(git_source.git.as_str());
    if let Some(git_ref) = &git_source.git_ref {
        match git_ref {
            GitRef::Branch(branch) => hasher.update(format!("branch:{branch}")),
            GitRef::Tag(tag) => hasher.update(format!("tag:{tag}")),
            GitRef::Rev(rev) => hasher.update(format!("rev:{rev}")),
        }
    }
    if let Some(sub_dir) = &git_source.sub_dir {
        hasher.update("sub_dir:");
        hasher.update(sub_dir);
    }
    hex::encode(hasher.finalize())
}

/// Compute a hash for the actor source only (no logical name)
pub fn compute_source_hash(source: &ActorSource) -> String {
    match source {
        ActorSource::Path(path_source) => {
            let mut hasher = Sha256::new();
            hasher.update("path:");
            hasher.update(&path_source.path);
            hex::encode(hasher.finalize())
        }
        ActorSource::Git(repo) => {
            // Use the shared git hash computation
            compute_git_source_hash(repo)
        }
    }
}
