use std::{
    collections::HashMap,
    fs, io,
    path::{Component as PathComponent, Path, PathBuf},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};
use similar::TextDiff;
use wasmind_actor_utils::common_messages::tools::UIDisplayInfo;

/// Helper function to format strings for error messages
fn format_string_for_error(s: &str) -> String {
    if s.len() > 200 {
        format!("\"{}...\" (truncated, {} chars total)", &s[..200], s.len())
    } else {
        format!("\"{}\"", s.replace('\n', "\\n").replace('\t', "\\t"))
    }
}

// Tool constants
pub const READ_FILE_NAME: &str = "read_file";
pub const READ_FILE_DESCRIPTION: &str = "Reads content from a file. For small files, it reads the entire file. For large files, it returns an error with metadata, requiring you to specify a line range. All returned file content is prefixed with line numbers in the format LINE_NUMBER:CONTENT. You can read a specific chunk by providing start_line and end_line.";
pub const READ_FILE_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "path": {
            "type": "string",
            "description": "Path to the file. Can be iether absolute (starting with /) or relative to the current working directory."
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

pub const EDIT_FILE_NAME: &str = "edit_file";
pub const EDIT_FILE_DESCRIPTION: &str = "Applies a list of search-and-replace edits to a file atomically. This is the primary tool for modifying files AND creating new files. Each edit finds an exact string match and replaces it. Supports both absolute paths (starting with /) and relative paths. To create a new file, use an edit with empty old_string.";
pub const EDIT_FILE_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "path": {
            "type": "string",
            "description": "Path to the file to edit or create. Can be either absolute (starting with /) or relative to the current working directory. If the file doesn't exist, it will be created (along with any necessary parent directories)."
        },
        "edits": {
            "type": "array",
            "description": "A list of search-and-replace edits to apply sequentially.",
            "items": {
                "type": "object",
                "properties": {
                    "old_string": {
                        "type": "string",
                        "description": "Exact string to find in the file. Must match exactly including all whitespace, indentation, and newlines. For new files, use empty string."
                    },
                    "new_string": {
                        "type": "string",
                        "description": "String to replace the old_string with. Can be empty to delete the old_string."
                    },
                    "replace_all": {
                        "type": "boolean",
                        "description": "If true, replace all occurrences of old_string. If false (default), replace only one occurrence and error if multiple matches found.",
                        "default": false
                    }
                },
                "required": ["old_string", "new_string"]
            }
        }
    },
    "required": ["path", "edits"]
}"#;

pub const FILE_TOOLS_USAGE_GUIDE: &str = r#"## File Interaction Tools

### read_file
Reads files with line numbers. Supports absolute and relative paths.

### edit_file - Search-and-Replace Editing
Uses exact string matching. Must read files before editing.

**Basic Example:**
```json
{
  "path": "file.py",
  "edits": [{
    "old_string": "def old_func():\n    return 1",
    "new_string": "def new_func():\n    return 2"
  }]
}
```

**Create new files (empty old_string):**
```json
{
  "path": "new.py", 
  "edits": [{
    "old_string": "",
    "new_string": "print('hello')"
  }]
}
```

**Replace all occurrences:**
```json
{
  "edits": [{
    "old_string": "old_var",
    "new_string": "new_var", 
    "replace_all": true
  }]
}
```

**Key Requirements:**
- Exact whitespace matching (copy from read_file output)
- Include context if string appears multiple times
- Use `replace_all: true` for multiple matches"#;

/// WASM-safe path normalization that supports both absolute and relative paths
fn wasm_safe_normalize_path(
    path: &Path,
    working_directory: &PathBuf,
    verify_exists: bool,
) -> Result<PathBuf, io::Error> {
    // Convert relative paths to absolute using the working directory
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        working_directory.join(path)
    };

    // Optionally check if the file exists to ensure we're working with a valid path
    // This also validates the path without needing canonicalize
    if verify_exists {
        fs::metadata(&absolute_path)?;
    }

    // Clean and normalize the path components
    Ok(clean_path_components(&absolute_path))
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileSlice {
    start_line: usize,  // 1-indexed
    end_line: usize,    // 1-indexed, inclusive
    lines: Vec<String>, // Raw lines without line numbers
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
                .map(|(i, line)| format!("{}:{}", i + 1, line))
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
                        result.push(format!("[... {omitted} lines omitted ...]"));
                    } else if last_end == 0 && slice.start_line > 1 {
                        let omitted = slice.start_line - 1;
                        result.push(format!("[... {omitted} lines omitted ...]"));
                    }

                    // Format the lines with line numbers on output
                    for (i, line) in slice.lines.iter().enumerate() {
                        result.push(format!("{}:{}", slice.start_line + i, line));
                    }
                    last_end = slice.end_line;
                }

                if last_end < *total_lines {
                    let omitted = total_lines - last_end;
                    result.push(format!("[... {omitted} lines omitted ...]"));
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
                            // Calculate how many lines overlap
                            let overlap = if last.end_line >= slice.start_line {
                                last.end_line - slice.start_line + 1
                            } else {
                                0
                            };

                            // Add non-overlapping lines from the new slice
                            for line in slice.lines.iter().skip(overlap) {
                                last.lines.push(line.clone());
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
    last_modified_at_read: SystemTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReadFileParams {
    pub path: String,
    pub start_line: Option<i32>,
    pub end_line: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edit {
    pub old_string: String,
    pub new_string: String,
    #[serde(default)]
    pub replace_all: bool, // Default: false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditFileParams {
    pub path: String,
    pub edits: Vec<Edit>,
}

#[derive(Debug, Clone)]
pub struct ReadFileResult {
    pub message: String,
    pub ui_display: UIDisplayInfo,
}

#[derive(Debug, Clone)]
pub struct ReadFileError {
    pub error_msg: String,
    pub ui_display: UIDisplayInfo,
}

#[derive(Debug, Clone)]
pub struct EditFileResult {
    pub message: String,
    pub ui_display: UIDisplayInfo,
}

#[derive(Debug, Clone)]
pub struct EditFileError {
    pub error_msg: String,
    pub ui_display: UIDisplayInfo,
}

pub struct FileInteractionManager {
    cache: HashMap<PathBuf, FileCacheEntry>,
    working_directory: PathBuf,
}

impl FileInteractionManager {
    pub fn new(working_directory: PathBuf) -> Self {
        Self {
            cache: HashMap::new(),
            working_directory,
        }
    }

    pub fn set_working_directory(&mut self, working_directory: PathBuf) {
        self.working_directory = working_directory;
    }

    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    pub fn get_files_info(&self) -> Vec<(PathBuf, String)> {
        let mut files = Vec::new();
        for (path, entry) in &self.cache {
            let content = entry.content.get_numbered_content();
            files.push((path.clone(), content));
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
        files
    }

    pub fn read_file(&mut self, params: ReadFileParams) -> Result<ReadFileResult, ReadFileError> {
        // Validate line numbers
        if let Some(start_line) = params.start_line {
            if start_line < 0 {
                return Err(ReadFileError {
                    error_msg: format!("Invalid start_line: {start_line} - lines are 1-indexed."),
                    ui_display: UIDisplayInfo {
                        collapsed: format!("{}: Invalid line number", params.path),
                        expanded: Some(format!(
                            "File: {}\nError: start_line must be positive (was: {})",
                            params.path, start_line
                        )),
                    },
                });
            }
        }

        if let Some(end_line) = params.end_line {
            if end_line < 0 {
                return Err(ReadFileError {
                    error_msg: format!("Invalid end_line: {end_line} - lines are 1-indexed."),
                    ui_display: UIDisplayInfo {
                        collapsed: format!("{}: Invalid line number", params.path),
                        expanded: Some(format!(
                            "File: {}\nError: end_line must be positive (was: {})",
                            params.path, end_line
                        )),
                    },
                });
            }
        }

        let start_line = params.start_line.map(|x| x as usize);
        let end_line = params.end_line.map(|x| x as usize);

        match self.get_or_read_file_content(&params.path, start_line, end_line) {
            Ok(content) => {
                // Get the canonical path to look up cache info
                let path_ref = Path::new(&params.path);
                let canonical_path =
                    wasm_safe_normalize_path(path_ref, &self.working_directory, true).map_err(
                        |e| ReadFileError {
                            error_msg: format!("Failed to normalize path: {}", e),
                            ui_display: UIDisplayInfo {
                                collapsed: format!("{}: Path error", params.path),
                                expanded: Some(format!(
                                    "File: {}\nError: Failed to normalize path: {}",
                                    params.path, e
                                )),
                            },
                        },
                    )?;

                // Get total lines info from cache
                let (total_lines, actual_start, actual_end) =
                    if let Some(entry) = self.cache.get(&canonical_path) {
                        match &entry.content {
                            FileContent::Full(full_content) => {
                                let total = full_content.lines().count();
                                (total, 1, total)
                            }
                            FileContent::Partial { total_lines, .. } => {
                                // Find the actual range that was read
                                if let (Some(req_start), Some(req_end)) = (start_line, end_line) {
                                    // Check what was actually read
                                    let actual_end = req_end.min(*total_lines);
                                    (*total_lines, req_start, actual_end)
                                } else {
                                    (*total_lines, 1, *total_lines)
                                }
                            }
                        }
                    } else {
                        // Fallback if not in cache (shouldn't happen)
                        let line_count = content.lines().count();
                        (line_count, 1, line_count)
                    };

                // Build message for LLM
                let message = match (start_line, end_line) {
                    (Some(req_start), Some(req_end)) => {
                        if req_end > total_lines {
                            // Requested more lines than exist
                            format!(
                                "Read file: {} (requested lines {}-{}, file has {} lines - read {})",
                                params.path,
                                req_start,
                                req_end,
                                total_lines,
                                if actual_end == total_lines && actual_start == 1 {
                                    "complete file".to_string()
                                } else {
                                    format!("lines {}-{}", actual_start, actual_end)
                                }
                            )
                        } else if actual_end == total_lines && actual_start == 1 {
                            format!(
                                "Read file: {} (lines {}-{} of {} total - read complete file)",
                                params.path, actual_start, actual_end, total_lines
                            )
                        } else {
                            let remaining = total_lines - actual_end;
                            format!(
                                "Read file: {} (lines {}-{} of {} total - {} lines remaining)",
                                params.path, actual_start, actual_end, total_lines, remaining
                            )
                        }
                    }
                    _ => {
                        format!(
                            "Read file: {} ({} lines total - read complete file)",
                            params.path, total_lines
                        )
                    }
                };

                let expanded = format!(
                    "File: {}\nTotal lines: {}\n\n{}",
                    params.path, total_lines, content
                );

                Ok(ReadFileResult {
                    message: message.clone(),
                    ui_display: UIDisplayInfo {
                        collapsed: message,
                        expanded: Some(expanded),
                    },
                })
            }
            Err(e) => Err(ReadFileError {
                error_msg: e.clone(),
                ui_display: UIDisplayInfo {
                    collapsed: format!("{}: Read failed", params.path),
                    expanded: Some(format!(
                        "File: {}\nOperation: Read\nError: {}",
                        params.path, e
                    )),
                },
            }),
        }
    }

    pub fn edit_file(&mut self, params: &EditFileParams) -> Result<EditFileResult, EditFileError> {
        let edits_count = params.edits.len();

        // Get the diff before applying edits (for new files, this will generate an empty-to-content diff)
        let diff_before_edit = self
            .get_edit_diff(params)
            .unwrap_or_else(|_| String::from("Could not generate diff"));

        match self.apply_edits(&params.path, &params.edits) {
            Ok(_new_content) => {
                let edit_summary = if edits_count == 1 {
                    "1 edit"
                } else {
                    &format!("{edits_count} edits")
                };
                let collapsed = format!("{}: {} applied", params.path, edit_summary);

                let expanded = format!(
                    "File: {}\nOperation: Edit\nChanges: {} operations applied\n\nDiff:\n{}",
                    params.path, edits_count, diff_before_edit
                );

                // Include the diff in the message so LLMs can see exactly what changed
                let message = format!(
                    "Successfully edited {}\n\nDiff of changes:\n```diff\n{}\n```",
                    params.path, diff_before_edit
                );

                Ok(EditFileResult {
                    message,
                    ui_display: UIDisplayInfo {
                        collapsed,
                        expanded: Some(expanded),
                    },
                })
            }
            Err(e) => Err(EditFileError {
                error_msg: e.clone(),
                ui_display: UIDisplayInfo {
                    collapsed: format!("{}: Edit failed", params.path),
                    expanded: Some(format!(
                        "File: {}\nOperation: Edit\nError: {}",
                        params.path, e
                    )),
                },
            }),
        }
    }

    pub fn get_edit_diff(&self, params: &EditFileParams) -> Result<String, String> {
        let path_ref = Path::new(&params.path);
        let canonical_path = wasm_safe_normalize_path(path_ref, &self.working_directory, false)
            .map_err(|e| format!("Failed to normalize path '{}': {}", path_ref.display(), e))?;

        let is_new_file = !canonical_path.exists();
        let has_create_edit = params.edits.iter().any(|edit| edit.old_string.is_empty());

        // Apply same validation as edit_file
        if !is_new_file {
            // Existing file - must be in cache (must have been read)
            // canonical_path already computed above

            if !self.cache.contains_key(&canonical_path) {
                return Err(format!(
                    "File '{}' must be read before it can be edited. Please use the read_file tool first.",
                    params.path
                ));
            }
        } else if !has_create_edit {
            // New file but no create edit
            return Err(format!(
                "File '{}' does not exist. To create a new file, use an edit with empty old_string.",
                params.path
            ));
        }

        // Get the current file content
        let old_content = if canonical_path.exists() {
            fs::read_to_string(&canonical_path)
                .map_err(|e| format!("Failed to read file '{}': {}", params.path, e))?
        } else {
            String::new()
        };

        // Apply edits to get the new content
        let new_content = self.apply_edits_to_content(&old_content, &params.edits)?;

        // Generate the diff
        let text_diff = TextDiff::from_lines(&old_content, &new_content);
        let diff_output = text_diff
            .unified_diff()
            .context_radius(10)
            .header(&params.path, &format!("{} (modified)", params.path))
            .to_string();

        Ok(diff_output)
    }

    fn get_or_read_file_content<P: AsRef<Path>>(
        &mut self,
        path: P,
        start_line: Option<usize>,
        end_line: Option<usize>,
    ) -> Result<String, String> {
        let path_ref = path.as_ref();
        let canonical_path = wasm_safe_normalize_path(path_ref, &self.working_directory, true)
            .map_err(|e| format!("Failed to normalize path '{}': {}", path_ref.display(), e))?;

        let needs_read = match (start_line, end_line) {
            (None, None) => self.has_been_modified(&canonical_path)?,
            (Some(start), Some(end)) => {
                if self.has_been_modified(&canonical_path)? {
                    true
                } else if let Some(entry) = self.cache.get(&canonical_path) {
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

        let canonical_path = wasm_safe_normalize_path(path_ref, &self.working_directory, true)
            .map_err(|e| format!("Failed to normalize path '{}': {}", path_ref.display(), e))?;

        if let (Some(start), Some(end)) = (start_line, end_line) {
            if let Some(existing_entry) = self.cache.get(&canonical_path).cloned() {
                if !self.has_been_modified(path_ref)? {
                    if let FileContent::Partial { slices, .. } = &existing_entry.content {
                        let already_covered = slices
                            .iter()
                            .any(|slice| slice.start_line <= start && slice.end_line >= end);

                        if already_covered {
                            return Ok(());
                        }

                        let contents = fs::read_to_string(&canonical_path).map_err(|e| {
                            format!("Failed to read file '{}': {}", path_ref.display(), e)
                        })?;

                        let lines: Vec<&str> = contents.lines().collect();
                        let total_lines = lines.len();

                        if start < 1 || start > total_lines || end < start {
                            return Err(format!(
                                "Invalid line range: {start}-{end} (file has {total_lines} lines)"
                            ));
                        }

                        let slice_lines: Vec<String> = lines[(start - 1)..lines.len().min(end)]
                            .iter()
                            .map(|line| line.to_string())
                            .collect();

                        let new_slice = FileSlice {
                            start_line: start,
                            end_line: end.min(total_lines),
                            lines: slice_lines,
                        };

                        let mut updated_entry = existing_entry;
                        updated_entry.content.merge_slice(new_slice);
                        self.cache.insert(canonical_path, updated_entry);
                        return Ok(());
                    }
                }
            }
        }

        let metadata = fs::metadata(&canonical_path).map_err(|e| {
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

        if file_size > MAX_FILE_SIZE_BYTES {
            return Err(format!(
                "File '{}' is too large ({} bytes). Maximum file size is {} bytes.",
                path_ref.display(),
                file_size,
                MAX_FILE_SIZE_BYTES
            ));
        }

        let contents = fs::read_to_string(&canonical_path)
            .map_err(|e| format!("Failed to read file '{}': {}", path_ref.display(), e))?;

        let lines: Vec<&str> = contents.lines().collect();
        let total_lines = lines.len();

        let content = match (start_line, end_line) {
            (None, None) => {
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
                if start < 1 || start > total_lines || end < start {
                    return Err(format!(
                        "Invalid line range: {start}-{end} (file has {total_lines} lines)"
                    ));
                }

                let slice_lines: Vec<String> = lines[(start - 1)..lines.len().min(end)]
                    .iter()
                    .map(|line| line.to_string())
                    .collect();

                let slice = FileSlice {
                    start_line: start,
                    end_line: end.min(total_lines),
                    lines: slice_lines,
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

        let entry = FileCacheEntry {
            content,
            last_modified_at_read,
        };

        self.cache.insert(canonical_path, entry);

        Ok(())
    }

    fn has_been_modified<P: AsRef<Path>>(&self, path: P) -> Result<bool, String> {
        let path_ref = path.as_ref();

        let canonical_path = match wasm_safe_normalize_path(path_ref, &self.working_directory, true)
        {
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

    /// Refresh the cache for a file while preserving the original caching strategy.
    /// If the file was originally cached as Full, re-read the full file.
    /// If the file was originally cached as Partial, re-read the same line ranges.
    fn refresh_cache_preserving_strategy(&mut self, path: &Path) -> Result<(), String> {
        let canonical_path = wasm_safe_normalize_path(path, &self.working_directory, true)
            .map_err(|e| format!("Failed to normalize path '{}': {}", path.display(), e))?;

        if let Some(entry) = self.cache.get(&canonical_path).cloned() {
            match &entry.content {
                FileContent::Full(_) => {
                    // Re-read full file to maintain Full cache strategy
                    self.read_and_cache_file(path, None, None)?;
                }
                FileContent::Partial { slices, .. } => {
                    // Re-read the same line ranges that were originally cached
                    // We need to clear the cache entry first, then rebuild it with the same ranges
                    self.cache.remove(&canonical_path);

                    for slice in slices {
                        self.read_and_cache_file(
                            path,
                            Some(slice.start_line),
                            Some(slice.end_line),
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    fn apply_edits_to_content(&self, content: &str, edits: &[Edit]) -> Result<String, String> {
        let mut current_content = content.to_string();

        // Apply edits sequentially
        for edit in edits {
            current_content = self.apply_single_edit(&current_content, edit)?;
        }

        Ok(current_content)
    }

    fn apply_single_edit(&self, content: &str, edit: &Edit) -> Result<String, String> {
        let old_string = &edit.old_string;
        let new_string = &edit.new_string;

        // Handle file creation (empty old_string)
        if old_string.is_empty() {
            if !content.is_empty() {
                return Err("Cannot use empty old_string on non-empty file. Empty old_string is only for creating new files.".to_string());
            }
            return Ok(new_string.clone());
        }

        // Count occurrences of old_string
        let matches: Vec<_> = content.match_indices(old_string).collect();

        match matches.len() {
            0 => Err(format!(
                "String not found in file.\nLooking for: {}\n\nThe string must match exactly including all whitespace and newlines.",
                format_string_for_error(old_string)
            )),
            1 => {
                // Single match - replace it
                Ok(content.replacen(old_string, new_string, 1))
            }
            count => {
                if edit.replace_all {
                    // Replace all occurrences
                    Ok(content.replace(old_string, new_string))
                } else {
                    // Multiple matches without replace_all - error
                    Err(format!(
                        "Multiple matches found ({} occurrences) for string:\n{}\n\nEither set \"replace_all\": true or include more context to make the string unique.",
                        count,
                        format_string_for_error(old_string)
                    ))
                }
            }
        }
    }

    fn apply_edits(&mut self, path: &str, edits: &[Edit]) -> Result<String, String> {
        let path_ref = Path::new(path);
        let canonical_path = wasm_safe_normalize_path(path_ref, &self.working_directory, false)
            .map_err(|e| format!("Failed to normalize path '{}': {}", path_ref.display(), e))?;
        let is_new_file = !canonical_path.exists();

        // Check for new file creation pattern
        let has_create_edit = edits.iter().any(|edit| edit.old_string.is_empty());

        if is_new_file {
            if !has_create_edit {
                return Err(format!(
                    "File '{}' does not exist. To create a new file, use an edit with empty old_string.",
                    path
                ));
            }

            // For new files, use the already computed canonical path and create parent directories
            let absolute_path = canonical_path;

            if let Some(parent) = absolute_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directory: {e}"))?;
            }

            // Start with empty content for new file
            let new_content = self.apply_edits_to_content("", edits)?;

            fs::write(&absolute_path, &new_content)
                .map_err(|e| format!("Failed to write file: {e}"))?;

            // Cache the new file (full read for new files)
            let _ = self.read_and_cache_file(&absolute_path, None, None);

            Ok(new_content)
        } else {
            // Existing file - must be in cache (must have been read first)
            // canonical_path already computed above

            if let Some(entry) = self.cache.get(&canonical_path) {
                // Check if file has been modified since last read
                let metadata = fs::metadata(&canonical_path)
                    .map_err(|e| format!("Failed to read file metadata: {e}"))?;
                let current_mtime = metadata
                    .modified()
                    .map_err(|e| format!("Failed to get modified time: {e}"))?;

                if current_mtime != entry.last_modified_at_read {
                    return Err(format!(
                        "File '{}' has been modified since last read. Please use the read_file tool first.",
                        canonical_path.display()
                    ));
                }
            } else {
                return Err(format!(
                    "File '{path}' must be read before it can be edited. Please use the read_file tool first."
                ));
            }

            // Read current content and apply edits
            let content = fs::read_to_string(&canonical_path)
                .map_err(|e| format!("Failed to read file: {e}"))?;

            let new_content = self.apply_edits_to_content(&content, edits)?;

            fs::write(&canonical_path, &new_content)
                .map_err(|e| format!("Failed to write file: {e}"))?;

            // Update cache while preserving original caching strategy (Full vs Partial)
            let _ = self.refresh_cache_preserving_strategy(&canonical_path);

            Ok(new_content)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // Helper functions
    fn create_test_file(content: &str) -> (TempDir, std::path::PathBuf) {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, content).unwrap();
        (temp_dir, file_path)
    }

    fn create_test_manager(working_dir: &std::path::Path) -> FileInteractionManager {
        FileInteractionManager::new(working_dir.to_path_buf())
    }

    fn read_file_first(manager: &mut FileInteractionManager, path: &str) {
        let read_params = ReadFileParams {
            path: path.to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(read_params).unwrap();
    }

    // Basic Search-and-Replace Operations Tests

    #[test]
    fn test_single_string_replacement() {
        let (temp_dir, file_path) =
            create_test_file("def calculate(a, b):\n    return a + b\n\nprint(calculate(2, 3))");
        let mut manager = create_test_manager(temp_dir.path());

        read_file_first(&mut manager, &file_path.to_string_lossy());

        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                old_string: "a + b".to_string(),
                new_string: "a * b".to_string(),
                replace_all: false,
            }],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_ok());

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(
            content,
            "def calculate(a, b):\n    return a * b\n\nprint(calculate(2, 3))"
        );
    }

    #[test]
    fn test_multiline_replacement() {
        let (temp_dir, file_path) = create_test_file(
            "def old_function(x):\n    result = x * 2\n    return result\n\nprint(\"done\")",
        );
        let mut manager = create_test_manager(temp_dir.path());

        read_file_first(&mut manager, &file_path.to_string_lossy());

        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                old_string: "def old_function(x):\n    result = x * 2\n    return result".to_string(),
                new_string: "def new_function(x, y):\n    \"\"\"Calculate the product.\"\"\"\n    return x * y".to_string(),
                replace_all: false,
            }],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_ok());

        let content = fs::read_to_string(&file_path).unwrap();
        let expected = "def new_function(x, y):\n    \"\"\"Calculate the product.\"\"\"\n    return x * y\n\nprint(\"done\")";
        assert_eq!(content, expected);
    }

    #[test]
    fn test_replace_all_functionality() {
        let (temp_dir, file_path) =
            create_test_file("debug = True\nif debug: print('debug')\ndebug_mode = debug");
        let mut manager = create_test_manager(temp_dir.path());

        read_file_first(&mut manager, &file_path.to_string_lossy());

        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                old_string: "debug".to_string(),
                new_string: "verbose".to_string(),
                replace_all: true,
            }],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_ok());

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(
            content,
            "verbose = True\nif verbose: print('verbose')\nverbose_mode = verbose"
        );
    }

    #[test]
    fn test_create_new_file_with_empty_old_string() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = create_test_manager(temp_dir.path());
        let file_path = temp_dir.path().join("new_file.py");

        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                old_string: "".to_string(),
                new_string: "#!/usr/bin/env python3\n\ndef main():\n    print(\"Hello, World!\")\n\nif __name__ == \"__main__\":\n    main()".to_string(),
                replace_all: false,
            }],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_ok());

        assert!(file_path.exists());
        let content = fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("#!/usr/bin/env python3"));
        assert!(content.contains("def main():"));
        assert!(content.contains("print(\"Hello, World!\")"));
    }

    #[test]
    fn test_multiple_edits_sequential() {
        let (temp_dir, file_path) =
            create_test_file("DEBUG = True\nPORT = 3000\nHOST = 'localhost'\nUSE_SSL = False");
        let mut manager = create_test_manager(temp_dir.path());

        read_file_first(&mut manager, &file_path.to_string_lossy());

        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![
                Edit {
                    old_string: "DEBUG = True".to_string(),
                    new_string: "DEBUG = False".to_string(),
                    replace_all: false,
                },
                Edit {
                    old_string: "PORT = 3000".to_string(),
                    new_string: "PORT = 8080".to_string(),
                    replace_all: false,
                },
                Edit {
                    old_string: "USE_SSL = False".to_string(),
                    new_string: "USE_SSL = True".to_string(),
                    replace_all: false,
                },
            ],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_ok());

        let content = fs::read_to_string(&file_path).unwrap();
        let expected = "DEBUG = False\nPORT = 8080\nHOST = 'localhost'\nUSE_SSL = True";
        assert_eq!(content, expected);
    }

    #[test]
    fn test_string_not_found_error() {
        let (temp_dir, file_path) = create_test_file("line 1\nline 2\nline 3");
        let mut manager = create_test_manager(temp_dir.path());

        read_file_first(&mut manager, &file_path.to_string_lossy());

        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                old_string: "line 4".to_string(), // This doesn't exist
                new_string: "replacement".to_string(),
                replace_all: false,
            }],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().error_msg;
        assert!(error_msg.contains("String not found"));
        assert!(error_msg.contains("line 4"));
    }

    #[test]
    fn test_multiple_matches_without_replace_all() {
        let (temp_dir, file_path) = create_test_file(
            "result = calculate(x)\nprint(result)\nresult = calculate(y)\nprint(result)",
        );
        let mut manager = create_test_manager(temp_dir.path());

        read_file_first(&mut manager, &file_path.to_string_lossy());

        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                old_string: "result = calculate(".to_string(),
                new_string: "output = calculate(".to_string(),
                replace_all: false, // This should cause error due to multiple matches
            }],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().error_msg;
        assert!(error_msg.contains("Multiple matches found"));
        assert!(error_msg.contains("replace_all"));
    }

    #[test]
    fn test_diff_output_in_success_messages() {
        let (temp_dir, file_path) = create_test_file("Line 1\nLine 2\nLine 3\nLine 4\nLine 5");
        let mut manager = create_test_manager(temp_dir.path());

        read_file_first(&mut manager, &file_path.to_string_lossy());

        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                old_string: "Line 3".to_string(),
                new_string: "Modified Line 3".to_string(),
                replace_all: false,
            }],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_ok());

        let edit_result = result.unwrap();
        assert!(edit_result.message.contains("Diff of changes:"));
        assert!(edit_result.message.contains("-Line 3"));
        assert!(edit_result.message.contains("+Modified Line 3"));
        assert!(edit_result.message.contains("@@")); // Unified diff format marker
    }

    #[test]
    fn test_exact_whitespace_matching() {
        let (temp_dir, file_path) = create_test_file(
            "    def function1():\n\t\tprint('spaces before')\n\tdef function2():\n\t\tprint('tabs before')",
        );
        let mut manager = create_test_manager(temp_dir.path());

        read_file_first(&mut manager, &file_path.to_string_lossy());

        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                old_string: "    def function1():\n\t\tprint('spaces before')".to_string(), // Exact whitespace
                new_string: "    def modified_function1():\n\t\tprint('modified spaces')"
                    .to_string(),
                replace_all: false,
            }],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_ok());

        let content = fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("def modified_function1():"));
        assert!(content.contains("print('modified spaces')"));
        assert!(content.contains("def function2():")); // function2 should be unchanged
    }

    #[test]
    fn test_context_for_uniqueness() {
        let (temp_dir, file_path) = create_test_file(
            "def process_user_data(data):\n    # TODO: Add validation\n    return data\n\ndef process_admin_data(data):\n    # TODO: Add validation\n    return data + \"_admin\"",
        );
        let mut manager = create_test_manager(temp_dir.path());

        read_file_first(&mut manager, &file_path.to_string_lossy());

        // Replace only the first TODO by including surrounding context
        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                old_string: "def process_user_data(data):\n    # TODO: Add validation\n    return data".to_string(),
                new_string: "def process_user_data(data):\n    if not data or not isinstance(data, dict):\n        raise ValueError(\"Invalid user data\")\n    return data".to_string(),
                replace_all: false,
            }],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_ok());

        let content = fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("if not data or not isinstance(data, dict):"));
        assert!(content.contains("raise ValueError(\"Invalid user data\")"));
        assert!(content.contains("def process_admin_data(data):\n    # TODO: Add validation")); // This TODO should remain
    }

    #[test]
    fn test_partial_cache_preservation_after_edit() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = create_test_manager(temp_dir.path());

        // Create a file with many lines
        let content = (1..=20)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let (_temp_dir, file_path) = create_test_file(&content);

        // Read only lines 5-8 to create a partial cache
        let read_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(5),
            end_line: Some(8),
        };
        manager.read_file(read_params).unwrap();

        // Verify we have a partial cache
        let canonical_path =
            wasm_safe_normalize_path(&file_path, &manager.working_directory, true).unwrap();
        let entry_before = manager.cache.get(&canonical_path).unwrap();
        assert!(matches!(entry_before.content, FileContent::Partial { .. }));

        // Edit the file - change "line 6" to "modified line 6"
        let edit_params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                old_string: "line 6".to_string(),
                new_string: "modified line 6".to_string(),
                replace_all: false,
            }],
        };

        let result = manager.edit_file(&edit_params);
        assert!(result.is_ok());

        // Verify cache is still partial and contains updated content
        let entry_after = manager.cache.get(&canonical_path).unwrap();
        match &entry_after.content {
            FileContent::Partial {
                slices,
                total_lines,
            } => {
                assert_eq!(*total_lines, 20);
                assert_eq!(slices.len(), 1);
                assert_eq!(slices[0].start_line, 5);
                assert_eq!(slices[0].end_line, 8);

                // Verify the content was updated
                let numbered_content = entry_after.content.get_numbered_content();
                assert!(numbered_content.contains("5:line 5"));
                assert!(numbered_content.contains("6:modified line 6")); // This should be updated
                assert!(numbered_content.contains("7:line 7"));
                assert!(numbered_content.contains("8:line 8"));

                // Should not contain other lines
                assert!(!numbered_content.contains("1:line 1"));
                assert!(!numbered_content.contains("10:line 10"));
            }
            FileContent::Full(_) => {
                panic!("Cache should still be Partial after edit, not Full");
            }
        }
    }

    #[test]
    fn test_full_cache_preservation_after_edit() {
        let (temp_dir, file_path) = create_test_file("line 1\nline 2\nline 3\nline 4\nline 5");
        let mut manager = create_test_manager(temp_dir.path());

        // Read full file to create full cache
        read_file_first(&mut manager, &file_path.to_string_lossy());

        // Verify we have a full cache
        let canonical_path =
            wasm_safe_normalize_path(&file_path, &manager.working_directory, true).unwrap();
        let entry_before = manager.cache.get(&canonical_path).unwrap();
        assert!(matches!(entry_before.content, FileContent::Full(_)));

        // Edit the file
        let edit_params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                old_string: "line 3".to_string(),
                new_string: "modified line 3".to_string(),
                replace_all: false,
            }],
        };

        let result = manager.edit_file(&edit_params);
        assert!(result.is_ok());

        // Verify cache is still full and contains updated content
        let entry_after = manager.cache.get(&canonical_path).unwrap();
        match &entry_after.content {
            FileContent::Full(content) => {
                assert!(content.contains("line 1"));
                assert!(content.contains("line 2"));
                assert!(content.contains("modified line 3")); // Updated
                assert!(content.contains("line 4"));
                assert!(content.contains("line 5"));
            }
            FileContent::Partial { .. } => {
                panic!("Cache should still be Full after edit, not Partial");
            }
        }
    }

    #[test]
    fn test_context_preservation_workflow() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = create_test_manager(temp_dir.path());

        // Create a large file (simulating a file where partial reads matter)
        let content = (1..=100)
            .map(|i| format!("function_{}() {{ return {}; }}", i, i * 10))
            .collect::<Vec<_>>()
            .join("\n");
        let (_temp_dir, file_path) = create_test_file(&content);

        // LLM reads only lines 40-50 for focused work
        let read_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(40),
            end_line: Some(50),
        };
        let read_result = manager.read_file(read_params).unwrap();

        // Verify partial context is provided to LLM
        assert!(read_result.message.contains("40-50"));

        // LLM edits function_45 based on the context it sees
        let edit_params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                old_string: "function_45() { return 450; }".to_string(),
                new_string: "function_45_optimized() { return 450 * 2; }".to_string(),
                replace_all: false,
            }],
        };

        let edit_result = manager.edit_file(&edit_params);
        assert!(edit_result.is_ok());

        // Verify edit was successful
        assert!(edit_result.unwrap().message.contains("Successfully edited"));

        // Key test: Verify LLM still sees only the same context range
        let files_info = manager.get_files_info();
        assert_eq!(files_info.len(), 1);

        let (_, cached_content) = &files_info[0];

        // Should still only see lines 40-50, not the entire 100-line file
        assert!(cached_content.contains("40:function_40()"));
        assert!(cached_content.contains("45:function_45_optimized()")); // Updated content
        assert!(cached_content.contains("50:function_50()"));

        // Should NOT see other lines (context preserved)
        assert!(!cached_content.contains("1:function_1()"));
        assert!(!cached_content.contains("90:function_90()"));
        assert!(cached_content.contains("[... 39 lines omitted ...]")); // Before
        assert!(cached_content.contains("[... 50 lines omitted ...]")); // After

        // Verify actual file on disk has the change
        let disk_content = fs::read_to_string(&file_path).unwrap();
        assert!(disk_content.contains("function_45_optimized() { return 450 * 2; }"));
        assert!(disk_content.contains("function_1() { return 10; }")); // Other lines intact
        assert!(disk_content.contains("function_100() { return 1000; }")); // Other lines intact
    }

    #[test]
    fn test_error_read_before_edit() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = create_test_manager(temp_dir.path());
        let content = "line 1\nline 2\nline 3";
        let (_temp_dir, file_path) = create_test_file(content);

        // Try to edit without reading first
        let edit_params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                old_string: "line 2".to_string(),
                new_string: "modified line 2".to_string(),
                replace_all: false,
            }],
        };

        let result = manager.edit_file(&edit_params);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .error_msg
                .contains("must be read before it can be edited")
        );
    }

    #[test]
    fn test_error_file_staleness() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = create_test_manager(temp_dir.path());
        let content = "line 1\nline 2\nline 3";
        let (_temp_dir, file_path) = create_test_file(content);

        // Read the file first
        let read_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(read_params).unwrap();

        // Modify the file externally to make it stale
        fs::write(&file_path, "externally modified content").unwrap();

        // Try to edit the now-stale file
        let edit_params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                old_string: "line 2".to_string(),
                new_string: "modified line 2".to_string(),
                replace_all: false,
            }],
        };

        let result = manager.edit_file(&edit_params);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .error_msg
                .contains("has been modified since last read")
        );
    }

    #[test]
    fn test_error_nonexistent_file() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = create_test_manager(temp_dir.path());
        let temp_dir = TempDir::new().unwrap();
        let nonexistent_path = temp_dir.path().join("does_not_exist.txt");

        // Try to edit a file that doesn't exist
        let edit_params = EditFileParams {
            path: nonexistent_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                old_string: "some text".to_string(),
                new_string: "replacement".to_string(),
                replace_all: false,
            }],
        };

        let result = manager.edit_file(&edit_params);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().error_msg.contains(
                "does not exist. To create a new file, use an edit with empty old_string"
            )
        );
    }

    #[test]
    fn test_error_invalid_empty_string_usage() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = create_test_manager(temp_dir.path());
        let content = "existing content";
        let (_temp_dir, file_path) = create_test_file(content);

        // Read the file first
        let read_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(read_params).unwrap();

        // Try to use empty old_string on a non-empty file
        let edit_params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                old_string: "".to_string(),
                new_string: "new content".to_string(),
                replace_all: false,
            }],
        };

        let result = manager.edit_file(&edit_params);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .error_msg
                .contains("Cannot use empty old_string on non-empty file")
        );
    }

    #[test]
    fn test_relative_paths_work_correctly() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = create_test_manager(temp_dir.path());

        // Create a file with a relative path
        let relative_path = "test_relative.txt";
        let edit_params = EditFileParams {
            path: relative_path.to_string(),
            edits: vec![Edit {
                old_string: "".to_string(),
                new_string: "This file was created with a relative path!".to_string(),
                replace_all: false,
            }],
        };

        // Create the file using relative path
        let result = manager.edit_file(&edit_params);
        assert!(
            result.is_ok(),
            "Failed to create file with relative path: {:?}",
            result
        );

        // Verify the file exists at the expected absolute location
        let absolute_path = temp_dir.path().join(relative_path);
        assert!(absolute_path.exists(), "File should exist at absolute path");

        let content = fs::read_to_string(&absolute_path).unwrap();
        assert_eq!(content, "This file was created with a relative path!");

        // Now read the file using the same relative path
        let read_params = ReadFileParams {
            path: relative_path.to_string(),
            start_line: None,
            end_line: None,
        };

        let read_result = manager.read_file(read_params);
        assert!(
            read_result.is_ok(),
            "Failed to read file with relative path: {:?}",
            read_result
        );

        // Edit the file using relative path
        let edit_params2 = EditFileParams {
            path: relative_path.to_string(),
            edits: vec![Edit {
                old_string: "This file was created with a relative path!".to_string(),
                new_string: "This file was edited using relative paths!".to_string(),
                replace_all: false,
            }],
        };

        let edit_result = manager.edit_file(&edit_params2);
        assert!(
            edit_result.is_ok(),
            "Failed to edit file with relative path: {:?}",
            edit_result
        );

        // Verify the edit worked
        let final_content = fs::read_to_string(&absolute_path).unwrap();
        assert_eq!(final_content, "This file was edited using relative paths!");
    }
}
