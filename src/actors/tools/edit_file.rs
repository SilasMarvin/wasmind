use genai::chat::{Tool, ToolCall};
use snafu::{ResultExt, Snafu};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use tracing::info;

use crate::actors::{Actor, ActorMessage, Message, ToolCallStatus, ToolCallType, ToolCallUpdate};
use crate::config::ParsedConfig;
use crate::scope::Scope;

pub const TOOL_NAME: &str = "edit_file";
pub const TOOL_DESCRIPTION: &str =
    "Edit file contents with various operations like insert, delete, or replace text";
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
    SearchStringNotFound {
        search_string: String,
        path: PathBuf,
    },

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
    WriteFile {
        source: std::io::Error,
        path: PathBuf,
    },

    #[snafu(display("Failed to canonicalize path '{}': {}", path.display(), source))]
    CanonicalizePath {
        source: std::io::Error,
        path: PathBuf,
    },
}

pub type Result<T, E = EditFileError> = std::result::Result<T, E>;

/// Actions that can be performed on a file
#[derive(Debug, Clone, PartialEq)]
pub enum EditAction {
    InsertAtStart {
        text: String,
    },
    InsertAtEnd {
        text: String,
    },
    Delete {
        search_string: String,
    },
    Replace {
        search_string: String,
        replacement_text: String,
        expected_occurrences: usize,
    },
    InsertBefore {
        search_string: String,
        text: String,
    },
    InsertAfter {
        search_string: String,
        text: String,
    },
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
        file_reader: &mut super::file_reader::FileReader,
    ) -> Result<String> {
        let path_ref = path.as_ref();
        
        // Special case: InsertAtStart on non-existent file - create it first
        if !path_ref.exists() && matches!(action, EditAction::InsertAtStart { .. }) {
            return self.create_file_with_insert_at_start(path_ref, action, file_reader);
        }
        
        // Check if the file exists first to provide a better error message
        if !path_ref.exists() {
            return Err(EditFileError::CanonicalizePath {
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("File does not exist. Please create the file first using a different tool or verify the path is correct.")
                ),
                path: path_ref.to_path_buf(),
            });
        }
        
        let canonical_path = fs::canonicalize(path_ref).context(CanonicalizePathSnafu {
            path: path_ref.to_path_buf(),
        })?;

        // Check if file has been read and is up to date
        if file_reader.get_cached_content(&canonical_path).is_none() {
            return Err(EditFileError::FileNotRead {
                path: canonical_path,
            });
        }

        if file_reader
            .has_been_modified(&canonical_path)
            .map_err(|_| EditFileError::FileModified {
                path: canonical_path.clone(),
            })?
        {
            return Err(EditFileError::FileModified {
                path: canonical_path,
            });
        }

        // Get the current file content from cache
        let current_content = file_reader
            .get_cached_content(&canonical_path)
            .expect("Content should be available after checks")
            .clone();

        // Apply the edit action
        let new_content = self.apply_edit_action(&current_content, &action, &canonical_path)?;

        // Write the new content to disk
        fs::write(&canonical_path, &new_content).context(WriteFileSnafu {
            path: canonical_path.clone(),
        })?;

        // Update the cache with new content
        file_reader
            .read_and_cache_file(&canonical_path)
            .map_err(|_| EditFileError::FileModified {
                path: canonical_path.clone(),
            })?;

        Ok(format!(
            "Successfully edited file: {}",
            canonical_path.display()
        ))
    }

    /// Create a new file with InsertAtStart action
    fn create_file_with_insert_at_start<P: AsRef<Path>>(
        &self,
        path: P,
        action: EditAction,
        file_reader: &mut super::file_reader::FileReader,
    ) -> Result<String> {
        let path_ref = path.as_ref();
        
        // Extract the text to insert
        let text = match action {
            EditAction::InsertAtStart { text } => text,
            _ => return Err(EditFileError::CanonicalizePath {
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Expected InsertAtStart action".to_string()
                ),
                path: path_ref.to_path_buf(),
            }),
        };
        
        // Create any necessary parent directories
        if let Some(parent) = path_ref.parent() {
            fs::create_dir_all(parent).context(WriteFileSnafu {
                path: path_ref.to_path_buf(),
            })?;
        }
        
        // Write the content to the new file (starting with empty content + inserted text)
        let new_content = format!("{}", text);
        fs::write(path_ref, &new_content).context(WriteFileSnafu {
            path: path_ref.to_path_buf(),
        })?;
        
        // Now read and cache the file so it's available for future operations
        file_reader
            .read_and_cache_file(path_ref)
            .map_err(|_| EditFileError::FileModified {
                path: path_ref.to_path_buf(),
            })?;
        
        Ok(format!(
            "Successfully created and edited file: {}",
            path_ref.display()
        ))
    }

    fn apply_edit_action(&self, content: &str, action: &EditAction, path: &Path) -> Result<String> {
        match action {
            EditAction::InsertAtStart { text } => Ok(format!("{}{}", text, content)),
            EditAction::InsertAtEnd { text } => Ok(format!("{}{}", content, text)),
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
                expected_occurrences,
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
            EditAction::InsertBefore {
                search_string,
                text,
            } => {
                if !content.contains(search_string) {
                    return Err(EditFileError::SearchStringNotFound {
                        search_string: search_string.clone(),
                        path: path.to_path_buf(),
                    });
                }
                Ok(content.replace(search_string, &format!("{}{}", text, search_string)))
            }
            EditAction::InsertAfter {
                search_string,
                text,
            } => {
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
        let action = args.get("action").and_then(|v| v.as_str()).ok_or_else(|| {
            EditFileError::MissingRequiredField {
                field: "action".to_string(),
                action: "unknown".to_string(),
            }
        })?;

        match action {
            "insert_at_start" => {
                let text = args
                    .get("replacement_text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| EditFileError::MissingRequiredField {
                        field: "replacement_text".to_string(),
                        action: action.to_string(),
                    })?;
                Ok(EditAction::InsertAtStart {
                    text: text.to_string(),
                })
            }
            "insert_at_end" => {
                let text = args
                    .get("replacement_text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| EditFileError::MissingRequiredField {
                        field: "replacement_text".to_string(),
                        action: action.to_string(),
                    })?;
                Ok(EditAction::InsertAtEnd {
                    text: text.to_string(),
                })
            }
            "delete" => {
                let search_string = args
                    .get("search_string")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| EditFileError::MissingRequiredField {
                        field: "search_string".to_string(),
                        action: action.to_string(),
                    })?;
                Ok(EditAction::Delete {
                    search_string: search_string.to_string(),
                })
            }
            "replace" => {
                let search_string = args
                    .get("search_string")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| EditFileError::MissingRequiredField {
                        field: "search_string".to_string(),
                        action: action.to_string(),
                    })?;
                let replacement_text = args
                    .get("replacement_text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| EditFileError::MissingRequiredField {
                        field: "replacement_text".to_string(),
                        action: action.to_string(),
                    })?;
                let expected_occurrences = args
                    .get("expected_occurrences")
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
                let search_string = args
                    .get("search_string")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| EditFileError::MissingRequiredField {
                        field: "search_string".to_string(),
                        action: action.to_string(),
                    })?;
                let text = args
                    .get("replacement_text")
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
                let search_string = args
                    .get("search_string")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| EditFileError::MissingRequiredField {
                        field: "search_string".to_string(),
                        action: action.to_string(),
                    })?;
                let text = args
                    .get("replacement_text")
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

/// EditFile actor
pub struct EditFile {
    tx: broadcast::Sender<ActorMessage>,
    #[allow(dead_code)] // TODO: Use for file operation settings, limits
    config: ParsedConfig,
    file_editor: FileEditor,
    file_reader: Arc<Mutex<super::file_reader::FileReader>>,
    scope: Scope,
}

impl EditFile {
    pub fn new(
        config: ParsedConfig,
        tx: broadcast::Sender<ActorMessage>,
        file_reader: Arc<Mutex<super::file_reader::FileReader>>,
        scope: Scope,
    ) -> Self {
        Self {
            config,
            tx,
            file_editor: FileEditor::new(),
            file_reader,
            scope,
        }
    }

    async fn handle_tool_call(&mut self, tool_call: ToolCall) {
        if tool_call.fn_name != TOOL_NAME {
            return;
        }

        // Parse the arguments
        let args = match serde_json::from_value::<serde_json::Value>(tool_call.fn_arguments) {
            Ok(args) => args,
            Err(e) => {
                let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Err(format!(
                        "Failed to parse arguments: {}",
                        e
                    ))),
                }));
                return;
            }
        };

        // Extract path
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => {
                let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Err(
                        "Missing required field: path".to_string()
                    )),
                }));
                return;
            }
        };

        // Parse action
        let action = match FileEditor::parse_action_from_args(&args) {
            Ok(action) => action,
            Err(e) => {
                let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Err(e.to_string())),
                }));
                return;
            }
        };

        let friendly_command_display = match &action {
            EditAction::InsertAtStart { text } => {
                format!("Insert at start of {}: {} chars", path, text.len())
            }
            EditAction::InsertAtEnd { text } => {
                format!("Insert at end of {}: {} chars", path, text.len())
            }
            EditAction::Delete { search_string } => {
                format!("Delete '{}' from {}", search_string, path)
            }
            EditAction::Replace {
                search_string,
                replacement_text,
                expected_occurrences,
            } => {
                format!(
                    "Replace {} occurrences of '{}' with '{}' in {}",
                    expected_occurrences, search_string, replacement_text, path
                )
            }
            EditAction::InsertBefore {
                search_string,
                text,
            } => {
                format!("Insert '{}' before '{}' in {}", text, search_string, path)
            }
            EditAction::InsertAfter {
                search_string,
                text,
            } => {
                format!("Insert '{}' after '{}' in {}", text, search_string, path)
            }
        };

        let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id.clone(),
            status: ToolCallStatus::Received {
                r#type: ToolCallType::EditFile,
                friendly_command_display,
            },
        }));

        // Execute the edit
        self.execute_edit(path, action, &tool_call.call_id).await;
    }

    async fn execute_edit(&mut self, path: &str, action: EditAction, tool_call_id: &str) {
        let mut file_reader = self.file_reader.lock().await;

        let result = self.file_editor.edit_file(path, action, &mut file_reader);

        let status = match &result {
            Ok(message) => {
                // Send system state update for successful file edit
                if let Ok(canonical_path) = std::fs::canonicalize(path) {
                    if let Ok(content) = file_reader.get_or_read_file_content(&canonical_path) {
                        if let Ok(metadata) = std::fs::metadata(&canonical_path) {
                            if let Ok(last_modified) = metadata.modified() {
                                let _ = self.broadcast(Message::FileEdited {
                                    path: canonical_path,
                                    content: content.to_string(),
                                    last_modified,
                                });
                            }
                        }
                    }
                }
                ToolCallStatus::Finished(Ok(message.clone()))
            }
            Err(e) => ToolCallStatus::Finished(Err(e.to_string())),
        };

        let _ = self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call_id.to_string(),
            status,
        }));
    }
}

#[async_trait::async_trait]
impl Actor for EditFile {
    const ACTOR_ID: &'static str = "edit_file";

    fn get_rx(&self) -> broadcast::Receiver<ActorMessage> {
        self.tx.subscribe()
    }

    fn get_tx(&self) -> broadcast::Sender<ActorMessage> {
        self.tx.clone()
    }

    fn get_scope(&self) -> &Scope {
        &self.scope
    }

    async fn on_start(&mut self) {
        info!("EditFile tool starting - broadcasting availability");

        let tool = Tool {
            name: TOOL_NAME.to_string(),
            description: Some(TOOL_DESCRIPTION.to_string()),
            schema: Some(serde_json::from_str(TOOL_INPUT_SCHEMA).unwrap()),
        };

        let _ = self.broadcast(Message::ToolsAvailable(vec![tool]));
    }

    async fn handle_message(&mut self, message: ActorMessage) {
        match message.message {
            Message::AssistantToolCall(tool_call) => self.handle_tool_call(tool_call).await,
            _ => (),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_edit_nonexistent_file_error_message() {
        let editor = FileEditor::new();
        let mut file_reader = super::super::file_reader::FileReader::default();
        
        // Try to edit a file that doesn't exist
        let result = editor.edit_file(
            "/tmp/this_file_definitely_does_not_exist_123456789.txt",
            EditAction::Replace {
                search_string: "old".to_string(),
                replacement_text: "new".to_string(),
                expected_occurrences: 1,
            },
            &mut file_reader,
        );
        
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        
        // Check that the error message contains our helpful guidance
        assert!(error_msg.contains("File does not exist"));
        assert!(error_msg.contains("Please create the file first"));
        assert!(error_msg.contains("verify the path is correct"));
    }
    
    #[test]
    fn test_edit_existing_file_not_read_error() {
        let editor = FileEditor::new();
        let mut file_reader = super::super::file_reader::FileReader::default();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        
        // Create a file but don't read it
        fs::write(&file_path, "test content").unwrap();
        
        // Try to edit without reading first
        let result = editor.edit_file(
            &file_path,
            EditAction::Replace {
                search_string: "test".to_string(),
                replacement_text: "new".to_string(),
                expected_occurrences: 1,
            },
            &mut file_reader,
        );
        
        assert!(result.is_err());
        match result.unwrap_err() {
            EditFileError::FileNotRead { path } => {
                assert!(path.to_string_lossy().contains("test.txt"));
            }
            _ => panic!("Expected FileNotRead error"),
        }
    }
    
    #[test]
    fn test_edit_file_with_invalid_path_chars() {
        let editor = FileEditor::new();
        let mut file_reader = super::super::file_reader::FileReader::default();
        
        // Try to edit a file with an invalid path
        let result = editor.edit_file(
            "\0invalid\0path",
            EditAction::InsertAtStart {
                text: "hello".to_string(),
            },
            &mut file_reader,
        );
        
        assert!(result.is_err());
        // This should produce an error when trying to write the file
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Failed to write file"));
    }
    
    #[test]
    fn test_insert_at_start_creates_new_file() {
        let editor = FileEditor::new();
        let mut file_reader = super::super::file_reader::FileReader::default();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("new_file.txt");
        
        // Ensure file doesn't exist
        assert!(!file_path.exists());
        
        // Use InsertAtStart on non-existent file
        let result = editor.edit_file(
            &file_path,
            EditAction::InsertAtStart {
                text: "Hello, world!\n".to_string(),
            },
            &mut file_reader,
        );
        
        assert!(result.is_ok());
        assert!(file_path.exists());
        
        // Verify the content
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Hello, world!\n");
        
        // Verify it was cached
        let canonical_path = fs::canonicalize(&file_path).unwrap();
        assert!(file_reader.get_cached_content(&canonical_path).is_some());
    }
    
    #[test]
    fn test_insert_at_start_creates_parent_directories() {
        let editor = FileEditor::new();
        let mut file_reader = super::super::file_reader::FileReader::default();
        let temp_dir = TempDir::new().unwrap();
        let nested_path = temp_dir.path().join("nested").join("dirs").join("file.txt");
        
        // Ensure parent directories don't exist
        assert!(!nested_path.parent().unwrap().exists());
        
        // Use InsertAtStart on non-existent file with non-existent parent dirs
        let result = editor.edit_file(
            &nested_path,
            EditAction::InsertAtStart {
                text: "Created with nested dirs!\n".to_string(),
            },
            &mut file_reader,
        );
        
        assert!(result.is_ok());
        assert!(nested_path.exists());
        assert!(nested_path.parent().unwrap().exists());
        
        // Verify the content
        let content = fs::read_to_string(&nested_path).unwrap();
        assert_eq!(content, "Created with nested dirs!\n");
    }
    
    #[test]
    fn test_other_actions_still_fail_on_nonexistent_file() {
        let editor = FileEditor::new();
        let mut file_reader = super::super::file_reader::FileReader::default();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("nonexistent.txt");
        
        // InsertAtEnd should still fail
        let result = editor.edit_file(
            &file_path,
            EditAction::InsertAtEnd {
                text: "This should fail".to_string(),
            },
            &mut file_reader,
        );
        assert!(result.is_err());
        
        // Replace should still fail
        let result = editor.edit_file(
            &file_path,
            EditAction::Replace {
                search_string: "old".to_string(),
                replacement_text: "new".to_string(),
                expected_occurrences: 1,
            },
            &mut file_reader,
        );
        assert!(result.is_err());
        
        // Delete should still fail
        let result = editor.edit_file(
            &file_path,
            EditAction::Delete {
                search_string: "delete_me".to_string(),
            },
            &mut file_reader,
        );
        assert!(result.is_err());
    }
}
