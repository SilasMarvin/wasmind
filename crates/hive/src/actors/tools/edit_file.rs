use crate::llm_client::{Tool, ToolCall};
use snafu::{ResultExt, Snafu};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::broadcast;
use tracing::info;

use crate::actors::{Actor, ActorContext, ActorMessage, Message, ToolCallStatus, ToolCallUpdate};
use crate::config::ParsedConfig;
use crate::scope::Scope;

use super::file_reader::FileCacheError;

pub const TOOL_NAME: &str = "edit_file";
pub const TOOL_DESCRIPTION: &str = "Applies a list of edits to a file atomically. This is the primary tool for modifying files. Each edit targets a specific line range. The tool processes edits from the bottom of the file to the top to ensure line number integrity during the operation.";
pub const TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "path": {
            "type": "string",
            "description": "The path to the file to edit."
        },
        "edits": {
            "type": "array",
            "description": "A list of edits to apply. Processed in reverse order of line number.",
            "items": {
                "type": "object",
                "properties": {
                    "start_line": {
                        "type": "integer",
                        "description": "The line number to start the edit on (inclusive). Line numbers start a 1."
                    },
                    "end_line": {
                        "type": "integer",
                        "description": "The line number to end the edit on (inclusive). For an insertion, set this to `start_line - 1`."
                    },
                    "new_content": {
                        "type": "string",
                        "description": "The new content to replace the specified lines with. Use an empty string to delete."
                    }
                },
                "required": ["start_line", "end_line", "new_content"]
            }
        }
    },
    "required": ["path", "edits"]
}"#;

// --- Error Handling with Snafu ---
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum EditFileError {
    #[snafu(display("File '{}' has not been read yet. Please use the read_file tool first.", path.display()))]
    FileNotRead { path: PathBuf },

    #[snafu(display("File '{}' has been modified since last read. Please use the read_file tool to refresh the file contents.", path.display()))]
    FileModified { path: PathBuf },

    #[snafu(display("Failed to create file: '{}'", path.display()))]
    CreateFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display(
        "Invalid line numbers for edit: start={}, end={}, total_lines={}",
        start,
        end,
        total
    ))]
    InvalidLineNumbers {
        start: usize,
        end: usize,
        total: usize,
    },

    #[snafu(display("Missing required field '{}'", field))]
    MissingRequiredField { field: String },

    #[snafu(display("Failed to read file '{}': {}", path.display(), source))]
    ReadFile {
        source: std::io::Error,
        path: PathBuf,
    },

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

    #[snafu(display("Failed to update file cache after making edits {}", source))]
    FileCache { source: FileCacheError },
}

pub type Result<T, E = EditFileError> = std::result::Result<T, E>;

/// Represents a single edit operation
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub start_line: usize, // 1-indexed
    pub end_line: usize,   // 1-indexed, inclusive
    pub new_content: String,
}

/// File editor that works with the FileReader cache
pub struct FileEditor;

impl FileEditor {
    pub fn new() -> Self {
        Self
    }

    /// Apply multiple edits to a file atomically
    pub fn apply_edits<P: AsRef<Path>>(
        &self,
        path: P,
        mut edits: Vec<Edit>,
        file_reader: &mut super::file_reader::FileReader,
    ) -> Result<String> {
        let path_ref = path.as_ref();

        // Check if the file exists
        if !path_ref.exists() {
            if edits.len() == 1 && edits[0].start_line == 1 && edits[0].end_line == 0 {
                if let Some(parent) = path_ref.parent() {
                    std::fs::create_dir_all(parent).context(CreateFileSnafu {
                        path: path_ref.to_owned(),
                    })?;
                }
                std::fs::File::create(&path).context(CreateFileSnafu {
                    path: path_ref.to_owned(),
                })?;
                // If this read fails something weird is happening. It will be caught later in this
                // function and the LLM will be asked to read it. This is fine but not ideal
                file_reader.read_and_cache_file(path_ref, None, None).ok();
            } else {
                return Err(EditFileError::CanonicalizePath {
                    source: std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "File does not exist",
                    ),
                    path: path_ref.to_path_buf(),
                });
            }
        }

        let canonical_path = fs::canonicalize(path_ref).context(CanonicalizePathSnafu {
            path: path_ref.to_path_buf(),
        })?;

        // MANDATORY: Check the file's modification time
        if let Some(entry) = file_reader.get_cached_entry(&canonical_path) {
            let metadata = fs::metadata(&canonical_path).context(ReadFileSnafu {
                path: canonical_path.clone(),
            })?;
            let current_mtime = metadata.modified().context(ReadFileSnafu {
                path: canonical_path.clone(),
            })?;

            if current_mtime != entry.last_modified_at_read {
                return Err(EditFileError::FileModified {
                    path: canonical_path,
                });
            }
        } else {
            return Err(EditFileError::FileNotRead {
                path: canonical_path,
            });
        }

        // Read the file into a Vec<String>
        let content = fs::read_to_string(&canonical_path).context(ReadFileSnafu {
            path: canonical_path.clone(),
        })?;
        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        let total_lines = lines.len();

        // Sort edits in descending order by start_line
        edits.sort_by(|a, b| b.start_line.cmp(&a.start_line));

        let mut applied_edits = vec![];

        // Apply each edit
        for edit in edits {
            // Validate line numbers
            if edit.start_line < 1 || edit.start_line > total_lines + 1 {
                return Err(EditFileError::InvalidLineNumbers {
                    start: edit.start_line,
                    end: edit.end_line,
                    total: total_lines,
                });
            }

            // Handle insertion (end_line = start_line - 1)
            if edit.end_line == edit.start_line - 1 {
                // Insert new lines at the position
                let new_lines: Vec<String> =
                    edit.new_content.lines().map(|s| s.to_string()).collect();
                let insert_pos = edit.start_line - 1;
                for (i, line) in new_lines.into_iter().enumerate() {
                    lines.insert(insert_pos + i, line);
                }
                applied_edits.push(edit);
            } else {
                // Replace or delete lines
                if edit.end_line < edit.start_line || edit.end_line > total_lines {
                    return Err(EditFileError::InvalidLineNumbers {
                        start: edit.start_line,
                        end: edit.end_line,
                        total: total_lines,
                    });
                }

                // Remove the old lines
                for _ in edit.start_line..=edit.end_line {
                    lines.remove(edit.start_line - 1);
                }

                // Insert new content if not deleting
                if !edit.new_content.is_empty() {
                    let new_lines: Vec<String> =
                        edit.new_content.lines().map(|s| s.to_string()).collect();
                    for (i, line) in new_lines.into_iter().enumerate() {
                        lines.insert(edit.start_line - 1 + i, line);
                    }
                }

                applied_edits.push(edit);
            }
        }

        // Write the modified content back to disk
        let new_content = lines.join("\n");
        fs::write(&canonical_path, &new_content).context(WriteFileSnafu {
            path: canonical_path.clone(),
        })?;

        // Update the cache with new content
        // This isn't the most efficient method but it works
        for edit in applied_edits {
            let lines_count = edit.new_content.lines().count();
            if lines_count > 0 {
                let (start_line, end_line) = (edit.start_line, edit.start_line + lines_count - 1);
                file_reader
                    .read_and_cache_file(&canonical_path, Some(start_line), Some(end_line))
                    .context(FileCacheSnafu)?;
            }
        }

        Ok(format!(
            "Successfully edited file: {}",
            canonical_path.display()
        ))
    }

    /// Parse edits from JSON arguments
    pub fn parse_edits_from_args(args: &serde_json::Value) -> Result<Vec<Edit>> {
        let edits_array = args
            .get("edits")
            .and_then(|v| v.as_array())
            .ok_or_else(|| EditFileError::MissingRequiredField {
                field: "edits".to_string(),
            })?;

        let mut edits = Vec::new();
        for edit_obj in edits_array {
            let start_line = edit_obj
                .get("start_line")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| EditFileError::MissingRequiredField {
                    field: "start_line".to_string(),
                })? as usize;

            let end_line = edit_obj
                .get("end_line")
                .and_then(|v| v.as_i64()) // Use i64 to handle -1
                .ok_or_else(|| EditFileError::MissingRequiredField {
                    field: "end_line".to_string(),
                })? as i64;

            // Convert -1 or negative values to start_line - 1 for insertions
            let end_line = if end_line < 0 {
                (start_line as i64 - 1) as usize
            } else {
                end_line as usize
            };

            let new_content = edit_obj
                .get("new_content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| EditFileError::MissingRequiredField {
                    field: "new_content".to_string(),
                })?
                .to_string();

            edits.push(Edit {
                start_line,
                end_line,
                new_content,
            });
        }

        Ok(edits)
    }
}

impl Default for FileEditor {
    fn default() -> Self {
        Self::new()
    }
}

/// EditFile actor
#[derive(hive_macros::ActorContext)]
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
        if tool_call.function.name != TOOL_NAME {
            return;
        }

        // Parse the arguments
        let args = match serde_json::from_str::<serde_json::Value>(&tool_call.function.arguments) {
            Ok(args) => args,
            Err(e) => {
                self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.id,
                    status: ToolCallStatus::Finished {
                        result: Err(format!("Failed to parse arguments: {}", e)),
                        tui_display: None,
                    },
                }));
                return;
            }
        };

        // Extract path
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => {
                self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.id,
                    status: ToolCallStatus::Finished {
                        result: Err("Missing required field: path".to_string()),
                        tui_display: None,
                    },
                }));
                return;
            }
        };

        // Parse edits
        let edits = match FileEditor::parse_edits_from_args(&args) {
            Ok(edits) => edits,
            Err(e) => {
                self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.id,
                    status: ToolCallStatus::Finished {
                        result: Err(e.to_string()),
                        tui_display: None,
                    },
                }));
                return;
            }
        };

        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.id.clone(),
            status: ToolCallStatus::Received,
        }));

        // Execute the edits
        self.execute_edits(path, edits, &tool_call.id).await;
    }

    async fn execute_edits(&mut self, path: &str, edits: Vec<Edit>, tool_call_id: &str) {
        let mut file_reader = self.file_reader.lock().unwrap();

        let result = self.file_editor.apply_edits(path, edits, &mut file_reader);

        let status = match &result {
            Ok(message) => {
                // Send system state update for successful file edit
                if let Ok(canonical_path) = std::fs::canonicalize(path) {
                    let content = file_reader
                        .get_or_read_file_content(&canonical_path, None, None)
                        .unwrap_or_else(|_| String::new());
                    if let Ok(metadata) = std::fs::metadata(&canonical_path) {
                        if let Ok(last_modified) = metadata.modified() {
                            self.broadcast(Message::FileEdited {
                                path: canonical_path,
                                content,
                                last_modified,
                            });
                        }
                    }
                }
                ToolCallStatus::Finished {
                    result: Ok(message.clone()),
                    tui_display: None,
                }
            }
            Err(e) => ToolCallStatus::Finished {
                result: Err(e.to_string()),
                tui_display: None,
            },
        };

        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call_id.to_string(),
            status,
        }));
    }
}

#[async_trait::async_trait]
impl Actor for EditFile {
    const ACTOR_ID: &'static str = "edit_file";

    async fn on_start(&mut self) {
        info!("EditFile tool starting - broadcasting availability");

        let tool = Tool {
            tool_type: "function".to_string(),
            function: crate::llm_client::ToolFunction {
                name: TOOL_NAME.to_string(),
                description: TOOL_DESCRIPTION.to_string(),
                parameters: serde_json::from_str(TOOL_INPUT_SCHEMA).unwrap(),
            },
        };

        self.broadcast(Message::ToolsAvailable(vec![tool]));
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
    fn test_edit_nonexistent_file_error() {
        let editor = FileEditor::new();
        let mut file_reader = super::super::file_reader::FileReader::default();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir
            .path()
            .join("this_file_definitely_does_not_exist_123456789.txt");

        // Try to edit a file that doesn't exist
        let edits = vec![Edit {
            start_line: 1,
            end_line: 1,
            new_content: "new line".to_string(),
        }];

        let result = editor.apply_edits(file_path.to_str().unwrap(), edits, &mut file_reader);

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("File does not exist"));

        // With start_line 1 and end_line 0 it should work as we create it
        let edits = vec![Edit {
            start_line: 1,
            end_line: 0,
            new_content: "new line".to_string(),
        }];

        let result = editor.apply_edits(file_path.to_str().unwrap(), edits, &mut file_reader);

        println!("{:?}", result);

        assert!(result.is_ok());
        // No need to remove file manually - tempfile will clean up
    }

    #[test]
    fn test_edit_existing_file_not_read_error() {
        let editor = FileEditor::new();
        let mut file_reader = super::super::file_reader::FileReader::default();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create a file but don't read it
        fs::write(&file_path, "line 1\nline 2\nline 3").unwrap();

        // Try to edit without reading first
        let edits = vec![Edit {
            start_line: 2,
            end_line: 2,
            new_content: "modified line 2".to_string(),
        }];

        let result = editor.apply_edits(&file_path, edits, &mut file_reader);

        assert!(result.is_err());
        match result.unwrap_err() {
            EditFileError::FileNotRead { path } => {
                assert!(path.to_string_lossy().contains("test.txt"));
            }
            _ => panic!("Expected FileNotRead error"),
        }
    }

    #[test]
    fn test_single_line_replacement() {
        let editor = FileEditor::new();
        let mut file_reader = super::super::file_reader::FileReader::default();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create and read a file
        fs::write(&file_path, "line 1\nline 2\nline 3").unwrap();
        file_reader
            .read_and_cache_file(&file_path, None, None)
            .unwrap();

        // Replace line 2
        let edits = vec![Edit {
            start_line: 2,
            end_line: 2,
            new_content: "new line 2".to_string(),
        }];

        let result = editor.apply_edits(&file_path, edits, &mut file_reader);
        assert!(result.is_ok());

        // Verify the content
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "line 1\nnew line 2\nline 3");
    }

    #[test]
    fn test_multi_line_replacement() {
        let editor = FileEditor::new();
        let mut file_reader = super::super::file_reader::FileReader::default();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create and read a file
        fs::write(&file_path, "line 1\nline 2\nline 3\nline 4\nline 5").unwrap();
        file_reader
            .read_and_cache_file(&file_path, None, None)
            .unwrap();

        // Replace lines 2-4 with new content
        let edits = vec![Edit {
            start_line: 2,
            end_line: 4,
            new_content: "new line 2\nnew line 3".to_string(),
        }];

        let result = editor.apply_edits(&file_path, edits, &mut file_reader);
        assert!(result.is_ok());

        // Verify the content
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "line 1\nnew line 2\nnew line 3\nline 5");
    }

    #[test]
    fn test_line_insertion() {
        let editor = FileEditor::new();
        let mut file_reader = super::super::file_reader::FileReader::default();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create and read a file
        fs::write(&file_path, "line 1\nline 2\nline 3").unwrap();
        file_reader
            .read_and_cache_file(&file_path, None, None)
            .unwrap();

        // Insert a new line between line 1 and 2
        let edits = vec![Edit {
            start_line: 2,
            end_line: 1, // end_line < start_line means insertion
            new_content: "inserted line".to_string(),
        }];

        let result = editor.apply_edits(&file_path, edits, &mut file_reader);
        assert!(result.is_ok());

        // Verify the content
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "line 1\ninserted line\nline 2\nline 3");
    }

    #[test]
    fn test_line_deletion() {
        let editor = FileEditor::new();
        let mut file_reader = super::super::file_reader::FileReader::default();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create and read a file
        fs::write(&file_path, "line 1\nline 2\nline 3\nline 4").unwrap();
        file_reader
            .read_and_cache_file(&file_path, None, None)
            .unwrap();

        // Delete lines 2-3
        let edits = vec![Edit {
            start_line: 2,
            end_line: 3,
            new_content: String::new(), // Empty content means delete
        }];

        let result = editor.apply_edits(&file_path, edits, &mut file_reader);
        assert!(result.is_ok());

        // Verify the content
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "line 1\nline 4");
    }

    #[test]
    fn test_multiple_edits_applied_in_reverse_order() {
        let editor = FileEditor::new();
        let mut file_reader = super::super::file_reader::FileReader::default();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create and read a file
        fs::write(&file_path, "line 1\nline 2\nline 3\nline 4\nline 5").unwrap();
        file_reader
            .read_and_cache_file(&file_path, None, None)
            .unwrap();

        // Multiple edits - should be sorted and applied in reverse order
        let edits = vec![
            Edit {
                start_line: 2,
                end_line: 2,
                new_content: "modified line 2".to_string(),
            },
            Edit {
                start_line: 4,
                end_line: 4,
                new_content: "modified line 4".to_string(),
            },
            Edit {
                start_line: 3,
                end_line: 3,
                new_content: String::new(), // Delete line 3
            },
        ];

        let result = editor.apply_edits(&file_path, edits, &mut file_reader);
        assert!(result.is_ok());

        // Verify the content
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "line 1\nmodified line 2\nmodified line 4\nline 5");
    }

    #[test]
    fn test_file_staleness_check() {
        let editor = FileEditor::new();
        let mut file_reader = super::super::file_reader::FileReader::default();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create and read a file
        fs::write(&file_path, "original content").unwrap();
        file_reader
            .read_and_cache_file(&file_path, None, None)
            .unwrap();

        // Modify the file on disk after caching
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&file_path, "modified externally").unwrap();

        // Try to edit - should fail due to staleness check
        let edits = vec![Edit {
            start_line: 1,
            end_line: 1,
            new_content: "new content".to_string(),
        }];

        let result = editor.apply_edits(&file_path, edits, &mut file_reader);
        assert!(result.is_err());
        match result.unwrap_err() {
            EditFileError::FileModified { .. } => {}
            _ => panic!("Expected FileModified error"),
        }
    }
}
