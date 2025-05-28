use snafu::{ResultExt, Snafu};

pub const TOOL_NAME: &str = "read_file";
pub const TOOL_DESCRIPTION: &str = "Read file contents";
pub const TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "path": {
            "type": "string",
            "description": "The path to the file to read"
        }
    },
    "required": ["path"]
}"#;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

// --- Error Handling with Snafu ---
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum FileCacheError {
    #[snafu(display("Failed to read file metadata for '{}': {}", path.display(), source))]
    ReadMetadata { source: io::Error, path: PathBuf },

    #[snafu(display("Failed to read file contents for '{}': {}", path.display(), source))]
    ReadFile { source: io::Error, path: PathBuf },

    #[snafu(display("Failed to get modified time for '{}': {}", path.display(), source))]
    GetModifiedTime { source: io::Error, path: PathBuf },

    #[snafu(display("Failed to canonicalize path '{}': {}", path.display(), source))]
    CanonicalizePath { source: io::Error, path: PathBuf },

    #[snafu(display("File '{}' should be in cache but was not found. This indicates an internal logic error.", path.display()))]
    CacheMissInternal { path: PathBuf },
}

pub type Result<T, E = FileCacheError> = std::result::Result<T, E>;

// --- Structs ---

/// Information stored for each cached file
#[derive(Debug, Clone)]
pub struct FileCacheEntry {
    pub contents: String,
    pub read_at: SystemTime,
    pub last_modified_at_read: SystemTime,
}

/// Manages file reading and caching.
#[derive(Debug, Default)]
pub struct FileReader {
    cache: HashMap<PathBuf, FileCacheEntry>,
}

impl FileReader {
    /// Creates a new, empty FileReader.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read_and_cache_file<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let path_ref = path.as_ref();

        let metadata = fs::metadata(path_ref).context(ReadMetadataSnafu {
            path: path_ref.to_path_buf(),
        })?;
        let last_modified_at_read = metadata.modified().context(GetModifiedTimeSnafu {
            path: path_ref.to_path_buf(),
        })?;

        let contents = fs::read_to_string(path_ref).context(ReadFileSnafu {
            path: path_ref.to_path_buf(),
        })?;

        let read_at = SystemTime::now();

        let entry = FileCacheEntry {
            contents,
            read_at,
            last_modified_at_read,
        };

        let canonical_path = fs::canonicalize(path_ref).context(CanonicalizePathSnafu {
            path: path_ref.to_path_buf(),
        })?;
        self.cache.insert(canonical_path, entry);

        Ok(())
    }

    pub fn has_been_modified<P: AsRef<Path>>(&self, path: P) -> Result<bool> {
        let path_ref = path.as_ref();

        let canonical_path_lookup = match fs::canonicalize(path_ref) {
            Ok(p) => p,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(true);
            }
            // FIX 1: Provide `source: e` when calling .fail() for variants that expect a source
            Err(e) => {
                return Err(FileCacheError::CanonicalizePath {
                    source: e,
                    path: path_ref.to_path_buf(),
                });
            }
        };

        if let Some(cached_entry) = self.cache.get(&canonical_path_lookup) {
            match fs::metadata(&canonical_path_lookup) {
                Ok(current_metadata) => {
                    let current_mtime =
                        current_metadata.modified().context(GetModifiedTimeSnafu {
                            path: canonical_path_lookup.clone(),
                        })?;
                    Ok(current_mtime != cached_entry.last_modified_at_read)
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(true),
                // FIX 2: Provide `source: e` when calling .fail()
                Err(e) => {
                    return Err(FileCacheError::ReadMetadata {
                        source: e,
                        path: canonical_path_lookup.clone(),
                    });
                }
            }
        } else {
            Ok(true)
        }
    }

    pub fn get_cached_content<P: AsRef<Path>>(&self, path: P) -> Option<&String> {
        fs::canonicalize(path.as_ref())
            .ok()
            .and_then(|p| self.cache.get(&p))
            .map(|entry| &entry.contents)
    }

    pub fn get_or_read_file_content<P: AsRef<Path> + Clone>(&mut self, path: P) -> Result<&String> {
        let path_ref = path.as_ref();
        let needs_read = self.has_been_modified(path_ref)?;

        if needs_read {
            self.read_and_cache_file(path.clone())?;
        }

        let canonical_path = fs::canonicalize(path_ref).context(CanonicalizePathSnafu {
            path: path_ref.to_path_buf(),
        })?;

        self.cache
            .get(&canonical_path)
            .map(|entry| &entry.contents)
            .ok_or_else(|| FileCacheError::CacheMissInternal {
                path: canonical_path.clone(),
            })
    }

    pub fn remove_from_cache<P: AsRef<Path>>(&mut self, path: P) -> Option<FileCacheEntry> {
        fs::canonicalize(path.as_ref())
            .ok()
            .and_then(|p| self.cache.remove(&p))
    }

    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    pub fn list_cached_paths(&self) -> Vec<&PathBuf> {
        self.cache.keys().collect()
    }
}

// --- Tests ---
#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use std::fs::{self, File};
    use std::io::Write;
    use std::thread;
    use std::time::Duration;
    use tempfile::TempDir; // Only if needed for specific FromResidual cases, likely not here.

    // FIX 3: Add use statement for pathdiff
    use pathdiff;

    fn create_temp_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        let mut file = File::create(&path).expect("Failed to create temp file");
        writeln!(file, "{}", content).expect("Failed to write to temp file");
        drop(file);
        thread::sleep(Duration::from_millis(20));
        path
    }

    fn modify_temp_file(path: &Path, new_content: &str) {
        thread::sleep(Duration::from_millis(100));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)
            .expect("Failed to open temp file for modification");
        writeln!(file, "{}", new_content).expect("Failed to write modified content");
        drop(file);
        thread::sleep(Duration::from_millis(20));
    }

    #[test]
    fn test_new_and_read_file() -> Result<()> {
        let mut reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        let file_path = create_temp_file(&tmp_dir, "test1.txt", "Hello World");

        let content = reader.get_or_read_file_content(&file_path)?;
        assert_eq!(content, "Hello World\n");

        let canonical_file_path = fs::canonicalize(&file_path).context(CanonicalizePathSnafu {
            path: file_path.clone(),
        })?; // Context for test Result
        assert!(reader.cache.contains_key(&canonical_file_path));
        let entry = reader.cache.get(&canonical_file_path).unwrap();
        assert_eq!(entry.contents, "Hello World\n");
        Ok(())
    }

    #[test]
    fn test_get_cached_content_no_modification() -> Result<()> {
        let mut reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        let file_path = create_temp_file(&tmp_dir, "test2.txt", "Cache Me");

        reader.get_or_read_file_content(&file_path)?;

        let modified_check_before_get = reader.has_been_modified(&file_path)?;
        assert!(
            !modified_check_before_get,
            "File should not be marked as modified yet"
        );

        let content = reader.get_or_read_file_content(&file_path)?;
        assert_eq!(content, "Cache Me\n");
        Ok(())
    }

    #[test]
    fn test_file_modification_detection_and_reread() -> Result<()> {
        let mut reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        let file_path = create_temp_file(&tmp_dir, "test3.txt", "Initial Content");

        let initial_content = reader.get_or_read_file_content(&file_path)?;
        assert_eq!(initial_content, "Initial Content\n");
        assert!(
            !reader.has_been_modified(&file_path)?,
            "File should not be modified immediately after read"
        );

        modify_temp_file(&file_path, "Updated Content");

        assert!(
            reader.has_been_modified(&file_path)?,
            "File should be detected as modified"
        );

        let updated_content = reader.get_or_read_file_content(&file_path)?;
        assert_eq!(updated_content, "Updated Content\n");

        assert!(
            !reader.has_been_modified(&file_path)?,
            "File should not be modified after re-read"
        );
        Ok(())
    }

    #[test]
    fn test_file_deletion() -> Result<()> {
        let mut reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        let file_path = create_temp_file(&tmp_dir, "test_delete.txt", "Content to delete");

        reader.get_or_read_file_content(&file_path)?;
        assert!(!reader.has_been_modified(&file_path)?);

        fs::remove_file(&file_path).unwrap();

        assert!(
            reader.has_been_modified(&file_path)?,
            "Deleted file should be marked as modified"
        );

        match reader.get_or_read_file_content(&file_path) {
            Ok(_) => panic!("Should have failed to read deleted file"),
            Err(e) => {
                match e {
                    FileCacheError::ReadMetadata { source, .. }
                    | FileCacheError::ReadFile { source, .. } => {
                        assert_eq!(source.kind(), io::ErrorKind::NotFound);
                    }
                    // CanonicalizePath might also fail with NotFound if get_or_read_file_content calls it first
                    FileCacheError::CanonicalizePath { source, .. } => {
                        assert_eq!(source.kind(), io::ErrorKind::NotFound);
                    }
                    _ => panic!("Unexpected error type: {:?}", e),
                }
            }
        }
        Ok(())
    }

    #[test]
    fn test_relative_and_canonical_paths() -> Result<()> {
        let mut reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();

        let sub_dir = tmp_dir.path().join("sub");
        fs::create_dir(&sub_dir).unwrap();
        let file_path_abs = create_temp_file(&tmp_dir, "sub/test_rel.txt", "Relative Test");

        let current_dir = std::env::current_dir().unwrap();
        let relative_path_to_file = pathdiff::diff_paths(&file_path_abs, current_dir).unwrap();

        let content1 = reader.get_or_read_file_content(&relative_path_to_file)?;
        assert_eq!(content1, "Relative Test\n");
        assert_eq!(reader.cache.len(), 1, "Cache should have one entry");

        let content2 = reader.get_or_read_file_content(&file_path_abs)?;
        assert_eq!(content2, "Relative Test\n");
        assert_eq!(
            reader.cache.len(),
            1,
            "Cache should still have one entry (same file)"
        );

        let canonical_path = fs::canonicalize(&file_path_abs).context(CanonicalizePathSnafu {
            path: file_path_abs.clone(),
        })?;
        assert!(reader.cache.contains_key(&canonical_path));
        Ok(())
    }

    #[test]
    fn test_get_cached_content_direct_access() -> Result<()> {
        let mut reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        let file_path = create_temp_file(&tmp_dir, "test_direct.txt", "Direct Access");

        assert!(reader.get_cached_content(&file_path).is_none());
        reader.get_or_read_file_content(&file_path)?;

        let cached_content = reader
            .get_cached_content(&file_path)
            .expect("Should be in cache");
        assert_eq!(cached_content, "Direct Access\n");

        modify_temp_file(&file_path, "Stale Data Check");
        let stale_cached_content = reader
            .get_cached_content(&file_path)
            .expect("Should still be in cache (stale)");
        assert_eq!(stale_cached_content, "Direct Access\n");

        let fresh_content = reader.get_or_read_file_content(&file_path)?;
        assert_eq!(fresh_content, "Stale Data Check\n");

        let updated_cached_content = reader
            .get_cached_content(&file_path)
            .expect("Should be updated in cache");
        assert_eq!(updated_cached_content, "Stale Data Check\n");
        Ok(())
    }

    #[test]
    fn test_remove_from_cache() -> Result<()> {
        let mut reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        let file_path = create_temp_file(&tmp_dir, "test_remove.txt", "Remove Me");

        reader.get_or_read_file_content(&file_path)?;
        assert!(reader.get_cached_content(&file_path).is_some());
        assert_eq!(reader.cache.len(), 1);

        let removed_entry = reader.remove_from_cache(&file_path);
        assert!(removed_entry.is_some());
        assert_eq!(removed_entry.unwrap().contents, "Remove Me\n");

        assert!(reader.get_cached_content(&file_path).is_none());
        assert!(reader.cache.is_empty());

        assert!(reader.remove_from_cache(&file_path).is_none());
        let non_existent_path = tmp_dir.path().join("ghost.txt");
        assert!(reader.remove_from_cache(&non_existent_path).is_none());
        Ok(())
    }

    #[test]
    fn test_clear_cache() -> Result<()> {
        let mut reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        let file_path1 = create_temp_file(&tmp_dir, "test_clear1.txt", "Clear Cache 1");
        let file_path2 = create_temp_file(&tmp_dir, "test_clear2.txt", "Clear Cache 2");

        reader.get_or_read_file_content(&file_path1)?;
        reader.get_or_read_file_content(&file_path2)?;
        assert_eq!(reader.cache.len(), 2);
        assert!(!reader.list_cached_paths().is_empty());

        reader.clear_cache();
        assert!(reader.cache.is_empty());
        assert!(reader.list_cached_paths().is_empty());
        Ok(())
    }

    #[test]
    fn test_non_existent_file_read_attempt() {
        let mut reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        let non_existent_path = tmp_dir.path().join("i_do_not_exist.txt");

        match reader.get_or_read_file_content(&non_existent_path) {
            Ok(_) => panic!("Should have failed to read non-existent file"),
            Err(e) => match e {
                FileCacheError::ReadMetadata { source, path } => {
                    assert_eq!(source.kind(), io::ErrorKind::NotFound);
                    assert_eq!(path, non_existent_path);
                }
                FileCacheError::CanonicalizePath { source, path } => {
                    assert_eq!(source.kind(), io::ErrorKind::NotFound);
                    assert_eq!(path, non_existent_path);
                }
                _ => panic!("Unexpected error type: {:?}", e),
            },
        }
        assert!(reader.cache.is_empty());
    }

    #[test]
    fn test_has_been_modified_for_non_existent_file() -> Result<()> {
        let reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        let non_existent_path = tmp_dir.path().join("ghost_file.txt");
        assert!(reader.has_been_modified(&non_existent_path)?);
        Ok(())
    }

    #[test]
    fn test_list_cached_paths() -> Result<()> {
        let mut reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        assert!(reader.list_cached_paths().is_empty());

        let file_path1 = create_temp_file(&tmp_dir, "list_test1.txt", "File 1");
        let file_path2 = create_temp_file(&tmp_dir, "list_test2.txt", "File 2");

        reader.get_or_read_file_content(&file_path1)?;
        let paths = reader.list_cached_paths();
        assert_eq!(paths.len(), 1);
        // FIX 5 & 6: Use .context for fs::canonicalize and handle Option from remove_from_cache
        assert!(paths.contains(&&fs::canonicalize(&file_path1).context(
            CanonicalizePathSnafu {
                path: file_path1.clone()
            }
        )?));

        reader.get_or_read_file_content(&file_path2)?;
        let paths = reader.list_cached_paths();
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&&fs::canonicalize(&file_path1).context(
            CanonicalizePathSnafu {
                path: file_path1.clone()
            }
        )?));
        assert!(paths.contains(&&fs::canonicalize(&file_path2).context(
            CanonicalizePathSnafu {
                path: file_path2.clone()
            }
        )?));

        // FIX 6: Change ? on Option to an explicit assertion or handling
        assert!(
            reader.remove_from_cache(&file_path1).is_some(),
            "Expected file_path1 to be removed from cache"
        );
        let paths = reader.list_cached_paths();
        assert_eq!(paths.len(), 1);
        assert!(paths.contains(&&fs::canonicalize(&file_path2).context(
            CanonicalizePathSnafu {
                path: file_path2.clone()
            }
        )?));

        reader.clear_cache();
        assert!(reader.list_cached_paths().is_empty());
        Ok(())
    }
}
