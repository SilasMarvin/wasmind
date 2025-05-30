use genai::chat::{Tool, ToolCall};
use snafu::{ResultExt, Snafu};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::{broadcast, Mutex};
use tracing::{debug, info};

use crate::actors::{Actor, Message, ToolCallStatus, ToolCallType, ToolCallUpdate};
use crate::config::ParsedConfig;

pub const TOOL_NAME: &str = "read_file";
pub const TOOL_DESCRIPTION: &str = "Read file contents";
pub const MAX_FILE_SIZE_BYTES: u64 = 1024 * 1024; // 1MB limit
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

    #[snafu(display("File '{}' is too large ({} bytes). Maximum file size is {} bytes.", path.display(), actual_size, max_size))]
    FileTooLarge { path: PathBuf, actual_size: u64, max_size: u64 },
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
        
        // Check file size before reading
        let file_size = metadata.len();
        if file_size > MAX_FILE_SIZE_BYTES {
            return Err(FileCacheError::FileTooLarge {
                path: path_ref.to_path_buf(),
                actual_size: file_size,
                max_size: MAX_FILE_SIZE_BYTES,
            });
        }
        
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

/// FileReader actor
pub struct FileReaderActor {
    tx: broadcast::Sender<Message>,
    config: ParsedConfig,
    file_reader: Arc<Mutex<FileReader>>,
}

impl FileReaderActor {
    pub fn with_file_reader(
        config: ParsedConfig,
        tx: broadcast::Sender<Message>,
        file_reader: Arc<Mutex<FileReader>>,
    ) -> Self {
        Self {
            config,
            tx,
            file_reader,
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
                let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Err(format!("Failed to parse arguments: {}", e))),
                }));
                return;
            }
        };

        // Extract path
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => {
                let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
                    call_id: tool_call.call_id,
                    status: ToolCallStatus::Finished(Err("Missing required field: path".to_string())),
                }));
                return;
            }
        };

        let friendly_command_display = format!("Read file: {}", path);

        let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call.call_id.clone(),
            status: ToolCallStatus::Received {
                r#type: ToolCallType::ReadFile,
                friendly_command_display,
            },
        }));

        // Execute the read
        self.execute_read(path, &tool_call.call_id).await;
    }

    async fn execute_read(&mut self, path: &str, tool_call_id: &str) {
        let mut file_reader = self.file_reader.lock().await;
        
        let result = file_reader.get_or_read_file_content(path);
        
        let status = match &result {
            Ok(content) => {
                // Send system state update for successful file read
                if let Ok(canonical_path) = std::fs::canonicalize(path) {
                    if let Ok(metadata) = std::fs::metadata(&canonical_path) {
                        if let Ok(last_modified) = metadata.modified() {
                            let _ = self.tx.send(Message::FileRead {
                                path: canonical_path,
                                content: content.to_string(),
                                last_modified,
                            });
                        }
                    }
                }
                ToolCallStatus::Finished(Ok(content.to_string()))
            }
            Err(e) => ToolCallStatus::Finished(Err(e.to_string())),
        };

        let _ = self.tx.send(Message::ToolCallUpdate(ToolCallUpdate {
            call_id: tool_call_id.to_string(),
            status,
        }));
    }
}

#[async_trait::async_trait]
impl Actor for FileReaderActor {
    fn new(config: ParsedConfig, tx: broadcast::Sender<Message>) -> Self {
        Self {
            config,
            tx: tx.clone(),
            file_reader: Arc::new(Mutex::new(FileReader::new())),
        }
    }

    fn get_rx(&self) -> broadcast::Receiver<Message> {
        self.tx.subscribe()
    }

    async fn on_start(&mut self) {
        info!("FileReader tool starting - broadcasting availability");
        
        let tool = Tool {
            name: TOOL_NAME.to_string(),
            description: Some(TOOL_DESCRIPTION.to_string()),
            schema: Some(serde_json::from_str(TOOL_INPUT_SCHEMA).unwrap()),
        };
        
        let _ = self.tx.send(Message::ToolsAvailable(vec![tool]));
    }

    async fn handle_message(&mut self, message: Message) {
        match message {
            Message::AssistantToolCall(tool_call) => self.handle_tool_call(tool_call).await,
            _ => (),
        }
    }
}