use crate::llm_client::{Tool, ToolCall};
use snafu::{ResultExt, Snafu};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::SystemTime;
use tokio::sync::broadcast;

use crate::actors::ActorMessage;
use crate::actors::{Actor, Message, ToolCallStatus, ToolCallType, ToolCallUpdate};
use crate::config::ParsedConfig;
use crate::scope::Scope;

pub const TOOL_NAME: &str = "read_file";
pub const TOOL_DESCRIPTION: &str = "Reads content from a file. For small files (<64KB), it reads the entire file. For large files, it returns an error with metadata, requiring you to specify a line range. All returned file content is prefixed with line numbers in the format LINE_NUMBER|CONTENT. You can read a specific chunk by providing start_line and end_line.";
pub const MAX_FILE_SIZE_BYTES: u64 = 10 * 1024 * 1024; // 10MB limit
pub const SMALL_FILE_SIZE_BYTES: u64 = 64 * 1024; // 64KB limit for automatic full read
pub const TOOL_INPUT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "path": {
            "type": "string",
            "description": "Path to the file."
        },
        "start_line": {
            "type": "integer",
            "description": "Optional starting line to read (1-indexed)."
        },
        "end_line": {
            "type": "integer",
            "description": "Optional ending line to read (inclusive)."
        }
    },
    "required": ["path"]
}"#;

// --- Error Handling with Snafu ---
#[derive(Debug, Snafu)]
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

    #[snafu(display("File '{}' is too large ({} bytes). Maximum file size is {} bytes. Files this large cannot be read even with line ranges.", path.display(), actual_size, max_size))]
    FileTooLarge {
        path: PathBuf,
        actual_size: u64,
        max_size: u64,
    },

    #[snafu(display("File too large. You must specify a line range (start_line and end_line). File metadata: {}", serde_json::to_string(&serde_json::json!({"path": path.display().to_string(), "size_bytes": actual_size, "total_lines": total_lines})).unwrap_or_else(|_| "{}".to_string())))]
    FileTooLargeNeedLineRange {
        path: PathBuf,
        actual_size: u64,
        total_lines: usize,
    },
}

pub type Result<T, E = FileCacheError> = std::result::Result<T, E>;

/// A single, contiguous chunk of a file's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSlice {
    pub start_line: usize, // 1-indexed
    pub end_line: usize,   // 1-indexed, inclusive
    pub content: String,
}

/// How we represent the file's content in our cache.
#[derive(Debug, Clone)]
pub enum FileContent {
    Full(String),
    Partial {
        // A vector of known slices. MUST be kept sorted by `start_line`
        // and slices MUST NOT overlap after merging.
        slices: Vec<FileSlice>,
        // The total lines in the file used when rendering the prompt
        total_lines: usize,
    },
}

impl FileContent {
    /// Get the full content as a string with line numbers
    pub fn get_numbered_content(&self) -> String {
        match self {
            FileContent::Full(content) => content
                .lines()
                .enumerate()
                .map(|(i, line)| format!("{}|{}", i + 1, line))
                .collect::<Vec<_>>()
                .join("\n"),
            FileContent::Partial {
                slices,
                total_lines,
            } => {
                let mut result = Vec::new();
                let mut last_end = 0;

                for slice in slices {
                    // Add omitted lines indicator if there's a gap
                    if last_end > 0 && slice.start_line > last_end + 1 {
                        let omitted = slice.start_line - last_end - 1;
                        result.push(format!("[... {} lines omitted ...]", omitted));
                    } else if last_end == 0 && slice.start_line > 1 {
                        let omitted = slice.start_line - 1;
                        result.push(format!("[... {} lines omitted ...]", omitted));
                    }

                    // Add the slice content (already numbered)
                    result.push(slice.content.clone());
                    last_end = slice.end_line;
                }

                // Add final omitted lines if needed
                if last_end < *total_lines {
                    let omitted = total_lines - last_end;
                    result.push(format!("[... {} lines omitted ...]", omitted));
                }

                result.join("\n")
            }
        }
    }

    /// Merge a new slice into partial content
    pub fn merge_slice(&mut self, new_slice: FileSlice) {
        if let FileContent::Partial { slices, .. } = self {
            // Insert the new slice
            slices.push(new_slice);

            // Sort by start_line
            slices.sort_by_key(|s| s.start_line);

            // Merge overlapping slices
            let mut merged = Vec::new();
            for slice in slices.drain(..) {
                if merged.is_empty() {
                    merged.push(slice);
                } else {
                    let last = merged.last_mut().unwrap();
                    if slice.start_line <= last.end_line + 1 {
                        // Overlapping or adjacent slices - merge them
                        if slice.end_line > last.end_line {
                            // Extend the content
                            let last_lines: Vec<&str> = last.content.lines().collect();
                            let new_lines: Vec<&str> = slice.content.lines().collect();

                            // Calculate how many lines to take from the new slice
                            let overlap = if last.end_line >= slice.start_line {
                                last.end_line - slice.start_line + 1
                            } else {
                                0
                            };

                            // Append non-overlapping lines
                            for line in new_lines.iter().skip(overlap) {
                                last.content.push('\n');
                                last.content.push_str(line);
                            }

                            last.end_line = slice.end_line;
                        }
                    } else {
                        merged.push(slice);
                    }
                }
            }
            *slices = merged;
        }
    }
}

/// Information stored for each cached file
#[derive(Debug, Clone)]
pub struct FileCacheEntry {
    pub content: FileContent,
    pub read_at: SystemTime,
    pub last_modified_at_read: SystemTime,
    pub size_bytes: u64,
}

/// Manages file reading and caching.
#[derive(Debug, Default)]
pub struct FileReader {
    cache: HashMap<PathBuf, FileCacheEntry>,
}

impl FileReader {
    pub fn read_and_cache_file<P: AsRef<Path>>(
        &mut self,
        path: P,
        start_line: Option<i32>,
        end_line: Option<i32>,
    ) -> Result<()> {
        let path_ref = path.as_ref();

        let metadata = fs::metadata(path_ref).context(ReadMetadataSnafu {
            path: path_ref.to_path_buf(),
        })?;

        let file_size = metadata.len();

        let last_modified_at_read = metadata.modified().context(GetModifiedTimeSnafu {
            path: path_ref.to_path_buf(),
        })?;

        // Check if file is absolutely too large to even attempt reading
        if file_size > MAX_FILE_SIZE_BYTES {
            return Err(FileCacheError::FileTooLarge {
                path: path_ref.to_path_buf(),
                actual_size: file_size,
                max_size: MAX_FILE_SIZE_BYTES,
            });
        }

        let contents = fs::read_to_string(path_ref).context(ReadFileSnafu {
            path: path_ref.to_path_buf(),
        })?;

        let lines: Vec<&str> = contents.lines().collect();
        let total_lines = lines.len();

        // Determine if we're reading the full file or a slice
        let content = match (start_line, end_line) {
            (None, None) => {
                // Check if file is small enough for automatic full read
                if file_size > SMALL_FILE_SIZE_BYTES {
                    return Err(FileCacheError::FileTooLargeNeedLineRange {
                        path: path_ref.to_path_buf(),
                        actual_size: file_size,
                        total_lines,
                    });
                }
                FileContent::Full(contents)
            }
            (Some(start), Some(end)) => {
                // Validate line numbers
                if start < 1
                    || start as usize > total_lines
                    || end < start
                    || end as usize > total_lines
                {
                    return Err(FileCacheError::ReadFile {
                        source: io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!(
                                "Invalid line range: {}-{} (file has {} lines)",
                                start, end, total_lines
                            ),
                        ),
                        path: path_ref.to_path_buf(),
                    });
                }

                let actual_end = end as usize;

                // Create numbered content for the slice
                let slice_lines: Vec<String> = lines[(start as usize - 1)..actual_end]
                    .iter()
                    .enumerate()
                    .map(|(i, line)| format!("{}|{}", start as usize + i, line))
                    .collect();

                let slice = FileSlice {
                    start_line: start as usize,
                    end_line: actual_end,
                    content: slice_lines.join("\n"),
                };

                FileContent::Partial {
                    slices: vec![slice],
                    total_lines,
                }
            }
            _ => {
                return Err(FileCacheError::ReadFile {
                    source: io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Both start_line and end_line must be provided for partial reads",
                    ),
                    path: path_ref.to_path_buf(),
                });
            }
        };

        let read_at = SystemTime::now();

        let entry = FileCacheEntry {
            content,
            read_at,
            last_modified_at_read,
            size_bytes: file_size,
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

    pub fn get_cached_content<P: AsRef<Path>>(&self, path: P) -> Option<String> {
        fs::canonicalize(path.as_ref())
            .ok()
            .and_then(|p| self.cache.get(&p))
            .map(|entry| entry.content.get_numbered_content())
    }

    pub fn get_or_read_file_content<P: AsRef<Path> + Clone>(
        &mut self,
        path: P,
        start_line: Option<i32>,
        end_line: Option<i32>,
    ) -> Result<String> {
        let path_ref = path.as_ref();
        let canonical_path = fs::canonicalize(path_ref).context(CanonicalizePathSnafu {
            path: path_ref.to_path_buf(),
        })?;

        // Check if we need to read or can use cache
        let needs_read = match (start_line, end_line) {
            (None, None) => {
                // Full file read - check if modified
                self.has_been_modified(path_ref)?
            }
            (Some(_), Some(_)) => {
                // Partial read - check if we have this slice or file was modified
                if self.has_been_modified(path_ref)? {
                    true
                } else if let Some(entry) = self.cache.get(&canonical_path) {
                    // Check if we already have this slice
                    match &entry.content {
                        FileContent::Full(_) => false, // We have the full file
                        FileContent::Partial { slices, .. } => {
                            // Check if the requested range is already covered
                            let start = start_line.unwrap() as usize;
                            let end = end_line.unwrap() as usize;

                            !slices
                                .iter()
                                .any(|slice| slice.start_line <= start && slice.end_line >= end)
                        }
                    }
                } else {
                    true
                }
            }
            _ => {
                return Err(FileCacheError::ReadFile {
                    source: io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Both start_line and end_line must be provided for partial reads",
                    ),
                    path: path_ref.to_path_buf(),
                });
            }
        };

        if needs_read {
            // Handle merging for partial reads
            if let (Some(start), Some(end)) = (start_line, end_line) {
                if let Some(existing_entry) = self.cache.get(&canonical_path).cloned() {
                    if !self.has_been_modified(path_ref)? {
                        // File hasn't changed, we can merge slices
                        let metadata = fs::metadata(path_ref).context(ReadMetadataSnafu {
                            path: path_ref.to_path_buf(),
                        })?;

                        let contents = fs::read_to_string(path_ref).context(ReadFileSnafu {
                            path: path_ref.to_path_buf(),
                        })?;

                        let lines: Vec<&str> = contents.lines().collect();
                        let total_lines = lines.len();

                        let actual_end = end as usize;

                        // Create the new slice
                        let slice_lines: Vec<String> = lines[(start as usize - 1)..actual_end]
                            .iter()
                            .enumerate()
                            .map(|(i, line)| format!("{}|{}", start as usize + i, line))
                            .collect();

                        let new_slice = FileSlice {
                            start_line: start as usize,
                            end_line: actual_end,
                            content: slice_lines.join("\n"),
                        };

                        // Merge into existing entry
                        let mut updated_entry = existing_entry;
                        match &mut updated_entry.content {
                            FileContent::Full(_) => {
                                // Already have full content, no need to merge
                            }
                            FileContent::Partial { .. } => {
                                updated_entry.content.merge_slice(new_slice);
                            }
                        }

                        self.cache.insert(canonical_path.clone(), updated_entry);
                    } else {
                        // File changed, do a fresh read
                        self.read_and_cache_file(path.clone(), start_line, end_line)?;
                    }
                } else {
                    // No existing entry, do a fresh read
                    self.read_and_cache_file(path.clone(), start_line, end_line)?;
                }
            } else {
                // Full file read
                self.read_and_cache_file(path.clone(), start_line, end_line)?;
            }
        }

        self.cache
            .get(&canonical_path)
            .map(|entry| entry.content.get_numbered_content())
            .ok_or_else(|| FileCacheError::CacheMissInternal {
                path: canonical_path.clone(),
            })
    }

    pub fn get_cached_entry<P: AsRef<Path>>(&self, path: P) -> Option<&FileCacheEntry> {
        fs::canonicalize(path.as_ref())
            .ok()
            .and_then(|p| self.cache.get(&p))
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

/// FileReader actor
pub struct FileReaderActor {
    tx: broadcast::Sender<ActorMessage>,
    #[allow(dead_code)] // TODO: Use for file size limits, timeout settings
    config: ParsedConfig,
    file_reader: Arc<Mutex<FileReader>>,
    scope: Scope,
}

impl FileReaderActor {
    pub fn new(
        config: ParsedConfig,
        tx: broadcast::Sender<ActorMessage>,
        file_reader: Arc<Mutex<FileReader>>,
        scope: Scope,
    ) -> Self {
        Self {
            config,
            tx,
            file_reader,
            scope,
        }
    }

    #[tracing::instrument(name = "file_reader_tool_call", skip(self, tool_call), fields(call_id = %tool_call.id, function = %tool_call.function.name))]
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
                self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.id,
                    status: ToolCallStatus::Finished(Err(
                        "Missing required field: path".to_string()
                    )),
                }));
                return;
            }
        };

        // Extract optional line range
        let start_line = args
            .get("start_line")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32);
        let end_line = args
            .get("end_line")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32);

        let friendly_command_display = match (start_line, end_line) {
            (Some(start), Some(end)) => format!("Read file: {} (lines {}-{})", path, start, end),
            _ => format!("Read file: {}", path),
        };

        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.id.clone(),
            status: ToolCallStatus::Received {
                r#type: ToolCallType::ReadFile,
                friendly_command_display,
            },
        }));

        // Execute the read
        self.execute_read(path, start_line, end_line, &tool_call.id)
            .await;
    }

    async fn execute_read(
        &mut self,
        path: &str,
        start_line: Option<i32>,
        end_line: Option<i32>,
        tool_call_id: &str,
    ) {
        let mut file_reader = self.file_reader.lock().unwrap();

        let result = file_reader.get_or_read_file_content(path, start_line, end_line);

        let status = match &result {
            Ok(content) => {
                // Send system state update for successful file read
                if let Ok(canonical_path) = std::fs::canonicalize(path) {
                    if let Ok(metadata) = std::fs::metadata(&canonical_path) {
                        if let Ok(last_modified) = metadata.modified() {
                            self.broadcast(Message::FileRead {
                                path: canonical_path,
                                content: content.to_string(),
                                last_modified,
                            });
                        }
                    }
                }
                let message = match (start_line, end_line) {
                    (Some(start), Some(end)) => {
                        format!("Read file: {} (lines {}-{})", path, start, end)
                    }
                    _ => format!("Read file: {}", path),
                };
                ToolCallStatus::Finished(Ok(message))
            }
            Err(e) => ToolCallStatus::Finished(Err(e.to_string())),
        };

        self.broadcast(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call_id.to_string(),
            status,
        }));
    }
}

#[async_trait::async_trait]
impl Actor for FileReaderActor {
    const ACTOR_ID: &'static str = "file_reader";

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
    fn test_file_slice_creation() {
        let slice = FileSlice {
            start_line: 5,
            end_line: 7,
            content: "5|line 5\n6|line 6\n7|line 7".to_string(),
        };

        assert_eq!(slice.start_line, 5);
        assert_eq!(slice.end_line, 7);
        assert!(slice.content.contains("5|line 5"));
    }

    #[test]
    fn test_file_content_full_get_numbered_content() {
        let content = FileContent::Full("line 1\nline 2\nline 3".to_string());
        let numbered = content.get_numbered_content();

        assert_eq!(numbered, "1|line 1\n2|line 2\n3|line 3");
    }

    #[test]
    fn test_file_content_partial_get_numbered_content() {
        let slice1 = FileSlice {
            start_line: 1,
            end_line: 2,
            content: "1|line 1\n2|line 2".to_string(),
        };
        let slice2 = FileSlice {
            start_line: 5,
            end_line: 6,
            content: "5|line 5\n6|line 6".to_string(),
        };

        let content = FileContent::Partial {
            slices: vec![slice1, slice2],
            total_lines: 10,
        };

        let numbered = content.get_numbered_content();
        assert!(numbered.contains("1|line 1"));
        assert!(numbered.contains("2|line 2"));
        assert!(numbered.contains("[... 2 lines omitted ...]"));
        assert!(numbered.contains("5|line 5"));
        assert!(numbered.contains("6|line 6"));
        assert!(numbered.contains("[... 4 lines omitted ...]"));
    }

    #[test]
    fn test_file_content_merge_slice_non_overlapping() {
        let slice1 = FileSlice {
            start_line: 1,
            end_line: 2,
            content: "1|line 1\n2|line 2".to_string(),
        };

        let mut content = FileContent::Partial {
            slices: vec![slice1],
            total_lines: 10,
        };

        let new_slice = FileSlice {
            start_line: 5,
            end_line: 6,
            content: "5|line 5\n6|line 6".to_string(),
        };

        content.merge_slice(new_slice);

        if let FileContent::Partial { slices, .. } = content {
            assert_eq!(slices.len(), 2);
            assert_eq!(slices[0].start_line, 1);
            assert_eq!(slices[1].start_line, 5);
        } else {
            panic!("Expected partial content");
        }
    }

    #[test]
    fn test_file_content_merge_slice_overlapping() {
        let slice1 = FileSlice {
            start_line: 1,
            end_line: 3,
            content: "1|line 1\n2|line 2\n3|line 3".to_string(),
        };

        let mut content = FileContent::Partial {
            slices: vec![slice1],
            total_lines: 10,
        };

        // Overlapping slice
        let new_slice = FileSlice {
            start_line: 3,
            end_line: 5,
            content: "3|line 3\n4|line 4\n5|line 5".to_string(),
        };

        content.merge_slice(new_slice);

        if let FileContent::Partial { slices, .. } = content {
            assert_eq!(slices.len(), 1);
            assert_eq!(slices[0].start_line, 1);
            assert_eq!(slices[0].end_line, 5);
            assert!(slices[0].content.contains("1|line 1"));
            assert!(slices[0].content.contains("5|line 5"));
        } else {
            panic!("Expected partial content");
        }
    }

    #[test]
    fn test_file_content_merge_slice_adjacent() {
        let slice1 = FileSlice {
            start_line: 1,
            end_line: 3,
            content: "1|line 1\n2|line 2\n3|line 3".to_string(),
        };

        let mut content = FileContent::Partial {
            slices: vec![slice1],
            total_lines: 10,
        };

        // Adjacent slice
        let new_slice = FileSlice {
            start_line: 4,
            end_line: 6,
            content: "4|line 4\n5|line 5\n6|line 6".to_string(),
        };

        content.merge_slice(new_slice);

        if let FileContent::Partial { slices, .. } = content {
            assert_eq!(slices.len(), 1);
            assert_eq!(slices[0].start_line, 1);
            assert_eq!(slices[0].end_line, 6);
        } else {
            panic!("Expected partial content");
        }
    }

    #[test]
    fn test_file_reader_partial_read() {
        let mut file_reader = FileReader::default();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create a file with several lines
        let content = "line 1\nline 2\nline 3\nline 4\nline 5";
        fs::write(&file_path, content).unwrap();

        // Read a slice
        let result = file_reader.read_and_cache_file(&file_path, Some(2), Some(4));
        assert!(result.is_ok());

        // Check that it was cached as partial content
        let cached = file_reader.get_cached_content(&file_path);
        assert!(cached.is_some());
        let cached_content = cached.unwrap();
        assert!(cached_content.contains("2|line 2"));
        assert!(cached_content.contains("3|line 3"));
        assert!(cached_content.contains("4|line 4"));
        assert!(!cached_content.contains("1|line 1"));
        assert!(!cached_content.contains("5|line 5"));
    }

    #[test]
    fn test_file_reader_full_read_small_file() {
        let mut file_reader = FileReader::default();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("small.txt");

        // Create a small file
        let content = "small file content";
        fs::write(&file_path, content).unwrap();

        // Read without specifying range
        let result = file_reader.read_and_cache_file(&file_path, None, None);
        assert!(result.is_ok());

        // Check that it was cached as full content
        let cached = file_reader.get_cached_content(&file_path);
        assert!(cached.is_some());
        let cached_content = cached.unwrap();
        assert_eq!(cached_content, "1|small file content");
    }

    #[test]
    fn test_file_reader_read_to_end() {
        let mut file_reader = FileReader::default();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create a file with several lines
        let content = "line 1\nline 2\nline 3\nline 4\nline 5";
        fs::write(&file_path, content).unwrap();

        // Read from line 3 to end (line 5)
        let result = file_reader.read_and_cache_file(&file_path, Some(3), Some(5));
        assert!(result.is_ok());

        // Check content
        let cached = file_reader.get_cached_content(&file_path);
        assert!(cached.is_some());
        let cached_content = cached.unwrap();
        assert!(cached_content.contains("3|line 3"));
        assert!(cached_content.contains("4|line 4"));
        assert!(cached_content.contains("5|line 5"));
        assert!(!cached_content.contains("1|line 1"));
        assert!(!cached_content.contains("2|line 2"));
    }

    #[test]
    fn test_file_reader_slice_merging() {
        let mut file_reader = FileReader::default();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create a file with several lines
        let content =
            "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10";
        fs::write(&file_path, content).unwrap();

        // Read first slice
        file_reader
            .read_and_cache_file(&file_path, Some(1), Some(3))
            .unwrap();

        // Read overlapping slice
        let result = file_reader.get_or_read_file_content(&file_path, Some(3), Some(6));
        assert!(result.is_ok());

        // Check that slices were merged
        let cached = file_reader.get_cached_content(&file_path).unwrap();
        assert!(cached.contains("1|line 1"));
        assert!(cached.contains("6|line 6"));
        assert!(cached.contains("[... 4 lines omitted ...]"));
    }

    #[test]
    fn test_file_too_large_need_line_range_json_metadata() {
        let mut file_reader = FileReader::default();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("large.txt");

        // Create a file larger than SMALL_FILE_SIZE_BYTES (64KB)
        let line = "This is a line of text that is long enough to make the file large.\n";
        let content = line.repeat(2000); // Should be > 64KB
        fs::write(&file_path, &content).unwrap();

        // Try to read the full file (should fail and include JSON metadata)
        let result = file_reader.read_and_cache_file(&file_path, None, None);
        assert!(result.is_err());

        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("File too large"));
        assert!(error_msg.contains("You must specify a line range"));
        assert!(error_msg.contains("File metadata:"));
        assert!(error_msg.contains("\"path\":"));
        assert!(error_msg.contains("\"size_bytes\":"));
        assert!(error_msg.contains("\"total_lines\":"));

        // Verify it's valid JSON by parsing the metadata part
        let metadata_start = error_msg.find("File metadata: ").unwrap() + "File metadata: ".len();
        let json_str = &error_msg[metadata_start..];
        let json: serde_json::Value = serde_json::from_str(json_str).unwrap();

        assert!(json["path"].as_str().unwrap().contains("large.txt"));
        assert!(json["size_bytes"].as_u64().unwrap() > SMALL_FILE_SIZE_BYTES);
        assert!(json["total_lines"].as_u64().unwrap() > 1000);
    }
}
