use snafu::{ResultExt, Snafu};
use std::fs;
use std::path::{Path, PathBuf};

pub const TOOL_NAME: &str = "edit_file";
pub const TOOL_DESCRIPTION: &str = "Edit file contents with various operations like insert, delete, or replace text";
pub const TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "path": {
            "type": "string",
            "description": "The path to the file to edit"
        },
        "action": {
            "type": "string",
            "enum": ["insert_at_start", "insert_at_end", "delete", "replace", "insert_before", "insert_after"],
            "description": "The action to perform on the file"
        },
        "search_string": {
            "type": "string",
            "description": "The text to search for (required for delete, replace, insert_before, insert_after actions)"
        },
        "replacement_text": {
            "type": "string",
            "description": "The text to insert or use as replacement (required for insert_*, replace actions)"
        },
        "expected_occurrences": {
            "type": "integer",
            "description": "The expected number of occurrences to replace (required for replace action)",
            "minimum": 1
        }
    },
    "required": ["path", "action"]
}"#;

// --- Error Handling with Snafu ---
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum EditFileError {
    #[snafu(display("File '{}' has not been read yet. Please use the read_file tool first.", path.display()))]
    FileNotRead { path: PathBuf },

    #[snafu(display("File '{}' has been modified since last read. Please use the read_file tool to refresh the file contents.", path.display()))]
    FileModified { path: PathBuf },

    #[snafu(display("Search string '{}' not found in file '{}'", search_string, path.display()))]
    SearchStringNotFound { search_string: String, path: PathBuf },

    #[snafu(display("Expected {} occurrences of '{}' in file '{}', but found {}", expected, search_string, path.display(), actual))]
    OccurrenceMismatch {
        expected: usize,
        actual: usize,
        search_string: String,
        path: PathBuf,
    },

    #[snafu(display("Missing required field '{}' for action '{}'", field, action))]
    MissingRequiredField { field: String, action: String },

    #[snafu(display("Failed to write file '{}': {}", path.display(), source))]
    WriteFile { source: std::io::Error, path: PathBuf },

    #[snafu(display("Failed to canonicalize path '{}': {}", path.display(), source))]
    CanonicalizePath { source: std::io::Error, path: PathBuf },
}

pub type Result<T, E = EditFileError> = std::result::Result<T, E>;

/// Actions that can be performed on a file
#[derive(Debug, Clone, PartialEq)]
pub enum EditAction {
    InsertAtStart { text: String },
    InsertAtEnd { text: String },
    Delete { search_string: String },
    Replace { 
        search_string: String, 
        replacement_text: String, 
        expected_occurrences: usize 
    },
    InsertBefore { search_string: String, text: String },
    InsertAfter { search_string: String, text: String },
}

/// File editor that works with the FileReader cache
pub struct FileEditor;

impl FileEditor {
    pub fn new() -> Self {
        Self
    }

    /// Edit a file using the provided FileReader cache
    pub fn edit_file<P: AsRef<Path>>(
        &self,
        path: P,
        action: EditAction,
        file_reader: &mut crate::tools::file_reader::FileReader,
    ) -> Result<String> {
        let path_ref = path.as_ref();
        let canonical_path = fs::canonicalize(path_ref).context(CanonicalizePathSnafu {
            path: path_ref.to_path_buf(),
        })?;

        // Check if file has been read and is up to date
        if file_reader.get_cached_content(&canonical_path).is_none() {
            return Err(EditFileError::FileNotRead {
                path: canonical_path,
            });
        }

        if file_reader.has_been_modified(&canonical_path)
            .map_err(|_| EditFileError::FileModified { path: canonical_path.clone() })? {
            return Err(EditFileError::FileModified {
                path: canonical_path,
            });
        }

        // Get the current file content from cache
        let current_content = file_reader.get_cached_content(&canonical_path)
            .expect("Content should be available after checks")
            .clone();

        // Apply the edit action
        let new_content = self.apply_edit_action(&current_content, &action, &canonical_path)?;

        // Write the new content to disk
        fs::write(&canonical_path, &new_content).context(WriteFileSnafu {
            path: canonical_path.clone(),
        })?;

        // Update the cache with new content
        file_reader.read_and_cache_file(&canonical_path)
            .map_err(|_| EditFileError::FileModified { path: canonical_path.clone() })?;

        Ok(format!("Successfully edited file: {}", canonical_path.display()))
    }

    fn apply_edit_action(
        &self,
        content: &str,
        action: &EditAction,
        path: &Path,
    ) -> Result<String> {
        match action {
            EditAction::InsertAtStart { text } => {
                Ok(format!("{}{}", text, content))
            }
            EditAction::InsertAtEnd { text } => {
                Ok(format!("{}{}", content, text))
            }
            EditAction::Delete { search_string } => {
                if !content.contains(search_string) {
                    return Err(EditFileError::SearchStringNotFound {
                        search_string: search_string.clone(),
                        path: path.to_path_buf(),
                    });
                }
                Ok(content.replace(search_string, ""))
            }
            EditAction::Replace { 
                search_string, 
                replacement_text, 
                expected_occurrences 
            } => {
                let actual_occurrences = content.matches(search_string).count();
                if actual_occurrences != *expected_occurrences {
                    return Err(EditFileError::OccurrenceMismatch {
                        expected: *expected_occurrences,
                        actual: actual_occurrences,
                        search_string: search_string.clone(),
                        path: path.to_path_buf(),
                    });
                }
                Ok(content.replace(search_string, replacement_text))
            }
            EditAction::InsertBefore { search_string, text } => {
                if !content.contains(search_string) {
                    return Err(EditFileError::SearchStringNotFound {
                        search_string: search_string.clone(),
                        path: path.to_path_buf(),
                    });
                }
                Ok(content.replace(search_string, &format!("{}{}", text, search_string)))
            }
            EditAction::InsertAfter { search_string, text } => {
                if !content.contains(search_string) {
                    return Err(EditFileError::SearchStringNotFound {
                        search_string: search_string.clone(),
                        path: path.to_path_buf(),
                    });
                }
                Ok(content.replace(search_string, &format!("{}{}", search_string, text)))
            }
        }
    }

    /// Parse edit action from JSON arguments
    pub fn parse_action_from_args(args: &serde_json::Value) -> Result<EditAction> {
        let action = args.get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| EditFileError::MissingRequiredField {
                field: "action".to_string(),
                action: "unknown".to_string(),
            })?;

        match action {
            "insert_at_start" => {
                let text = args.get("replacement_text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| EditFileError::MissingRequiredField {
                        field: "replacement_text".to_string(),
                        action: action.to_string(),
                    })?;
                Ok(EditAction::InsertAtStart { text: text.to_string() })
            }
            "insert_at_end" => {
                let text = args.get("replacement_text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| EditFileError::MissingRequiredField {
                        field: "replacement_text".to_string(),
                        action: action.to_string(),
                    })?;
                Ok(EditAction::InsertAtEnd { text: text.to_string() })
            }
            "delete" => {
                let search_string = args.get("search_string")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| EditFileError::MissingRequiredField {
                        field: "search_string".to_string(),
                        action: action.to_string(),
                    })?;
                Ok(EditAction::Delete { search_string: search_string.to_string() })
            }
            "replace" => {
                let search_string = args.get("search_string")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| EditFileError::MissingRequiredField {
                        field: "search_string".to_string(),
                        action: action.to_string(),
                    })?;
                let replacement_text = args.get("replacement_text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| EditFileError::MissingRequiredField {
                        field: "replacement_text".to_string(),
                        action: action.to_string(),
                    })?;
                let expected_occurrences = args.get("expected_occurrences")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| EditFileError::MissingRequiredField {
                        field: "expected_occurrences".to_string(),
                        action: action.to_string(),
                    })? as usize;
                Ok(EditAction::Replace {
                    search_string: search_string.to_string(),
                    replacement_text: replacement_text.to_string(),
                    expected_occurrences,
                })
            }
            "insert_before" => {
                let search_string = args.get("search_string")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| EditFileError::MissingRequiredField {
                        field: "search_string".to_string(),
                        action: action.to_string(),
                    })?;
                let text = args.get("replacement_text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| EditFileError::MissingRequiredField {
                        field: "replacement_text".to_string(),
                        action: action.to_string(),
                    })?;
                Ok(EditAction::InsertBefore {
                    search_string: search_string.to_string(),
                    text: text.to_string(),
                })
            }
            "insert_after" => {
                let search_string = args.get("search_string")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| EditFileError::MissingRequiredField {
                        field: "search_string".to_string(),
                        action: action.to_string(),
                    })?;
                let text = args.get("replacement_text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| EditFileError::MissingRequiredField {
                        field: "replacement_text".to_string(),
                        action: action.to_string(),
                    })?;
                Ok(EditAction::InsertAfter {
                    search_string: search_string.to_string(),
                    text: text.to_string(),
                })
            }
            _ => Err(EditFileError::MissingRequiredField {
                field: "action".to_string(),
                action: format!("unknown action: {}", action),
            }),
        }
    }
}

impl Default for FileEditor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::file_reader::FileReader;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_temp_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        let mut file = File::create(&path).expect("Failed to create temp file");
        write!(file, "{}", content).expect("Failed to write to temp file");
        path
    }

    #[test]
    fn test_edit_file_not_read() {
        let editor = FileEditor::new();
        let mut file_reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        let file_path = create_temp_file(&tmp_dir, "test.txt", "Hello World");

        let action = EditAction::InsertAtEnd { text: "\nNew line".to_string() };
        let result = editor.edit_file(&file_path, action, &mut file_reader);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), EditFileError::FileNotRead { .. }));
    }

    #[test]
    fn test_insert_at_start() -> Result<()> {
        let editor = FileEditor::new();
        let mut file_reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        let file_path = create_temp_file(&tmp_dir, "test.txt", "Hello World");

        // First read the file
        file_reader.get_or_read_file_content(&file_path).unwrap();

        let action = EditAction::InsertAtStart { text: "Start: ".to_string() };
        let result = editor.edit_file(&file_path, action, &mut file_reader)?;

        assert!(result.contains("Successfully edited"));
        
        let new_content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(new_content, "Start: Hello World");
        Ok(())
    }

    #[test]
    fn test_insert_at_end() -> Result<()> {
        let editor = FileEditor::new();
        let mut file_reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        let file_path = create_temp_file(&tmp_dir, "test.txt", "Hello World");

        file_reader.get_or_read_file_content(&file_path).unwrap();

        let action = EditAction::InsertAtEnd { text: " - End".to_string() };
        let result = editor.edit_file(&file_path, action, &mut file_reader)?;

        assert!(result.contains("Successfully edited"));
        
        let new_content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(new_content, "Hello World - End");
        Ok(())
    }

    #[test]
    fn test_replace_with_correct_occurrences() -> Result<()> {
        let editor = FileEditor::new();
        let mut file_reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        let file_path = create_temp_file(&tmp_dir, "test.txt", "Hello World Hello");

        file_reader.get_or_read_file_content(&file_path).unwrap();

        let action = EditAction::Replace {
            search_string: "Hello".to_string(),
            replacement_text: "Hi".to_string(),
            expected_occurrences: 2,
        };
        let result = editor.edit_file(&file_path, action, &mut file_reader)?;

        assert!(result.contains("Successfully edited"));
        
        let new_content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(new_content, "Hi World Hi");
        Ok(())
    }

    #[test]
    fn test_replace_with_wrong_occurrences() {
        let editor = FileEditor::new();
        let mut file_reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        let file_path = create_temp_file(&tmp_dir, "test.txt", "Hello World");

        file_reader.get_or_read_file_content(&file_path).unwrap();

        let action = EditAction::Replace {
            search_string: "Hello".to_string(),
            replacement_text: "Hi".to_string(),
            expected_occurrences: 2, // Should be 1
        };
        let result = editor.edit_file(&file_path, action, &mut file_reader);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), EditFileError::OccurrenceMismatch { .. }));
    }

    #[test]
    fn test_insert_before() -> Result<()> {
        let editor = FileEditor::new();
        let mut file_reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        let file_path = create_temp_file(&tmp_dir, "test.txt", "Hello World");

        file_reader.get_or_read_file_content(&file_path).unwrap();

        let action = EditAction::InsertBefore {
            search_string: "World".to_string(),
            text: "Beautiful ".to_string(),
        };
        let result = editor.edit_file(&file_path, action, &mut file_reader)?;

        assert!(result.contains("Successfully edited"));
        
        let new_content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(new_content, "Hello Beautiful World");
        Ok(())
    }

    #[test]
    fn test_insert_after() -> Result<()> {
        let editor = FileEditor::new();
        let mut file_reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        let file_path = create_temp_file(&tmp_dir, "test.txt", "Hello World");

        file_reader.get_or_read_file_content(&file_path).unwrap();

        let action = EditAction::InsertAfter {
            search_string: "Hello".to_string(),
            text: " Beautiful".to_string(),
        };
        let result = editor.edit_file(&file_path, action, &mut file_reader)?;

        assert!(result.contains("Successfully edited"));
        
        let new_content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(new_content, "Hello Beautiful World");
        Ok(())
    }

    #[test]
    fn test_delete_text() -> Result<()> {
        let editor = FileEditor::new();
        let mut file_reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        let file_path = create_temp_file(&tmp_dir, "test.txt", "Hello Beautiful World");

        file_reader.get_or_read_file_content(&file_path).unwrap();

        let action = EditAction::Delete {
            search_string: " Beautiful".to_string(),
        };
        let result = editor.edit_file(&file_path, action, &mut file_reader)?;

        assert!(result.contains("Successfully edited"));
        
        let new_content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(new_content, "Hello World");
        Ok(())
    }

    #[test]
    fn test_search_string_not_found() {
        let editor = FileEditor::new();
        let mut file_reader = FileReader::new();
        let tmp_dir = TempDir::new().unwrap();
        let file_path = create_temp_file(&tmp_dir, "test.txt", "Hello World");

        file_reader.get_or_read_file_content(&file_path).unwrap();

        let action = EditAction::Delete {
            search_string: "NotFound".to_string(),
        };
        let result = editor.edit_file(&file_path, action, &mut file_reader);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), EditFileError::SearchStringNotFound { .. }));
    }

    #[test]
    fn test_parse_action_from_args() -> Result<()> {
        let args = serde_json::json!({
            "action": "replace",
            "search_string": "old",
            "replacement_text": "new",
            "expected_occurrences": 2
        });

        let action = FileEditor::parse_action_from_args(&args)?;
        
        assert_eq!(action, EditAction::Replace {
            search_string: "old".to_string(),
            replacement_text: "new".to_string(),
            expected_occurrences: 2,
        });
        Ok(())
    }
}