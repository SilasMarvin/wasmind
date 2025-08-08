use std::{
    collections::HashMap,
    fs, io,
    path::{Component as PathComponent, Path, PathBuf},
    time::SystemTime,
};

use bindings::exports::hive::actor::actor::MessageEnvelope;
use hive_actor_utils::common_messages::{
    assistant::{Section, SystemPromptContent, SystemPromptContribution},
    tools::{
        ExecuteTool, ToolCallResult, ToolCallStatus, ToolCallStatusUpdate, ToolsAvailable,
        UIDisplayInfo,
    },
};
use serde::{Deserialize, Serialize};

#[allow(warnings)]
mod bindings;

hive_actor_utils::actors::macros::generate_actor_trait!();

/// WASM-safe alternative to wasm_safe_normalize_path that works without OS-level path resolution
fn wasm_safe_normalize_path(path: &Path) -> Result<PathBuf, io::Error> {
    // Check if path is absolute - relative paths are not supported in WASM
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "Relative paths are not supported. Please use an absolute path starting with '/'. Got: '{}'",
                path.display()
            ),
        ));
    }

    // First check if the file exists to ensure we're working with a valid path
    // This also validates the path without needing canonicalize
    fs::metadata(path)?;

    // Clean and normalize the path components
    Ok(clean_path_components(path))
}

/// Clean and normalize path components without OS calls
/// Handles . and .. components, converts relative paths to a consistent form
fn clean_path_components(path: &Path) -> PathBuf {
    let mut components = vec![];

    for component in path.components() {
        match component {
            PathComponent::ParentDir => {
                // Only pop if we have normal components to pop
                if let Some(PathComponent::Normal(_)) = components.last() {
                    components.pop();
                }
            }
            PathComponent::Normal(name) => {
                components.push(PathComponent::Normal(name));
            }
            PathComponent::RootDir => {
                // Clear everything and start with root
                components.clear();
                components.push(PathComponent::RootDir);
            }
            PathComponent::CurDir => {
                // Skip current directory components
            }
            PathComponent::Prefix(prefix) => {
                // Preserve Windows prefixes (unlikely in WASM but handle gracefully)
                components.push(PathComponent::Prefix(prefix));
            }
        }
    }

    // Rebuild the path from cleaned components
    let mut result = PathBuf::new();
    for component in components {
        match component {
            PathComponent::RootDir => result.push("/"),
            PathComponent::Normal(name) => result.push(name),
            PathComponent::Prefix(prefix) => result.push(prefix.as_os_str()),
            _ => {} // Skip other component types
        }
    }

    // If we ended up with an empty path, use the original
    if result.as_os_str().is_empty() {
        path.to_path_buf()
    } else {
        result
    }
}

const MAX_FILE_SIZE_BYTES: u64 = 10 * 1024 * 1024; // 10MB limit
const SMALL_FILE_SIZE_BYTES: u64 = 64 * 1024; // 64KB limit for automatic full read

const READ_FILE_NAME: &str = "read_file";
const READ_FILE_DESCRIPTION: &str = "Reads content from a file. For small files (<64KB), it reads the entire file. For large files, it returns an error with metadata, requiring you to specify a line range. All returned file content is prefixed with line numbers in the format LINE_NUMBER|CONTENT. You can read a specific chunk by providing start_line and end_line. IMPORTANT: Always use absolute paths (starting with /) - relative paths will fail.";
const READ_FILE_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "path": {
            "type": "string",
            "description": "ABSOLUTE path to the file (must start with /). Relative paths are not supported and will cause errors."
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

const EDIT_FILE_NAME: &str = "edit_file";
const EDIT_FILE_DESCRIPTION: &str = "Applies a list of edits to a file atomically. This is the primary tool for modifying files AND creating new files. Each edit targets a specific line range. The tool processes edits from the bottom of the file to the top to ensure line number integrity during the operation. To create a new file, use a single edit with start_line=1 and end_line=0. IMPORTANT: Always use absolute paths (starting with /) - relative paths will fail.";
const EDIT_FILE_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "path": {
            "type": "string",
            "description": "ABSOLUTE path to the file to edit or create (must start with /). If the file doesn't exist, it will be created (along with any necessary parent directories). Relative paths are not supported and will cause errors."
        },
        "edits": {
            "type": "array",
            "description": "A list of edits to apply. Processed in reverse order of line number.",
            "items": {
                "type": "object",
                "properties": {
                    "start_line": {
                        "type": "integer",
                        "description": "The line number to start the edit on (inclusive). Line numbers start at 1."
                    },
                    "end_line": {
                        "type": "integer",
                        "description": "The line number to end the edit on (inclusive). For an insertion, set this to `start_line - 1`. For creating a new file, use start_line=1 and end_line=0."
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

const FILE_TOOLS_USAGE_GUIDE: &str = r#"## File Interaction Tools

The file interaction actor provides two essential tools for reading and editing files:

**CRITICAL: Always use ABSOLUTE paths (starting with /) - relative paths will fail!**

### read_file
- Reads file content with automatic line numbering (format: LINE_NUMBER|CONTENT)
- LINE_NUMBERs are 1-based -- they start at 1 not 0!
- Small files (<64KB) are read automatically in full
- Large files require specifying line ranges (start_line and end_line)
- Caches file content for efficient subsequent reads and edits
- Must use absolute paths: `/path/to/file.txt` ✓, `file.txt` ✗

### edit_file  
- **Primary tool for both editing existing files AND creating new files**
- Applies multiple edits to a file atomically
- Processes edits in reverse line order to maintain line number integrity
- Supports insertions (set end_line to start_line - 1), replacements, and deletions
- Must use absolute paths: `/path/to/file.txt` ✓, `file.txt` ✗

#### Creating New Files
To create a new file, use edit_file with a single edit:
```json
{
  "path": "/absolute/path/to/new_file.txt",
  "edits": [{
    "start_line": 1,
    "end_line": 0,
    "new_content": "Hello, World!\nThis is a new file."
  }]
}
```

#### Editing Existing Files
For existing files, you can apply multiple edits in one operation:
```json
{
  "path": "existing_file.txt", 
  "edits": [
    {
      "start_line": 5,
      "end_line": 7,
      "new_content": "Replace lines 5-7 with this content"
    },
    {
      "start_line": 3,
      "end_line": 2,
      "new_content": "Insert this between lines 2 and 3"
    }
  ]
}
```

### Important Notes
- **Always check the FilesReadAndEdited section** in the system prompt for currently open files
- New files: Only single edit operations allowed (create content first, then edit in separate operations)
- Cached files: Will warn if file was modified externally since last read
- Automatic directory creation: Parent directories are created automatically if needed

### FilesReadAndEdited Section
The system prompt includes a special section called "FilesReadAndEdited" that contains:
- Keys: Canonical file paths of all files that have been read or edited
- Values: The current cached content of each file (with line numbers)

This section is automatically updated whenever you read or edit files, giving you a complete view of all open files in your workspace."#;

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileSlice {
    start_line: usize, // 1-indexed
    end_line: usize,   // 1-indexed, inclusive
    content: String,
}

#[derive(Debug, Clone)]
enum FileContent {
    Full(String),
    Partial {
        slices: Vec<FileSlice>,
        total_lines: usize,
    },
}

impl FileContent {
    fn get_numbered_content(&self) -> String {
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
                    if last_end > 0 && slice.start_line > last_end + 1 {
                        let omitted = slice.start_line - last_end - 1;
                        result.push(format!("[... {} lines omitted ...]", omitted));
                    } else if last_end == 0 && slice.start_line > 1 {
                        let omitted = slice.start_line - 1;
                        result.push(format!("[... {} lines omitted ...]", omitted));
                    }

                    result.push(slice.content.clone());
                    last_end = slice.end_line;
                }

                if last_end < *total_lines {
                    let omitted = total_lines - last_end;
                    result.push(format!("[... {} lines omitted ...]", omitted));
                }

                result.join("\n")
            }
        }
    }

    fn merge_slice(&mut self, new_slice: FileSlice) {
        if let FileContent::Partial { slices, .. } = self {
            slices.push(new_slice);
            slices.sort_by_key(|s| s.start_line);

            let mut merged = Vec::new();
            for slice in slices.drain(..) {
                if merged.is_empty() {
                    merged.push(slice);
                } else {
                    let last = merged.last_mut().unwrap();
                    if slice.start_line <= last.end_line + 1 {
                        if slice.end_line > last.end_line {
                            let new_lines: Vec<&str> = slice.content.lines().collect();

                            let overlap = if last.end_line >= slice.start_line {
                                last.end_line - slice.start_line + 1
                            } else {
                                0
                            };

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

#[derive(Debug, Clone)]
struct FileCacheEntry {
    content: FileContent,
    _read_at: SystemTime,
    last_modified_at_read: SystemTime,
    _size_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct ReadFileParams {
    path: String,
    start_line: Option<i32>,
    end_line: Option<i32>,
}

#[derive(Debug, Clone, PartialEq)]
struct Edit {
    start_line: usize, // 1-indexed
    end_line: usize,   // 1-indexed, inclusive
    new_content: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct EditFileParams {
    path: String,
    edits: Vec<serde_json::Value>,
}

#[derive(hive_actor_utils::actors::macros::Actor)]
struct FileInteractionActor {
    scope: String,
    cache: HashMap<PathBuf, FileCacheEntry>,
}

impl GeneratedActorTrait for FileInteractionActor {
    fn new(scope: String, _config_str: String) -> Self {
        // Broadcast available tools
        let tools = vec![
            hive_actor_utils::llm_client_types::Tool {
                tool_type: "function".to_string(),
                function: hive_actor_utils::llm_client_types::ToolFunctionDefinition {
                    name: READ_FILE_NAME.to_string(),
                    description: READ_FILE_DESCRIPTION.to_string(),
                    parameters: serde_json::from_str(READ_FILE_SCHEMA).unwrap(),
                },
            },
            hive_actor_utils::llm_client_types::Tool {
                tool_type: "function".to_string(),
                function: hive_actor_utils::llm_client_types::ToolFunctionDefinition {
                    name: EDIT_FILE_NAME.to_string(),
                    description: EDIT_FILE_DESCRIPTION.to_string(),
                    parameters: serde_json::from_str(EDIT_FILE_SCHEMA).unwrap(),
                },
            },
        ];

        let _ = Self::broadcast_common_message(ToolsAvailable { tools });

        // Broadcast usage guide to Tools section
        let _ = Self::broadcast_common_message(SystemPromptContribution {
            agent: scope.clone(),
            key: "file_interaction:usage_guide".to_string(),
            content: SystemPromptContent::Text(FILE_TOOLS_USAGE_GUIDE.to_string()),
            priority: 900,
            section: Some(Section::Tools),
        });

        Self {
            scope,
            cache: HashMap::new(),
        }
    }

    fn handle_message(&mut self, message: MessageEnvelope) {
        // Only handle messages from our own scope
        if message.from_scope != self.scope {
            return;
        }

        if let Some(execute_tool) = Self::parse_as::<ExecuteTool>(&message) {
            match execute_tool.tool_call.function.name.as_str() {
                READ_FILE_NAME => self.handle_read_file(execute_tool),
                EDIT_FILE_NAME => self.handle_edit_file(execute_tool),
                _ => {}
            }
        }
    }

    fn destructor(&mut self) {
        // Clear cache on destruction
        self.cache.clear();
    }
}

impl FileInteractionActor {
    /// Broadcast unified system prompt contribution for all cached files
    fn update_unified_files_system_prompt(&self) {
        // Collect all files from cache
        let mut files = Vec::new();
        for (path, entry) in &self.cache {
            // Always use numbered content for consistent formatting
            let content = entry.content.get_numbered_content();

            files.push(serde_json::json!({
                "path": path.display().to_string(),
                "content": content
            }));
        }

        // Sort files by path for consistent ordering
        files.sort_by(|a, b| {
            a["path"]
                .as_str()
                .unwrap_or("")
                .cmp(b["path"].as_str().unwrap_or(""))
        });

        let data = serde_json::json!({
            "files": files
        });

        let default_template = r#"{% for file in data.files -%}
<file path="{{ file.path }}">{{ file.content }}</file>
{% endfor %}"#
            .to_string();

        let _ = Self::broadcast_common_message(SystemPromptContribution {
            agent: self.scope.clone(),
            key: "file_interaction:files_read_and_edited".to_string(),
            content: SystemPromptContent::Data {
                data,
                default_template,
            },
            priority: 500,
            section: Some(Section::Custom("FilesReadAndEdited".to_string())),
        });
    }

    fn handle_read_file(&mut self, execute_tool: ExecuteTool) {
        let tool_call_id = &execute_tool.tool_call.id;

        // Parse parameters
        let params: ReadFileParams =
            match serde_json::from_str(&execute_tool.tool_call.function.arguments) {
                Ok(params) => params,
                Err(e) => {
                    self.send_error_result(
                        tool_call_id,
                        format!("Failed to parse read_file parameters: {}", e),
                        UIDisplayInfo {
                            collapsed: "Parameter Error".to_string(),
                            expanded: Some(format!("Failed to parse parameters:\n{}", e)),
                        },
                    );
                    return;
                }
            };

        // Validate line numbers
        if let Some(start_line) = params.start_line {
            if start_line < 0 {
                self.send_error_result(
                    tool_call_id,
                    format!("Invalid start_line: {} - lines are 1-indexed.", start_line),
                    UIDisplayInfo {
                        collapsed: "Invalid Line Number".to_string(),
                        expanded: Some(format!(
                            "start_line must be positive (was: {})",
                            start_line
                        )),
                    },
                );
                return;
            }
        }

        if let Some(end_line) = params.end_line {
            if end_line < 0 {
                self.send_error_result(
                    tool_call_id,
                    format!("Invalid end_line: {} - lines are 1-indexed.", end_line),
                    UIDisplayInfo {
                        collapsed: "Invalid Line Number".to_string(),
                        expanded: Some(format!("end_line must be positive (was: {})", end_line)),
                    },
                );
                return;
            }
        }

        let start_line = params.start_line.map(|x| x as usize);
        let end_line = params.end_line.map(|x| x as usize);

        // Execute the read
        match self.get_or_read_file_content(&params.path, start_line, end_line) {
            Ok(content) => {
                // Update unified system prompt contribution for all files
                self.update_unified_files_system_prompt();

                let message = match (start_line, end_line) {
                    (Some(start), Some(end)) => {
                        format!("Read file: {} (lines {}-{})", params.path, start, end)
                    }
                    _ => format!("Read file: {}", params.path),
                };

                self.send_success_result(
                    tool_call_id,
                    message,
                    UIDisplayInfo {
                        collapsed: format!("Read: {}", params.path),
                        expanded: Some(content),
                    },
                );
            }
            Err(e) => {
                self.send_error_result(
                    tool_call_id,
                    e.to_string(),
                    UIDisplayInfo {
                        collapsed: "Read Error".to_string(),
                        expanded: Some(e.to_string()),
                    },
                );
            }
        }
    }

    fn handle_edit_file(&mut self, execute_tool: ExecuteTool) {
        let tool_call_id = &execute_tool.tool_call.id;

        // Parse parameters
        let params: EditFileParams =
            match serde_json::from_str(&execute_tool.tool_call.function.arguments) {
                Ok(params) => params,
                Err(e) => {
                    self.send_error_result(
                        tool_call_id,
                        format!("Failed to parse edit_file parameters: {}", e),
                        UIDisplayInfo {
                            collapsed: "Parameter Error".to_string(),
                            expanded: Some(format!("Failed to parse parameters:\n{}", e)),
                        },
                    );
                    return;
                }
            };

        // Parse edits
        let edits = match self.parse_edits_from_params(&params) {
            Ok(edits) => edits,
            Err(e) => {
                self.send_error_result(
                    tool_call_id,
                    e.to_string(),
                    UIDisplayInfo {
                        collapsed: "Edit Parse Error".to_string(),
                        expanded: Some(e.to_string()),
                    },
                );
                return;
            }
        };

        // Execute the edits
        match self.apply_edits(&params.path, edits) {
            Ok(message) => {
                // Update unified system prompt contribution for all files
                self.update_unified_files_system_prompt();

                self.send_success_result(
                    tool_call_id,
                    message.clone(),
                    UIDisplayInfo {
                        collapsed: format!("Edited: {}", params.path),
                        expanded: Some(message),
                    },
                );
            }
            Err(e) => {
                self.send_error_result(
                    tool_call_id,
                    e.to_string(),
                    UIDisplayInfo {
                        collapsed: "Edit Error".to_string(),
                        expanded: Some(e.to_string()),
                    },
                );
            }
        }
    }

    fn send_error_result(&self, tool_call_id: &str, error_msg: String, ui_display: UIDisplayInfo) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            status: ToolCallStatus::Done {
                result: Err(ToolCallResult {
                    content: error_msg,
                    ui_display_info: ui_display,
                }),
            },
        };

        let _ = Self::broadcast_common_message(update);
    }

    fn send_success_result(&self, tool_call_id: &str, result: String, ui_display: UIDisplayInfo) {
        let update = ToolCallStatusUpdate {
            id: tool_call_id.to_string(),
            status: ToolCallStatus::Done {
                result: Ok(ToolCallResult {
                    content: result,
                    ui_display_info: ui_display,
                }),
            },
        };

        let _ = Self::broadcast_common_message(update);
    }

    fn get_or_read_file_content<P: AsRef<Path>>(
        &mut self,
        path: P,
        start_line: Option<usize>,
        end_line: Option<usize>,
    ) -> Result<String, String> {
        let path_ref = path.as_ref();
        let canonical_path = wasm_safe_normalize_path(path_ref)
            .map_err(|e| format!("Failed to normalize path '{}': {}", path_ref.display(), e))?;

        // Check if we need to read or can use cache
        let needs_read = match (start_line, end_line) {
            (None, None) => {
                // Full file read - check if modified
                self.has_been_modified(&canonical_path)?
            }
            (Some(start), Some(end)) => {
                // Partial read - check if we have this slice or file was modified
                if self.has_been_modified(&canonical_path)? {
                    true
                } else if let Some(entry) = self.cache.get(&canonical_path) {
                    // Check if we already have this slice
                    match &entry.content {
                        FileContent::Full(_) => false, // We have the full file
                        FileContent::Partial { slices, .. } => {
                            // Check if the requested range is already covered
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
                return Err(
                    "Both start_line and end_line must be provided for partial reads".to_string(),
                );
            }
        };

        if needs_read {
            self.read_and_cache_file(path_ref, start_line, end_line)?;
        }

        self.cache
            .get(&canonical_path)
            .map(|entry| entry.content.get_numbered_content())
            .ok_or_else(|| {
                format!(
                    "File '{}' should be in cache but was not found",
                    canonical_path.display()
                )
            })
    }

    fn read_and_cache_file<P: AsRef<Path>>(
        &mut self,
        path: P,
        start_line: Option<usize>,
        end_line: Option<usize>,
    ) -> Result<(), String> {
        let path_ref = path.as_ref();

        let canonical_path = wasm_safe_normalize_path(path_ref)
            .map_err(|e| format!("Failed to normalize path '{}': {}", path_ref.display(), e))?;

        // Check if we can merge with existing partial content
        if let (Some(start), Some(end)) = (start_line, end_line) {
            if let Some(existing_entry) = self.cache.get(&canonical_path).cloned() {
                // Only merge if file hasn't been modified and we have partial content
                if !self.has_been_modified(path_ref)? {
                    if let FileContent::Partial { slices, .. } = &existing_entry.content {
                        // Check if we already have this exact slice
                        let already_covered = slices
                            .iter()
                            .any(|slice| slice.start_line <= start && slice.end_line >= end);

                        if already_covered {
                            // We already have this slice, no need to read again
                            return Ok(());
                        }

                        // Read the file and create new slice to merge
                        let contents = fs::read_to_string(path_ref).map_err(|e| {
                            format!("Failed to read file '{}': {}", path_ref.display(), e)
                        })?;

                        let lines: Vec<&str> = contents.lines().collect();
                        let total_lines = lines.len();

                        // Validate line numbers
                        if start < 1 || start > total_lines || end < start {
                            return Err(format!(
                                "Invalid line range: {}-{} (file has {} lines)",
                                start, end, total_lines
                            ));
                        }

                        // Create new slice
                        let slice_lines: Vec<String> = lines[(start - 1)..lines.len().min(end)]
                            .iter()
                            .enumerate()
                            .map(|(i, line)| format!("{}|{}", start + i, line))
                            .collect();

                        let new_slice = FileSlice {
                            start_line: start,
                            end_line: end.min(total_lines),
                            content: slice_lines.join("\n"),
                        };

                        // Merge with existing entry
                        let mut updated_entry = existing_entry;
                        updated_entry.content.merge_slice(new_slice);
                        self.cache.insert(canonical_path, updated_entry);
                        return Ok(());
                    }
                }
            }
        }

        // Fall back to normal read/cache logic (full read or fresh partial read)

        let metadata = fs::metadata(path_ref).map_err(|e| {
            format!(
                "Failed to read file metadata for '{}': {}",
                path_ref.display(),
                e
            )
        })?;

        let file_size = metadata.len();
        let last_modified_at_read = metadata.modified().map_err(|e| {
            format!(
                "Failed to get modified time for '{}': {}",
                path_ref.display(),
                e
            )
        })?;

        // Check if file is absolutely too large to even attempt reading
        if file_size > MAX_FILE_SIZE_BYTES {
            return Err(format!(
                "File '{}' is too large ({} bytes). Maximum file size is {} bytes.",
                path_ref.display(),
                file_size,
                MAX_FILE_SIZE_BYTES
            ));
        }

        let contents = fs::read_to_string(path_ref)
            .map_err(|e| format!("Failed to read file '{}': {}", path_ref.display(), e))?;

        let lines: Vec<&str> = contents.lines().collect();
        let total_lines = lines.len();

        // Determine if we're reading the full file or a slice
        let content = match (start_line, end_line) {
            (None, None) => {
                // Check if file is small enough for automatic full read
                if file_size > SMALL_FILE_SIZE_BYTES {
                    let metadata = serde_json::json!({
                        "path": path_ref.display().to_string(),
                        "size_bytes": file_size,
                        "total_lines": total_lines
                    });
                    return Err(format!(
                        "File too large. You must specify a line range (start_line and end_line). File metadata: {}",
                        serde_json::to_string(&metadata).unwrap()
                    ));
                }
                FileContent::Full(contents)
            }
            (Some(start), Some(end)) => {
                // Validate line numbers
                if start < 1 || start > total_lines || end < start {
                    return Err(format!(
                        "Invalid line range: {}-{} (file has {} lines)",
                        start, end, total_lines
                    ));
                }

                // Create numbered content for the slice
                let slice_lines: Vec<String> = lines[(start - 1)..lines.len().min(end)]
                    .iter()
                    .enumerate()
                    .map(|(i, line)| format!("{}|{}", start + i, line))
                    .collect();

                let slice = FileSlice {
                    start_line: start,
                    end_line: end.min(total_lines),
                    content: slice_lines.join("\n"),
                };

                FileContent::Partial {
                    slices: vec![slice],
                    total_lines,
                }
            }
            _ => {
                return Err(
                    "Both start_line and end_line must be provided for partial reads".to_string(),
                );
            }
        };

        let read_at = SystemTime::now();

        let entry = FileCacheEntry {
            content,
            _read_at: read_at,
            last_modified_at_read,
            _size_bytes: file_size,
        };

        self.cache.insert(canonical_path, entry);

        Ok(())
    }

    fn has_been_modified<P: AsRef<Path>>(&self, path: P) -> Result<bool, String> {
        let path_ref = path.as_ref();

        let canonical_path = match wasm_safe_normalize_path(path_ref) {
            Ok(p) => p,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(true);
            }
            Err(e) => {
                return Err(format!(
                    "Failed to normalize path '{}': {}",
                    path_ref.display(),
                    e
                ));
            }
        };

        if let Some(cached_entry) = self.cache.get(&canonical_path) {
            match fs::metadata(&canonical_path) {
                Ok(current_metadata) => {
                    let current_mtime = current_metadata.modified().map_err(|e| {
                        format!(
                            "Failed to get modified time for '{}': {}",
                            canonical_path.display(),
                            e
                        )
                    })?;
                    Ok(current_mtime != cached_entry.last_modified_at_read)
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(true),
                Err(e) => Err(format!(
                    "Failed to read metadata for '{}': {}",
                    canonical_path.display(),
                    e
                )),
            }
        } else {
            Ok(true)
        }
    }

    fn parse_edits_from_params(&self, params: &EditFileParams) -> Result<Vec<Edit>, String> {
        let mut edits = Vec::new();

        for edit_obj in &params.edits {
            let start_line = edit_obj
                .get("start_line")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| "Missing required field: start_line".to_string())?
                as usize;

            let end_line = edit_obj
                .get("end_line")
                .and_then(|v| v.as_i64()) // Use i64 to handle -1
                .ok_or_else(|| "Missing required field: end_line".to_string())?
                as i64;

            // Convert -1 or negative values to start_line - 1 for insertions
            let end_line = if end_line < 0 {
                (start_line as i64 - 1) as usize
            } else {
                end_line as usize
            };

            let new_content = edit_obj
                .get("new_content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required field: new_content".to_string())?
                .to_string();

            edits.push(Edit {
                start_line,
                end_line,
                new_content,
            });
        }

        Ok(edits)
    }

    fn apply_edits(&mut self, path: &str, mut edits: Vec<Edit>) -> Result<String, String> {
        let path_ref = Path::new(path);

        // Check if path is absolute first
        if !path_ref.is_absolute() {
            return Err(format!(
                "Relative paths are not supported. Please use an absolute path starting with '/'. Got: '{}'",
                path
            ));
        }

        // Check if the file exists
        if !path_ref.exists() {
            // Create parent directories if needed
            if let Some(parent) = path_ref.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directory: {}", e))?;
            }
            // Create an empty file - we'll populate it with the edits
            fs::File::create(&path).map_err(|e| format!("Failed to create file: {}", e))?;
        }

        let canonical_path = wasm_safe_normalize_path(path_ref)
            .map_err(|e| format!("Failed to normalize path '{}': {}", path_ref.display(), e))?;

        // Check the file's modification time if it's in cache
        if let Some(entry) = self.cache.get(&canonical_path) {
            let metadata = fs::metadata(&canonical_path)
                .map_err(|e| format!("Failed to read file metadata: {}", e))?;
            let current_mtime = metadata
                .modified()
                .map_err(|e| format!("Failed to get modified time: {}", e))?;

            if current_mtime != entry.last_modified_at_read {
                return Err(format!(
                    "File '{}' has been modified since last read. Please use the read_file tool first.",
                    canonical_path.display()
                ));
            }
        }
        // Note: We no longer require files to be read first before editing

        // Read the file into a Vec<String>
        let content = fs::read_to_string(&canonical_path)
            .map_err(|e| format!("Failed to read file: {}", e))?;
        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        let is_empty_file = lines.is_empty();

        // For empty files, only allow single edit operations that create content
        if is_empty_file && edits.len() > 1 {
            return Err(format!(
                "Cannot apply multiple edits to a new/empty file. For new files, use a single edit operation to create the initial content. Found {} edits.",
                edits.len()
            ));
        }

        // Sort edits in descending order by start_line to preserve line numbers during editing
        edits.sort_by(|a, b| b.start_line.cmp(&a.start_line));

        // Apply each edit
        for edit in edits {
            let current_total_lines = lines.len();

            // Validate line numbers based on current line count
            if edit.start_line < 1 || edit.start_line > current_total_lines + 1 {
                return Err(format!(
                    "Invalid line numbers for edit: start={}, end={}, current_lines={}",
                    edit.start_line, edit.end_line, current_total_lines
                ));
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
            } else {
                // Replace or delete lines
                if edit.end_line < edit.start_line || edit.end_line > current_total_lines {
                    return Err(format!(
                        "Invalid line numbers for edit: start={}, end={}, current_lines={}",
                        edit.start_line, edit.end_line, current_total_lines
                    ));
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
            }
        }

        // Write the modified content back to disk
        let new_content = if lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", lines.join("\n"))
        };
        fs::write(&canonical_path, &new_content)
            .map_err(|e| format!("Failed to write file: {}", e))?;

        // Update the cache with new content
        let _ = self.read_and_cache_file(&canonical_path, None, None);

        Ok(format!(
            "Successfully edited file: {}",
            canonical_path.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(slice.content, "5|line 5\n6|line 6\n7|line 7");
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
        let expected = "1|line 1\n2|line 2\n[... 2 lines omitted ...]\n5|line 5\n6|line 6\n[... 4 lines omitted ...]";
        assert_eq!(numbered, expected);
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
            assert_eq!(
                slices[0].content,
                "1|line 1\n2|line 2\n3|line 3\n4|line 4\n5|line 5"
            );
        } else {
            panic!("Expected partial content");
        }
    }

    #[test]
    fn test_edit_parse() {
        let params = EditFileParams {
            path: "/tmp/test.txt".to_string(),
            edits: vec![serde_json::json!({
                "start_line": 1,
                "end_line": 1,
                "new_content": "Hello, world!"
            })],
        };

        let actor = FileInteractionActor {
            scope: "test".to_string(),
            cache: HashMap::new(),
        };

        let result = actor.parse_edits_from_params(&params);
        assert!(result.is_ok());

        let edits = result.unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].start_line, 1);
        assert_eq!(edits[0].end_line, 1);
        assert_eq!(edits[0].new_content, "Hello, world!");
    }

    #[test]
    fn test_file_reader_slice_merging() {
        let mut actor = FileInteractionActor {
            scope: "test".to_string(),
            cache: HashMap::new(),
        };
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create a file with several lines
        let content =
            "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10";
        fs::write(&file_path, content).unwrap();

        // Read first slice
        actor
            .read_and_cache_file(&file_path, Some(1), Some(3))
            .unwrap();

        // Read overlapping slice - should merge
        let result = actor.get_or_read_file_content(&file_path, Some(3), Some(6));
        assert!(result.is_ok());

        // Check that slices were merged
        let cached = actor
            .cache
            .get(&wasm_safe_normalize_path(&file_path).unwrap())
            .unwrap();
        if let FileContent::Partial { slices, .. } = &cached.content {
            // Should have merged into one slice covering lines 1-6
            assert_eq!(slices.len(), 1);
            assert_eq!(slices[0].start_line, 1);
            assert_eq!(slices[0].end_line, 6);
        } else {
            panic!("Expected partial content");
        }
    }

    #[test]
    fn test_file_reader_slice_already_covered() {
        let mut actor = FileInteractionActor {
            scope: "test".to_string(),
            cache: HashMap::new(),
        };
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create a file with several lines
        let content = "line 1\nline 2\nline 3\nline 4\nline 5";
        fs::write(&file_path, content).unwrap();

        // Read first slice
        actor
            .read_and_cache_file(&file_path, Some(1), Some(5))
            .unwrap();

        // Try to read a subset - should be already covered
        let result = actor.read_and_cache_file(&file_path, Some(2), Some(4));
        assert!(result.is_ok());

        // Verify we still have just one slice
        let cached = actor
            .cache
            .get(&wasm_safe_normalize_path(&file_path).unwrap())
            .unwrap();
        if let FileContent::Partial { slices, .. } = &cached.content {
            assert_eq!(slices.len(), 1);
            assert_eq!(slices[0].start_line, 1);
            assert_eq!(slices[0].end_line, 5);
        } else {
            panic!("Expected partial content");
        }
    }

    #[test]
    fn test_file_reader_no_merge_on_modified_file() {
        let mut actor = FileInteractionActor {
            scope: "test".to_string(),
            cache: HashMap::new(),
        };
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create initial file
        let content = "line 1\nline 2\nline 3\nline 4\nline 5";
        fs::write(&file_path, content).unwrap();

        // Read first slice
        actor
            .read_and_cache_file(&file_path, Some(1), Some(3))
            .unwrap();

        // Modify the file
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(
            &file_path,
            "modified line 1\nmodified line 2\nmodified line 3\nmodified line 4\nmodified line 5",
        )
        .unwrap();

        // Try to read another slice - should not merge due to modification
        let result = actor.read_and_cache_file(&file_path, Some(4), Some(5));
        assert!(result.is_ok());

        // Should have fresh cache entry for lines 4-5, not merged
        let cached = actor
            .cache
            .get(&wasm_safe_normalize_path(&file_path).unwrap())
            .unwrap();
        if let FileContent::Partial { slices, .. } = &cached.content {
            // Should be a fresh slice, not merged
            assert_eq!(slices.len(), 1);
            assert_eq!(slices[0].start_line, 4);
            assert_eq!(slices[0].end_line, 5);
        } else {
            panic!("Expected partial content");
        }
    }

    // --- Comprehensive Edit Operation Tests ---

    #[test]
    fn test_edit_create_new_file() {
        let mut actor = FileInteractionActor {
            scope: "test".to_string(),
            cache: HashMap::new(),
        };
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("new_file.txt");

        // Create a new file with content
        let edits = vec![Edit {
            start_line: 1,
            end_line: 0,
            new_content: "Hello, World!\nThis is a new file.".to_string(),
        }];

        let result = actor.apply_edits(file_path.to_str().unwrap(), edits);
        assert!(result.is_ok());

        // Verify file was created and has correct content
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Hello, World!\nThis is a new file.\n");
    }

    #[test]
    fn test_edit_create_new_file_with_multiple_operations_error() {
        let mut actor = FileInteractionActor {
            scope: "test".to_string(),
            cache: HashMap::new(),
        };
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("multi_edit_new.txt");

        // Try to create a new file with multiple edit operations - should fail
        let edits = vec![
            Edit {
                start_line: 1,
                end_line: 0,
                new_content: "Line 1\nLine 2\nLine 3".to_string(),
            },
            Edit {
                start_line: 2,
                end_line: 2,
                new_content: "Modified Line 2".to_string(),
            },
        ];

        let result = actor.apply_edits(file_path.to_str().unwrap(), edits);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Cannot apply multiple edits to a new/empty file")
        );
    }

    #[test]
    fn test_edit_create_then_edit_workflow() {
        let mut actor = FileInteractionActor {
            scope: "test".to_string(),
            cache: HashMap::new(),
        };
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("workflow.txt");

        // Step 1: Create file with initial content
        let create_edit = vec![Edit {
            start_line: 1,
            end_line: 0,
            new_content: "Line 1\nLine 2\nLine 3".to_string(),
        }];

        let result = actor.apply_edits(file_path.to_str().unwrap(), create_edit);
        assert!(result.is_ok());

        // Step 2: Now we can edit the existing file
        let modify_edit = vec![Edit {
            start_line: 2,
            end_line: 2,
            new_content: "Modified Line 2".to_string(),
        }];

        let result = actor.apply_edits(file_path.to_str().unwrap(), modify_edit);
        assert!(result.is_ok());

        // Verify the final content
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Line 1\nModified Line 2\nLine 3\n");
    }

    #[test]
    fn test_edit_empty_file() {
        let mut actor = FileInteractionActor {
            scope: "test".to_string(),
            cache: HashMap::new(),
        };
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("empty.txt");

        // Create an empty file
        fs::File::create(&file_path).unwrap();

        // Add content to empty file
        let edits = vec![Edit {
            start_line: 1,
            end_line: 0,
            new_content: "First line\nSecond line".to_string(),
        }];

        let result = actor.apply_edits(file_path.to_str().unwrap(), edits);
        assert!(result.is_ok());

        // Verify content
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "First line\nSecond line\n");
    }

    #[test]
    fn test_edit_single_line_operations() {
        let mut actor = FileInteractionActor {
            scope: "test".to_string(),
            cache: HashMap::new(),
        };
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("single_ops.txt");

        // Create initial file
        let initial_content = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5";
        fs::write(&file_path, initial_content).unwrap();

        // Test replace operation
        let edits = vec![Edit {
            start_line: 3,
            end_line: 3,
            new_content: "Modified Line 3".to_string(),
        }];

        let result = actor.apply_edits(file_path.to_str().unwrap(), edits);
        assert!(result.is_ok());

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Line 1\nLine 2\nModified Line 3\nLine 4\nLine 5\n");
    }

    #[test]
    fn test_edit_insert_lines() {
        let mut actor = FileInteractionActor {
            scope: "test".to_string(),
            cache: HashMap::new(),
        };
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("insert_test.txt");

        // Create initial file
        let initial_content = "Line 1\nLine 2\nLine 3";
        fs::write(&file_path, initial_content).unwrap();

        // Insert new lines between line 2 and 3
        let edits = vec![Edit {
            start_line: 3,
            end_line: 2, // end_line < start_line indicates insertion
            new_content: "Inserted Line A\nInserted Line B".to_string(),
        }];

        let result = actor.apply_edits(file_path.to_str().unwrap(), edits);
        assert!(result.is_ok());

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(
            content,
            "Line 1\nLine 2\nInserted Line A\nInserted Line B\nLine 3\n"
        );
    }

    #[test]
    fn test_edit_delete_lines() {
        let mut actor = FileInteractionActor {
            scope: "test".to_string(),
            cache: HashMap::new(),
        };
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("delete_test.txt");

        // Create initial file
        let initial_content = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5";
        fs::write(&file_path, initial_content).unwrap();

        // Delete lines 2-4
        let edits = vec![Edit {
            start_line: 2,
            end_line: 4,
            new_content: "".to_string(),
        }];

        let result = actor.apply_edits(file_path.to_str().unwrap(), edits);
        assert!(result.is_ok());

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Line 1\nLine 5\n");
    }

    #[test]
    fn test_edit_multiple_operations_sorted() {
        let mut actor = FileInteractionActor {
            scope: "test".to_string(),
            cache: HashMap::new(),
        };
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("multi_ops.txt");

        // Create initial file
        let initial_content = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5";
        fs::write(&file_path, initial_content).unwrap();

        // Multiple edits - should be processed in reverse order
        let edits = vec![
            Edit {
                start_line: 2,
                end_line: 2,
                new_content: "Modified Line 2".to_string(),
            },
            Edit {
                start_line: 4,
                end_line: 4,
                new_content: "Modified Line 4".to_string(),
            },
            Edit {
                start_line: 6,
                end_line: 5, // Insert after line 5
                new_content: "New Line 6".to_string(),
            },
        ];

        let result = actor.apply_edits(file_path.to_str().unwrap(), edits);
        assert!(result.is_ok());

        let content = fs::read_to_string(&file_path).unwrap();
        let expected = "Line 1\nModified Line 2\nLine 3\nModified Line 4\nLine 5\nNew Line 6\n";
        assert_eq!(content, expected);
    }

    #[test]
    fn test_edit_error_invalid_line_numbers() {
        let mut actor = FileInteractionActor {
            scope: "test".to_string(),
            cache: HashMap::new(),
        };
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("invalid_lines.txt");

        // Create initial file
        let initial_content = "Line 1\nLine 2\nLine 3";
        fs::write(&file_path, initial_content).unwrap();

        // Try to edit line that doesn't exist
        let edits = vec![Edit {
            start_line: 10, // File only has 3 lines
            end_line: 10,
            new_content: "Invalid".to_string(),
        }];

        let result = actor.apply_edits(file_path.to_str().unwrap(), edits);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid line numbers"));
    }

    #[test]
    fn test_edit_file_without_read_first() {
        let mut actor = FileInteractionActor {
            scope: "test".to_string(),
            cache: HashMap::new(),
        };
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("no_read_first.txt");

        // Create initial file
        let initial_content = "Original content";
        fs::write(&file_path, initial_content).unwrap();

        // Edit file without reading it first - should work now
        let edits = vec![Edit {
            start_line: 1,
            end_line: 1,
            new_content: "Modified content".to_string(),
        }];

        let result = actor.apply_edits(file_path.to_str().unwrap(), edits);
        assert!(result.is_ok());

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Modified content\n");
    }

    #[test]
    fn test_edit_file_staleness_check() {
        let mut actor = FileInteractionActor {
            scope: "test".to_string(),
            cache: HashMap::new(),
        };
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("staleness.txt");

        // Create and read file into cache
        let initial_content = "Original content";
        fs::write(&file_path, initial_content).unwrap();
        actor.read_and_cache_file(&file_path, None, None).unwrap();

        // Modify file externally
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&file_path, "Externally modified").unwrap();

        // Try to edit - should fail due to staleness
        let edits = vec![Edit {
            start_line: 1,
            end_line: 1,
            new_content: "Should fail".to_string(),
        }];

        let result = actor.apply_edits(file_path.to_str().unwrap(), edits);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("has been modified since last read")
        );
    }

    #[test]
    fn test_edit_create_nested_directory() {
        let mut actor = FileInteractionActor {
            scope: "test".to_string(),
            cache: HashMap::new(),
        };
        let temp_dir = TempDir::new().unwrap();
        let nested_path = temp_dir.path().join("nested").join("deep").join("file.txt");

        // Create file in nested directory that doesn't exist
        let edits = vec![Edit {
            start_line: 1,
            end_line: 0,
            new_content: "Content in nested directory".to_string(),
        }];

        let result = actor.apply_edits(nested_path.to_str().unwrap(), edits);
        assert!(result.is_ok());

        // Verify file and directories were created
        assert!(nested_path.exists());
        let content = fs::read_to_string(&nested_path).unwrap();
        assert_eq!(content, "Content in nested directory\n");
    }

    #[test]
    fn test_documentation_example_file_creation() {
        let mut actor = FileInteractionActor {
            scope: "test".to_string(),
            cache: HashMap::new(),
        };
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("doc_example.txt");

        // Test the exact example from our documentation
        let edits = vec![Edit {
            start_line: 1,
            end_line: 0,
            new_content: "Hello, World!\nThis is a new file.".to_string(),
        }];

        let result = actor.apply_edits(file_path.to_str().unwrap(), edits);
        assert!(result.is_ok());

        // Verify it matches the documentation example
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Hello, World!\nThis is a new file.\n");
    }
}
