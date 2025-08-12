use std::{
    collections::HashMap,
    fs, io,
    path::{Component as PathComponent, Path, PathBuf},
    time::SystemTime,
};

use wasmind_actor_utils::common_messages::tools::UIDisplayInfo;
use serde::{Deserialize, Serialize};
use similar::TextDiff;

// Tool constants
pub const READ_FILE_NAME: &str = "read_file";
pub const READ_FILE_DESCRIPTION: &str = "Reads content from a file. For small files (<64KB), it reads the entire file. For large files, it returns an error with metadata, requiring you to specify a line range. All returned file content is prefixed with line numbers in the format LINE_NUMBER:CONTENT. You can read a specific chunk by providing start_line and end_line. IMPORTANT: Always use absolute paths (starting with /) - relative paths will fail.";
pub const READ_FILE_SCHEMA: &str = r#"{
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

pub const EDIT_FILE_NAME: &str = "edit_file";
pub const EDIT_FILE_DESCRIPTION: &str = "Applies a list of edits to a file atomically. This is the primary tool for modifying files AND creating new files. Each edit targets a specific line range. The tool processes edits from the bottom of the file to the top to ensure line number integrity during the operation. To create a new file, use a single edit with start_line=1 and end_line=0. IMPORTANT: Always use absolute paths (starting with /) - relative paths will fail.";
pub const EDIT_FILE_SCHEMA: &str = r#"{
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

pub const FILE_TOOLS_USAGE_GUIDE: &str = r#"## File Interaction Tools

The file interaction actor provides two essential tools for reading and editing files:

**CRITICAL: Always use ABSOLUTE paths (starting with /) - relative paths will fail!**

### read_file
- Reads file content with automatic line numbering (format: LINE_NUMBER:CONTENT)
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileSlice {
    start_line: usize, // 1-indexed
    end_line: usize,   // 1-indexed, inclusive
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
    start_line: usize, // 1-indexed
    end_line: usize,   // 1-indexed, inclusive
    new_content: String,
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
}

impl FileInteractionManager {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
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
                let message = match (start_line, end_line) {
                    (Some(start), Some(end)) => {
                        format!("Read file: {} (lines {}-{})", params.path, start, end)
                    }
                    _ => format!("Read file: {}", params.path),
                };

                let line_count = content.lines().count();
                let collapsed = match (start_line, end_line) {
                    (Some(start), Some(end)) => {
                        format!(
                            "{}: {} lines ({}–{})",
                            params.path,
                            end - start + 1,
                            start,
                            end
                        )
                    }
                    _ => {
                        format!("{}: {} lines", params.path, line_count)
                    }
                };

                let expanded = format!("File: {}\n\n{}", params.path, content);

                Ok(ReadFileResult {
                    message,
                    ui_display: UIDisplayInfo {
                        collapsed,
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
        match self.apply_edits(&params.path, &params.edits) {
            Ok(_new_content) => {
                let edit_summary = if edits_count == 1 {
                    "1 edit"
                } else {
                    &format!("{edits_count} edits")
                };
                let collapsed = format!("{}: {} applied", params.path, edit_summary);

                let expanded = format!(
                    "File: {}\nOperation: Edit\nChanges: {} operations applied",
                    params.path, edits_count
                );

                Ok(EditFileResult {
                    message: format!("Edited: {}", params.path),
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

        if !path_ref.is_absolute() {
            return Err(format!(
                "Relative paths are not supported. Please use an absolute path starting with '/'. Got: '{}'",
                params.path
            ));
        }

        let is_new_file = !path_ref.exists();
        let is_single_create_edit = params.edits.len() == 1 
            && params.edits[0].start_line == 1 
            && params.edits[0].end_line == 0;

        // Apply same validation as edit_file
        if !is_new_file {
            // Existing file - must be in cache (must have been read)
            let canonical_path = wasm_safe_normalize_path(path_ref)
                .map_err(|e| format!("Failed to normalize path '{}': {}", path_ref.display(), e))?;
            
            if !self.cache.contains_key(&canonical_path) {
                return Err(format!(
                    "File '{}' must be read before it can be edited. Please use the read_file tool first.",
                    params.path
                ));
            }
        } else if !is_single_create_edit {
            // New file but not a single create edit
            return Err(format!(
                "Cannot apply multiple edits to a new file. For new files, use a single edit operation with start_line=1 and end_line=0. Found {} edits.",
                params.edits.len()
            ));
        }

        // Get the current file content
        let old_content = if path_ref.exists() {
            fs::read_to_string(path_ref)
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
        let canonical_path = wasm_safe_normalize_path(path_ref)
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

        let canonical_path = wasm_safe_normalize_path(path_ref)
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

                        let contents = fs::read_to_string(path_ref).map_err(|e| {
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

    fn apply_edits_to_content(&self, content: &str, edits: &[Edit]) -> Result<String, String> {
        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        let is_empty_file = lines.is_empty();

        if is_empty_file && edits.len() > 1 {
            return Err(format!(
                "Cannot apply multiple edits to a new/empty file. For new files, use a single edit operation to create the initial content. Found {} edits.",
                edits.len()
            ));
        }

        let mut edits_vec: Vec<Edit> = edits.to_vec();
        edits_vec.sort_by(|a, b| b.start_line.cmp(&a.start_line));

        for edit in edits_vec {
            let current_total_lines = lines.len();
            // Validate line numbers based on current line count
            if edit.start_line < 1 {
                return Err(format!(
                    "start_line must be at least 1 (got {})",
                    edit.start_line
                ));
            }
            if edit.start_line > current_total_lines + 1 {
                return Err(format!(
                    "start_line cannot be greater than {} for a file with {} lines (got {})",
                    current_total_lines + 1, current_total_lines, edit.start_line
                ));
            }

            if edit.end_line == edit.start_line - 1 {
                let new_lines: Vec<String> =
                    edit.new_content.lines().map(|s| s.to_string()).collect();
                let insert_pos = edit.start_line - 1;
                for (i, line) in new_lines.into_iter().enumerate() {
                    lines.insert(insert_pos + i, line);
                }
            } else {
                if edit.end_line < edit.start_line {
                    return Err(format!(
                        "end_line ({}) must be greater than or equal to start_line ({})",
                        edit.end_line, edit.start_line
                    ));
                }
                if edit.end_line > current_total_lines {
                    return Err(format!(
                        "end_line cannot be greater than {} for a file with {} lines (got {})",
                        current_total_lines, current_total_lines, edit.end_line
                    ));
                }

                for _ in edit.start_line..=edit.end_line {
                    lines.remove(edit.start_line - 1);
                }

                if !edit.new_content.is_empty() {
                    let new_lines: Vec<String> =
                        edit.new_content.lines().map(|s| s.to_string()).collect();
                    for (i, line) in new_lines.into_iter().enumerate() {
                        lines.insert(edit.start_line - 1 + i, line);
                    }
                }
            }
        }

        let new_content = if lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", lines.join("\n"))
        };

        Ok(new_content)
    }

    fn apply_edits(&mut self, path: &str, edits: &[Edit]) -> Result<String, String> {
        let path_ref = Path::new(path);

        if !path_ref.is_absolute() {
            return Err(format!(
                "Relative paths are not supported. Please use an absolute path starting with '/'. Got: '{path}'"
            ));
        }

        let is_new_file = !path_ref.exists();
        let is_single_create_edit = edits.len() == 1 
            && edits[0].start_line == 1 
            && edits[0].end_line == 0;

        if !is_new_file {
            // Existing file - must be in cache (must have been read)
            let canonical_path = wasm_safe_normalize_path(path_ref)
                .map_err(|e| format!("Failed to normalize path '{}': {}", path_ref.display(), e))?;
            
            if let Some(entry) = self.cache.get(&canonical_path) {
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
        } else if !is_single_create_edit {
            // New file but not a single create edit
            return Err(format!(
                "Cannot apply multiple edits to a new file. For new files, use a single edit operation with start_line=1 and end_line=0. Found {} edits.",
                edits.len()
            ));
        } else {
            // Creating new file
            if let Some(parent) = path_ref.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directory: {e}"))?;
            }
            fs::File::create(path).map_err(|e| format!("Failed to create file: {e}"))?;
        }

        let canonical_path = wasm_safe_normalize_path(path_ref)
            .map_err(|e| format!("Failed to normalize path '{}': {}", path_ref.display(), e))?;

        let content = fs::read_to_string(&canonical_path)
            .map_err(|e| format!("Failed to read file: {e}"))?;

        let new_content = self.apply_edits_to_content(&content, edits)?;

        fs::write(&canonical_path, &new_content)
            .map_err(|e| format!("Failed to write file: {e}"))?;

        let _ = self.read_and_cache_file(&canonical_path, None, None);

        Ok(new_content)
    }
}

impl Default for FileInteractionManager {
    fn default() -> Self {
        Self::new()
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
            lines: vec!["line 5".to_string(), "line 6".to_string(), "line 7".to_string()],
        };

        assert_eq!(slice.start_line, 5);
        assert_eq!(slice.end_line, 7);
        assert_eq!(slice.lines.len(), 3);
        assert_eq!(slice.lines[0], "line 5");
    }

    #[test]
    fn test_file_content_full_get_numbered_content() {
        let content = FileContent::Full("line 1\nline 2\nline 3".to_string());
        let numbered = content.get_numbered_content();

        assert_eq!(numbered, "1:line 1\n2:line 2\n3:line 3");
    }

    #[test]
    fn test_file_content_partial_get_numbered_content() {
        let slice1 = FileSlice {
            start_line: 1,
            end_line: 2,
            lines: vec!["line 1".to_string(), "line 2".to_string()],
        };
        let slice2 = FileSlice {
            start_line: 5,
            end_line: 6,
            lines: vec!["line 5".to_string(), "line 6".to_string()],
        };

        let content = FileContent::Partial {
            slices: vec![slice1, slice2],
            total_lines: 10,
        };

        let numbered = content.get_numbered_content();
        let expected = "1:line 1\n2:line 2\n[... 2 lines omitted ...]\n5:line 5\n6:line 6\n[... 4 lines omitted ...]";
        assert_eq!(numbered, expected);
    }

    #[test]
    fn test_manager_read_file() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create test file
        let content = "line 1\nline 2\nline 3";
        std::fs::write(&file_path, content).unwrap();

        let params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };

        let result = manager.read_file(params);
        assert!(result.is_ok());

        let read_result = result.unwrap();
        assert!(read_result.message.contains("Read file:"));
        assert!(read_result.ui_display.expanded.is_some());
    }

    #[test]
    fn test_manager_edit_file() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("edit_test.txt");

        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 1,
                end_line: 0,
                new_content: "Hello, World!".to_string(),
            }],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_ok());

        let edit_result = result.unwrap();
        assert!(edit_result.message.contains("Edited:"));

        // Verify file was created
        assert!(file_path.exists());
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Hello, World!\n");
    }

    #[test]
    fn test_file_content_merge_slice_non_overlapping() {
        let slice1 = FileSlice {
            start_line: 1,
            end_line: 2,
            lines: vec!["line 1".to_string(), "line 2".to_string()],
        };

        let mut content = FileContent::Partial {
            slices: vec![slice1],
            total_lines: 10,
        };

        let new_slice = FileSlice {
            start_line: 5,
            end_line: 6,
            lines: vec!["line 5".to_string(), "line 6".to_string()],
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
            lines: vec!["line 1".to_string(), "line 2".to_string(), "line 3".to_string()],
        };

        let mut content = FileContent::Partial {
            slices: vec![slice1],
            total_lines: 10,
        };

        // Overlapping slice
        let new_slice = FileSlice {
            start_line: 3,
            end_line: 5,
            lines: vec!["line 3".to_string(), "line 4".to_string(), "line 5".to_string()],
        };

        content.merge_slice(new_slice);

        if let FileContent::Partial { slices, .. } = content {
            assert_eq!(slices.len(), 1);
            assert_eq!(slices[0].start_line, 1);
            assert_eq!(slices[0].end_line, 5);
            assert_eq!(slices[0].lines.len(), 5);
            assert_eq!(slices[0].lines[4], "line 5");
        } else {
            panic!("Expected partial content");
        }
    }

    #[test]
    fn test_file_reader_slice_merging() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create a file with several lines
        let content =
            "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10";
        fs::write(&file_path, content).unwrap();

        // Read first slice
        manager
            .read_and_cache_file(&file_path, Some(1), Some(3))
            .unwrap();

        // Read overlapping slice - should merge
        let result = manager.get_or_read_file_content(&file_path, Some(3), Some(6));
        assert!(result.is_ok());

        // Check that slices were merged
        let cached = manager
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
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create a file with several lines
        let content = "line 1\nline 2\nline 3\nline 4\nline 5";
        fs::write(&file_path, content).unwrap();

        // Read first slice
        manager
            .read_and_cache_file(&file_path, Some(1), Some(5))
            .unwrap();

        // Try to read a subset - should be already covered
        let result = manager.read_and_cache_file(&file_path, Some(2), Some(4));
        assert!(result.is_ok());

        // Verify we still have just one slice
        let cached = manager
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
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create initial file
        let content = "line 1\nline 2\nline 3\nline 4\nline 5";
        fs::write(&file_path, content).unwrap();

        // Read first slice
        manager
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
        let result = manager.read_and_cache_file(&file_path, Some(4), Some(5));
        assert!(result.is_ok());

        // Should have fresh cache entry for lines 4-5, not merged
        let cached = manager
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

    #[test]
    fn test_edit_parse() {
        let params = EditFileParams {
            path: "/tmp/test.txt".to_string(),
            edits: vec![Edit {
                start_line: 1,
                end_line: 1,
                new_content: "Hello, world!".to_string(),
            }],
        };

        // Test that Edit structs work correctly
        assert_eq!(params.edits.len(), 1);
        assert_eq!(params.edits[0].start_line, 1);
        assert_eq!(params.edits[0].end_line, 1);
        assert_eq!(params.edits[0].new_content, "Hello, world!");
    }

    // --- Comprehensive Edit Operation Tests ---

    #[test]
    fn test_edit_create_new_file() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("new_file.txt");

        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 1,
                end_line: 0,
                new_content: "Hello, World!\nThis is a new file.".to_string(),
            }],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_ok());

        // Verify file was created and has correct content
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Hello, World!\nThis is a new file.\n");
    }

    #[test]
    fn test_edit_create_new_file_with_multiple_operations_error() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("multi_edit_new.txt");

        // Try to create a new file with multiple edit operations - should fail
        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![
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
            ],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .error_msg
                .contains("Cannot apply multiple edits to a new file")
        );
    }

    #[test]
    fn test_edit_create_then_edit_workflow() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("workflow.txt");

        // Step 1: Create file with initial content
        let create_params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 1,
                end_line: 0,
                new_content: "Line 1\nLine 2\nLine 3".to_string(),
            }],
        };

        let result = manager.edit_file(&create_params);
        assert!(result.is_ok());

        // Verify cache was populated after creation
        let canonical_path = wasm_safe_normalize_path(&file_path).unwrap();
        assert!(manager.cache.contains_key(&canonical_path));
        let cached_entry = manager.cache.get(&canonical_path).unwrap();
        let cached_content = cached_entry.content.get_numbered_content();
        assert!(cached_content.contains("1:Line 1"));
        assert!(cached_content.contains("2:Line 2"));
        assert!(cached_content.contains("3:Line 3"));

        // Step 2: Now we can edit the existing file
        let modify_params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 2,
                end_line: 2,
                new_content: "Modified Line 2".to_string(),
            }],
        };

        let result = manager.edit_file(&modify_params);
        assert!(result.is_ok());

        // Verify cache was updated after edit
        let cached_entry_after_edit = manager.cache.get(&canonical_path).unwrap();
        let cached_content_after = cached_entry_after_edit.content.get_numbered_content();
        assert!(cached_content_after.contains("1:Line 1"));
        assert!(cached_content_after.contains("2:Modified Line 2"));
        assert!(cached_content_after.contains("3:Line 3"));

        // Verify the final content on disk matches cache
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Line 1\nModified Line 2\nLine 3\n");
    }

    #[test]
    fn test_edit_empty_file() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("empty.txt");

        // Create an empty file
        fs::File::create(&file_path).unwrap();

        // Read the empty file first
        let read_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(read_params).unwrap();

        // Add content to empty file
        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 1,
                end_line: 0,
                new_content: "First line\nSecond line".to_string(),
            }],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_ok());

        // Verify content
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "First line\nSecond line\n");
    }

    #[test]
    fn test_edit_single_line_operations() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("single_ops.txt");

        // Create initial file
        let initial_content = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5";
        fs::write(&file_path, initial_content).unwrap();

        // Read the file first
        let read_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(read_params).unwrap();

        // Verify initial cache state
        let canonical_path = wasm_safe_normalize_path(&file_path).unwrap();
        let initial_cached = manager.cache.get(&canonical_path).unwrap();
        let initial_content_cached = initial_cached.content.get_numbered_content();
        assert!(initial_content_cached.contains("3:Line 3"));

        // Test replace operation
        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 3,
                end_line: 3,
                new_content: "Modified Line 3".to_string(),
            }],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_ok());

        // Verify cache was updated with the edit
        let cached_after_edit = manager.cache.get(&canonical_path).unwrap();
        let cached_content = cached_after_edit.content.get_numbered_content();
        assert!(cached_content.contains("1:Line 1"));
        assert!(cached_content.contains("2:Line 2"));
        assert!(cached_content.contains("3:Modified Line 3"));
        assert!(cached_content.contains("4:Line 4"));
        assert!(cached_content.contains("5:Line 5"));

        // Verify disk content matches cache
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Line 1\nLine 2\nModified Line 3\nLine 4\nLine 5\n");
    }

    #[test]
    fn test_edit_insert_lines() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("insert_test.txt");

        // Create initial file
        let initial_content = "Line 1\nLine 2\nLine 3";
        fs::write(&file_path, initial_content).unwrap();

        // Read the file first
        let read_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(read_params).unwrap();

        // Verify initial cache
        let canonical_path = wasm_safe_normalize_path(&file_path).unwrap();
        let initial_cached = manager.cache.get(&canonical_path).unwrap();
        let initial_content_cached = initial_cached.content.get_numbered_content();
        assert!(initial_content_cached.contains("1:Line 1"));
        assert!(initial_content_cached.contains("2:Line 2"));
        assert!(initial_content_cached.contains("3:Line 3"));

        // Insert new lines between line 2 and 3
        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 3,
                end_line: 2, // end_line < start_line indicates insertion
                new_content: "Inserted Line A\nInserted Line B".to_string(),
            }],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_ok());

        // Verify cache was updated with inserted lines
        let cached_after_edit = manager.cache.get(&canonical_path).unwrap();
        let cached_content = cached_after_edit.content.get_numbered_content();
        assert!(cached_content.contains("1:Line 1"));
        assert!(cached_content.contains("2:Line 2"));
        assert!(cached_content.contains("3:Inserted Line A"));
        assert!(cached_content.contains("4:Inserted Line B"));
        assert!(cached_content.contains("5:Line 3"));

        // Verify disk content matches cache
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(
            content,
            "Line 1\nLine 2\nInserted Line A\nInserted Line B\nLine 3\n"
        );
    }

    #[test]
    fn test_edit_delete_lines() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("delete_test.txt");

        // Create initial file
        let initial_content = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5";
        fs::write(&file_path, initial_content).unwrap();

        // Read the file first
        let read_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(read_params).unwrap();

        // Verify initial cache has all 5 lines
        let canonical_path = wasm_safe_normalize_path(&file_path).unwrap();
        let initial_cached = manager.cache.get(&canonical_path).unwrap();
        let initial_content_cached = initial_cached.content.get_numbered_content();
        assert!(initial_content_cached.contains("2:Line 2"));
        assert!(initial_content_cached.contains("3:Line 3"));
        assert!(initial_content_cached.contains("4:Line 4"));

        // Delete lines 2-4
        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 2,
                end_line: 4,
                new_content: "".to_string(),
            }],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_ok());

        // Verify cache was updated with deletion
        let cached_after_edit = manager.cache.get(&canonical_path).unwrap();
        let cached_content = cached_after_edit.content.get_numbered_content();
        assert!(cached_content.contains("1:Line 1"));
        assert!(cached_content.contains("2:Line 5"));
        // Verify deleted lines are no longer in cache
        assert!(!cached_content.contains("Line 2"));
        assert!(!cached_content.contains("Line 3"));
        assert!(!cached_content.contains("Line 4"));

        // Verify disk content matches cache
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Line 1\nLine 5\n");
    }

    #[test]
    fn test_edit_multiple_operations_sorted() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("multi_ops.txt");

        // Create initial file
        let initial_content = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5";
        fs::write(&file_path, initial_content).unwrap();

        // Read the file first
        let read_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(read_params).unwrap();

        // Verify initial cache state
        let canonical_path = wasm_safe_normalize_path(&file_path).unwrap();
        let initial_cached = manager.cache.get(&canonical_path).unwrap();
        let initial_content_cached = initial_cached.content.get_numbered_content();
        assert!(initial_content_cached.contains("2:Line 2"));
        assert!(initial_content_cached.contains("4:Line 4"));
        assert!(initial_content_cached.contains("5:Line 5"));

        // Multiple edits - should be processed in reverse order
        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![
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
            ],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_ok());

        // Verify cache was updated with all edits
        let cached_after_edit = manager.cache.get(&canonical_path).unwrap();
        let cached_content = cached_after_edit.content.get_numbered_content();
        assert!(cached_content.contains("1:Line 1"));
        assert!(cached_content.contains("2:Modified Line 2"));
        assert!(cached_content.contains("3:Line 3"));
        assert!(cached_content.contains("4:Modified Line 4"));
        assert!(cached_content.contains("5:Line 5"));
        assert!(cached_content.contains("6:New Line 6"));

        // Verify disk content matches cache
        let content = fs::read_to_string(&file_path).unwrap();
        let expected = "Line 1\nModified Line 2\nLine 3\nModified Line 4\nLine 5\nNew Line 6\n";
        assert_eq!(content, expected);
    }


    #[test]
    fn test_improved_error_messages() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("error_test.txt");

        // Create initial file with 3 lines
        let initial_content = "Line 1\nLine 2\nLine 3";
        fs::write(&file_path, initial_content).unwrap();

        // Read the file first
        let read_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(read_params).unwrap();

        // Test start_line < 1
        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 0,
                end_line: 1,
                new_content: "Test".to_string(),
            }],
        };
        let result = manager.edit_file(&params);
        assert!(result.is_err());
        assert!(result.unwrap_err().error_msg.contains("start_line must be at least 1 (got 0)"));

        // Test start_line > total_lines + 1
        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 5, // File has 3 lines, so max valid is 4
                end_line: 5,
                new_content: "Test".to_string(),
            }],
        };
        let result = manager.edit_file(&params);
        assert!(result.is_err());
        assert!(result.unwrap_err().error_msg.contains("start_line cannot be greater than 4 for a file with 3 lines (got 5)"));

        // Test end_line < start_line
        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 3,
                end_line: 1,
                new_content: "Test".to_string(),
            }],
        };
        let result = manager.edit_file(&params);
        assert!(result.is_err());
        assert!(result.unwrap_err().error_msg.contains("end_line (1) must be greater than or equal to start_line (3)"));

        // Test end_line > total_lines
        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 2,
                end_line: 5, // File has 3 lines
                new_content: "Test".to_string(),
            }],
        };
        let result = manager.edit_file(&params);
        assert!(result.is_err());
        assert!(result.unwrap_err().error_msg.contains("end_line cannot be greater than 3 for a file with 3 lines (got 5)"));
    }

    #[test]
    fn test_edit_file_without_read_first() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("no_read_first.txt");

        // Create initial file
        let initial_content = "Original content";
        fs::write(&file_path, initial_content).unwrap();

        // Edit file without reading it first - should fail now
        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 1,
                end_line: 1,
                new_content: "Modified content".to_string(),
            }],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_err());
        assert!(result.unwrap_err().error_msg.contains("must be read before it can be edited"));

        // Now read the file first
        let read_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(read_params).unwrap();

        // Now edit should work
        let result = manager.edit_file(&params);
        assert!(result.is_ok());

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Modified content\n");
    }

    #[test]
    fn test_edit_file_staleness_check() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("staleness.txt");

        // Create and read file into cache
        let initial_content = "Original content";
        fs::write(&file_path, initial_content).unwrap();
        manager.read_and_cache_file(&file_path, None, None).unwrap();

        // Modify file externally
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&file_path, "Externally modified").unwrap();

        // Try to edit - should fail due to staleness
        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 1,
                end_line: 1,
                new_content: "Should fail".to_string(),
            }],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .error_msg
                .contains("has been modified since last read")
        );
    }

    #[test]
    fn test_edit_create_nested_directory() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let nested_path = temp_dir.path().join("nested").join("deep").join("file.txt");

        // Create file in nested directory that doesn't exist
        let params = EditFileParams {
            path: nested_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 1,
                end_line: 0,
                new_content: "Content in nested directory".to_string(),
            }],
        };

        let result = manager.edit_file(&params);
        assert!(result.is_ok());

        // Verify file and directories were created
        assert!(nested_path.exists());
        let content = fs::read_to_string(&nested_path).unwrap();
        assert_eq!(content, "Content in nested directory\n");
    }


    #[test]
    fn test_get_edit_diff() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("diff_test.txt");

        // Create initial file
        let initial_content = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5";
        fs::write(&file_path, initial_content).unwrap();

        // Read the file first
        let read_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(read_params).unwrap();

        // Create edit parameters to modify line 3
        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 3,
                end_line: 3,
                new_content: "Modified Line 3".to_string(),
            }],
        };

        // Get the diff
        let result = manager.get_edit_diff(&params);
        assert!(result.is_ok());

        let diff_output = result.unwrap();
        assert!(diff_output.contains("Line 3"));
        assert!(diff_output.contains("Modified Line 3"));
        assert!(diff_output.contains("@@")); // Unified diff format marker
    }

    #[test]
    fn test_get_edit_diff_new_file() {
        let manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("new_diff_test.txt");

        // Create edit parameters for a new file
        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 1,
                end_line: 0,
                new_content: "New file content\nSecond line".to_string(),
            }],
        };

        // Get the diff
        let result = manager.get_edit_diff(&params);
        assert!(result.is_ok());

        let diff_output = result.unwrap();
        assert!(diff_output.contains("New file content"));
        assert!(diff_output.contains("Second line"));
        assert!(diff_output.contains("@@")); // Unified diff format marker
    }

    #[test]
    fn test_get_edit_diff_requires_read_first() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("diff_validation_test.txt");

        // Create initial file
        let initial_content = "Original content";
        fs::write(&file_path, initial_content).unwrap();

        // Try to get diff without reading first - should fail
        let params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 1,
                end_line: 1,
                new_content: "Modified content".to_string(),
            }],
        };

        let result = manager.get_edit_diff(&params);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be read before it can be edited"));

        // Now read the file first
        let read_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(read_params).unwrap();

        // Now get_edit_diff should work
        let result = manager.get_edit_diff(&params);
        assert!(result.is_ok());
    }

    #[test]
    fn test_partial_read_invalid_line_ranges() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("invalid_ranges.txt");

        // Create test file with 5 lines
        let content = "line 1\nline 2\nline 3\nline 4\nline 5";
        fs::write(&file_path, content).unwrap();

        // Test start_line = 0 (should error - lines are 1-indexed)
        let params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(0),
            end_line: Some(3),
        };
        let result = manager.read_file(params);
        assert!(result.is_err());
        let error_msg = &result.unwrap_err().error_msg;
        assert!(error_msg.contains("Invalid line range"));

        // Test negative start_line
        let params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(-1),
            end_line: Some(3),
        };
        let result = manager.read_file(params);
        assert!(result.is_err());
        assert!(result.unwrap_err().error_msg.contains("Invalid start_line: -1"));

        // Test negative end_line
        let params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(1),
            end_line: Some(-1),
        };
        let result = manager.read_file(params);
        assert!(result.is_err());
        assert!(result.unwrap_err().error_msg.contains("Invalid end_line: -1"));

        // Test start_line > end_line (after converting to internal format, this should error when reading)
        let params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(4),
            end_line: Some(2),
        };
        let result = manager.read_file(params);
        assert!(result.is_err());
        assert!(result.unwrap_err().error_msg.contains("Invalid line range"));

        // Test start_line beyond file (6 > 5 lines)
        let params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(6),
            end_line: Some(7),
        };
        let result = manager.read_file(params);
        assert!(result.is_err());
        assert!(result.unwrap_err().error_msg.contains("Invalid line range"));
    }

    #[test]
    fn test_multiple_overlapping_chunks_merging() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("overlapping_chunks.txt");

        // Create test file with 10 lines
        let content = "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10";
        fs::write(&file_path, content).unwrap();

        // Read lines 1-5
        let params1 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(1),
            end_line: Some(5),
        };
        let result = manager.read_file(params1);
        assert!(result.is_ok());

        // Read lines 3-7 (overlaps with previous: 3,4,5)
        let params2 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(3),
            end_line: Some(7),
        };
        let result = manager.read_file(params2);
        assert!(result.is_ok());

        // Read lines 6-10 (overlaps with previous: 6,7)
        let params3 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(6),
            end_line: Some(10),
        };
        let result = manager.read_file(params3);
        assert!(result.is_ok());

        // Verify the cache has merged into one continuous chunk (lines 1-10)
        let canonical_path = wasm_safe_normalize_path(&file_path).unwrap();
        let cached_entry = manager.cache.get(&canonical_path).unwrap();

        if let FileContent::Partial { slices, total_lines } = &cached_entry.content {
            assert_eq!(*total_lines, 10);
            assert_eq!(slices.len(), 1); // Should be merged into one slice
            assert_eq!(slices[0].start_line, 1);
            assert_eq!(slices[0].end_line, 10);
            assert_eq!(slices[0].lines.len(), 10);
            assert_eq!(slices[0].lines[0], "line 1");
            assert_eq!(slices[0].lines[9], "line 10");
        } else {
            panic!("Expected partial content");
        }

        // Verify the formatted output looks correct
        let formatted = cached_entry.content.get_numbered_content();
        assert!(formatted.contains("1:line 1"));
        assert!(formatted.contains("10:line 10"));
        assert!(!formatted.contains("omitted")); // No gaps should remain
    }

    #[test]
    fn test_adjacent_chunks_merging() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("adjacent_chunks.txt");

        // Create test file with 9 lines
        let content = "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9";
        fs::write(&file_path, content).unwrap();

        // Read lines 1-3
        let params1 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(1),
            end_line: Some(3),
        };
        manager.read_file(params1).unwrap();

        // Read lines 4-6 (adjacent to previous chunk)
        let params2 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(4),
            end_line: Some(6),
        };
        manager.read_file(params2).unwrap();

        // Read lines 7-9 (adjacent to previous chunk)
        let params3 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(7),
            end_line: Some(9),
        };
        manager.read_file(params3).unwrap();

        // Verify all chunks merged into one continuous chunk
        let canonical_path = wasm_safe_normalize_path(&file_path).unwrap();
        let cached_entry = manager.cache.get(&canonical_path).unwrap();

        if let FileContent::Partial { slices, total_lines } = &cached_entry.content {
            assert_eq!(*total_lines, 9);
            assert_eq!(slices.len(), 1); // Should be merged into one slice
            assert_eq!(slices[0].start_line, 1);
            assert_eq!(slices[0].end_line, 9);
            assert_eq!(slices[0].lines.len(), 9);
            
            // Verify content integrity
            for i in 0..9 {
                assert_eq!(slices[0].lines[i], format!("line {}", i + 1));
            }
        } else {
            panic!("Expected partial content");
        }

        // Verify the formatted output
        let formatted = cached_entry.content.get_numbered_content();
        for i in 1..=9 {
            assert!(formatted.contains(&format!("{}:line {}", i, i)));
        }
        assert!(!formatted.contains("omitted")); // No gaps
    }

    #[test]
    fn test_gap_chunks_with_proper_ordering() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("gap_chunks.txt");

        // Create test file with 10 lines
        let content = "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10";
        fs::write(&file_path, content).unwrap();

        // Read lines 1-3
        let params1 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(1),
            end_line: Some(3),
        };
        manager.read_file(params1).unwrap();

        // Read lines 7-9 (gap between 3 and 7)
        let params2 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(7),
            end_line: Some(9),
        };
        manager.read_file(params2).unwrap();

        // Read lines 5-6 (fills part of the gap)
        let params3 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(5),
            end_line: Some(6),
        };
        manager.read_file(params3).unwrap();

        // Verify we have separate chunks with proper ordering
        let canonical_path = wasm_safe_normalize_path(&file_path).unwrap();
        let cached_entry = manager.cache.get(&canonical_path).unwrap();

        if let FileContent::Partial { slices, total_lines } = &cached_entry.content {
            assert_eq!(*total_lines, 10);
            // After reading 1-3, 7-9, 5-6, the merge logic should result in:
            // 1-3, 5-9 (since 5-6 and 7-9 are adjacent and should merge)
            assert_eq!(slices.len(), 2);
            
            // Verify chunks are properly ordered
            assert_eq!(slices[0].start_line, 1);
            assert_eq!(slices[0].end_line, 3);
            assert_eq!(slices[1].start_line, 5);
            assert_eq!(slices[1].end_line, 9); // Should have merged 5-6 and 7-9
            
            // Verify content integrity
            assert_eq!(slices[0].lines, vec!["line 1", "line 2", "line 3"]);
            assert_eq!(slices[1].lines, vec!["line 5", "line 6", "line 7", "line 8", "line 9"]);
        } else {
            panic!("Expected partial content");
        }

        // Verify the formatted output shows gaps
        let formatted = cached_entry.content.get_numbered_content();
        assert!(formatted.contains("1:line 1"));
        assert!(formatted.contains("3:line 3"));
        assert!(formatted.contains("[... 1 lines omitted ...]")); // Gap at line 4
        assert!(formatted.contains("5:line 5"));
        assert!(formatted.contains("9:line 9"));
        assert!(formatted.contains("[... 1 lines omitted ...]")); // Gap at line 10

        // Now fill the remaining gap at line 4
        let params4 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(4),
            end_line: Some(4),
        };
        manager.read_file(params4).unwrap();

        // Should now merge into fewer chunks
        let cached_entry = manager.cache.get(&canonical_path).unwrap();
        if let FileContent::Partial { slices, .. } = &cached_entry.content {
            // Should merge 1-3, 4, 5-9 into 1-9 (one continuous chunk)
            assert_eq!(slices.len(), 1); // Just 1-9, with gap only at line 10
            assert_eq!(slices[0].start_line, 1);
            assert_eq!(slices[0].end_line, 9);
        }
    }

    #[test]
    fn test_get_files_info_multiple_files() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();

        // Create first file - will be read fully (small)
        let file1_path = temp_dir.path().join("small_file.txt");
        let file1_content = "line 1\nline 2\nline 3";
        fs::write(&file1_path, file1_content).unwrap();

        // Create second file - will be read partially
        let file2_path = temp_dir.path().join("partial_file.txt");
        let file2_content = "A1\nA2\nA3\nA4\nA5\nA6\nA7\nA8\nA9\nA10";
        fs::write(&file2_path, file2_content).unwrap();

        // Create third file - will also be read fully
        let file3_path = temp_dir.path().join("another_small.txt");
        let file3_content = "X\nY\nZ";
        fs::write(&file3_path, file3_content).unwrap();

        // Read first file completely
        let params1 = ReadFileParams {
            path: file1_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(params1).unwrap();

        // Read second file partially (lines 2-4 and 7-8)
        let params2a = ReadFileParams {
            path: file2_path.to_string_lossy().to_string(),
            start_line: Some(2),
            end_line: Some(4),
        };
        manager.read_file(params2a).unwrap();

        let params2b = ReadFileParams {
            path: file2_path.to_string_lossy().to_string(),
            start_line: Some(7),
            end_line: Some(8),
        };
        manager.read_file(params2b).unwrap();

        // Read third file completely
        let params3 = ReadFileParams {
            path: file3_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(params3).unwrap();

        // Get files info
        let files_info = manager.get_files_info();
        assert_eq!(files_info.len(), 3);

        // Files should be sorted by path
        let sorted_paths: Vec<_> = files_info.iter().map(|(p, _)| p.clone()).collect();
        let mut expected_paths = vec![
            file3_path.clone(), // "another_small.txt" comes first
            file2_path.clone(), // "partial_file.txt" 
            file1_path.clone(), // "small_file.txt"
        ];
        expected_paths.sort();
        assert_eq!(sorted_paths, expected_paths);

        // Check content of each file
        for (path, content) in files_info {
            if path.file_name().unwrap() == "small_file.txt" {
                assert!(content.contains("1:line 1"));
                assert!(content.contains("2:line 2"));
                assert!(content.contains("3:line 3"));
                assert!(!content.contains("omitted"));
            } else if path.file_name().unwrap() == "partial_file.txt" {
                assert!(content.contains("2:A2"));
                assert!(content.contains("3:A3"));
                assert!(content.contains("4:A4"));
                assert!(content.contains("7:A7"));
                assert!(content.contains("8:A8"));
                assert!(content.contains("[... 1 lines omitted ...]")); // Gap at line 1
                assert!(content.contains("[... 2 lines omitted ...]")); // Gap at lines 5-6
                assert!(content.contains("[... 2 lines omitted ...]")); // Gap at lines 9-10
            } else if path.file_name().unwrap() == "another_small.txt" {
                assert!(content.contains("1:X"));
                assert!(content.contains("2:Y"));
                assert!(content.contains("3:Z"));
                assert!(!content.contains("omitted"));
            }
        }
    }

    #[test]
    fn test_get_files_info_complex_partial_reads() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("complex_partial.txt");

        // Create file with 15 lines
        let content = (1..=15)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&file_path, content).unwrap();

        // Read several non-contiguous chunks to create a complex pattern
        // Read lines 2-4
        let params1 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(2),
            end_line: Some(4),
        };
        manager.read_file(params1).unwrap();

        // Read lines 7-9
        let params2 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(7),
            end_line: Some(9),
        };
        manager.read_file(params2).unwrap();

        // Read lines 12-14
        let params3 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(12),
            end_line: Some(14),
        };
        manager.read_file(params3).unwrap();

        // Read line 1 (extends first chunk)
        let params4 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(1),
            end_line: Some(1),
        };
        manager.read_file(params4).unwrap();

        // Read line 15 (extends last chunk)
        let params5 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(15),
            end_line: Some(15),
        };
        manager.read_file(params5).unwrap();

        // Get files info and verify complex partial structure
        let files_info = manager.get_files_info();
        assert_eq!(files_info.len(), 1);

        let (path, content_str) = &files_info[0];
        assert_eq!(path.file_name().unwrap(), "complex_partial.txt");

        // Verify it shows the chunks we read
        assert!(content_str.contains("1:line 1"));
        assert!(content_str.contains("2:line 2"));
        assert!(content_str.contains("4:line 4"));
        assert!(content_str.contains("7:line 7"));
        assert!(content_str.contains("9:line 9"));
        assert!(content_str.contains("12:line 12"));
        assert!(content_str.contains("15:line 15"));

        // Verify it shows gaps where we didn't read
        assert!(content_str.contains("[... 2 lines omitted ...]")); // lines 5-6
        assert!(content_str.contains("[... 2 lines omitted ...]")); // lines 10-11

        // Should not contain lines we didn't read
        assert!(!content_str.contains("5:line 5"));
        assert!(!content_str.contains("6:line 6"));
        assert!(!content_str.contains("10:line 10"));
        assert!(!content_str.contains("11:line 11"));

        // Verify the internal cache structure
        let canonical_path = wasm_safe_normalize_path(&file_path).unwrap();
        let cached_entry = manager.cache.get(&canonical_path).unwrap();
        
        if let FileContent::Partial { slices, total_lines } = &cached_entry.content {
            assert_eq!(*total_lines, 15);
            assert_eq!(slices.len(), 3); // Should have 3 chunks: 1-4, 7-9, 12-15
            
            // Verify chunk boundaries
            assert_eq!(slices[0].start_line, 1);
            assert_eq!(slices[0].end_line, 4);
            assert_eq!(slices[1].start_line, 7);
            assert_eq!(slices[1].end_line, 9);
            assert_eq!(slices[2].start_line, 12);
            assert_eq!(slices[2].end_line, 15);
        } else {
            panic!("Expected partial content");
        }
    }

    #[test]
    fn test_partial_read_cache_optimization() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("cache_optimization.txt");

        // Create test file with 10 lines
        let content = (1..=10)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&file_path, content).unwrap();

        // Read a larger chunk first (lines 1-10)
        let params1 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(1),
            end_line: Some(10),
        };
        let result1 = manager.read_file(params1);
        assert!(result1.is_ok());

        // Verify we have the full range cached
        let canonical_path = wasm_safe_normalize_path(&file_path).unwrap();
        let cached_entry = manager.cache.get(&canonical_path).unwrap();
        let original_cache_time = cached_entry.last_modified_at_read;

        // Now try to read a subset (lines 5-7) - should hit cache
        let params2 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(5),
            end_line: Some(7),
        };
        let result2 = manager.read_file(params2);
        assert!(result2.is_ok());

        // Verify cache wasn't re-read (same modification time)
        let cached_entry_after = manager.cache.get(&canonical_path).unwrap();
        assert_eq!(cached_entry_after.last_modified_at_read, original_cache_time);

        // Verify we still have the same cache structure
        if let FileContent::Partial { slices, total_lines } = &cached_entry_after.content {
            assert_eq!(*total_lines, 10);
            assert_eq!(slices.len(), 1); // Still one chunk
            assert_eq!(slices[0].start_line, 1);
            assert_eq!(slices[0].end_line, 10);
        } else {
            panic!("Expected partial content");
        }

        // The result should contain the subset we asked for
        let result2_content = result2.unwrap();
        assert!(result2_content.message.contains("Read file:"));
        assert!(result2_content.ui_display.collapsed.contains("5–7"));
        
        // Try reading a subset that's completely cached (lines 8-10)
        // This should hit the cache since we already have lines 1-10
        let params3 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(8),
            end_line: Some(10),
        };
        let result3 = manager.read_file(params3);
        assert!(result3.is_ok());

        // Verify cache structure is still the same (wasn't re-read)
        let cached_entry_final = manager.cache.get(&canonical_path).unwrap();
        assert_eq!(cached_entry_final.last_modified_at_read, original_cache_time);

        // Try reading completely beyond cached range (lines 15-20) - should fail
        let params4 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(15),
            end_line: Some(20),
        };
        let result4 = manager.read_file(params4);
        assert!(result4.is_err());
        assert!(result4.unwrap_err().error_msg.contains("Invalid line range"));
    }

    #[test]
    fn test_partial_to_full_upgrade() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("upgrade_test.txt");

        // Create small test file (will be under 64KB limit)
        let content = (1..=5)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&file_path, content).unwrap();

        // First, read the file partially (lines 2-4)
        let params1 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(2),
            end_line: Some(4),
        };
        let result1 = manager.read_file(params1);
        assert!(result1.is_ok());

        // Verify we have partial content
        let canonical_path = wasm_safe_normalize_path(&file_path).unwrap();
        let cached_entry = manager.cache.get(&canonical_path).unwrap();
        
        match &cached_entry.content {
            FileContent::Partial { slices, total_lines } => {
                assert_eq!(*total_lines, 5);
                assert_eq!(slices.len(), 1);
                assert_eq!(slices[0].start_line, 2);
                assert_eq!(slices[0].end_line, 4);
            }
            _ => panic!("Expected partial content after partial read"),
        }

        // Clear cache to force re-read for full file
        manager.cache.clear();

        // Now read the entire file (no line range specified)
        let params2 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        let result2 = manager.read_file(params2);
        assert!(result2.is_ok());

        // Verify cache has Full content
        let cached_entry_after = manager.cache.get(&canonical_path).unwrap();
        
        match &cached_entry_after.content {
            FileContent::Full(full_content) => {
                // Should contain all lines
                let lines: Vec<&str> = full_content.lines().collect();
                assert_eq!(lines.len(), 5);
                for i in 1..=5 {
                    assert_eq!(lines[i-1], format!("line {}", i));
                }
            }
            _ => panic!("Expected full content after full read"),
        }

        // Verify the full content is properly formatted when retrieved
        let formatted = cached_entry_after.content.get_numbered_content();
        for i in 1..=5 {
            assert!(formatted.contains(&format!("{}:line {}", i, i)));
        }
        assert!(!formatted.contains("omitted")); // No gaps in full content

        // Verify subsequent partial reads work on the full content
        let params3 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(1),
            end_line: Some(2),
        };
        let result3 = manager.read_file(params3);
        assert!(result3.is_ok());

        // Cache should still be Full (no downgrade)
        let cached_entry_final = manager.cache.get(&canonical_path).unwrap();
        match &cached_entry_final.content {
            FileContent::Full(_) => {}, // Good, still full
            _ => panic!("Cache should remain full after partial read on full content"),
        }
    }

    #[test]
    fn test_slice_merge_exact_boundaries() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("exact_boundaries.txt");

        // Create test file with 10 lines
        let content = (1..=10)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&file_path, content).unwrap();

        // Read lines 1-5
        let params1 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(1),
            end_line: Some(5),
        };
        manager.read_file(params1).unwrap();

        // Read lines 5-10 (shares boundary at line 5)
        let params2 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(5),
            end_line: Some(10),
        };
        manager.read_file(params2).unwrap();

        // Verify they merged into one continuous chunk
        let canonical_path = wasm_safe_normalize_path(&file_path).unwrap();
        let cached_entry = manager.cache.get(&canonical_path).unwrap();

        if let FileContent::Partial { slices, total_lines } = &cached_entry.content {
            assert_eq!(*total_lines, 10);
            assert_eq!(slices.len(), 1); // Should merge into single chunk
            assert_eq!(slices[0].start_line, 1);
            assert_eq!(slices[0].end_line, 10);
            
            // Verify no duplication of line 5
            assert_eq!(slices[0].lines.len(), 10);
            for i in 0..10 {
                assert_eq!(slices[0].lines[i], format!("line {}", i + 1));
            }
        } else {
            panic!("Expected partial content");
        }

        // Test the edge case where slices meet exactly (6-8, then 8-10)
        manager.cache.clear();
        
        // Read lines 6-8
        let params3 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(6),
            end_line: Some(8),
        };
        manager.read_file(params3).unwrap();

        // Read lines 8-10 (shares boundary at line 8)  
        let params4 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(8),
            end_line: Some(10),
        };
        manager.read_file(params4).unwrap();

        // Verify merge handled duplicate line 8 correctly
        let cached_entry = manager.cache.get(&canonical_path).unwrap();
        if let FileContent::Partial { slices, .. } = &cached_entry.content {
            assert_eq!(slices.len(), 1); // Should merge
            assert_eq!(slices[0].start_line, 6);
            assert_eq!(slices[0].end_line, 10);
            assert_eq!(slices[0].lines.len(), 5); // Lines 6,7,8,9,10 (no duplication)
            assert_eq!(slices[0].lines[2], "line 8"); // Verify line 8 appears only once
        }
    }

    #[test]
    fn test_slice_merge_subset_scenarios() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("subset_merge.txt");

        // Create test file with 10 lines
        let content = (1..=10)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&file_path, content).unwrap();

        // Read a large chunk first (lines 1-10)
        let params1 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(1),
            end_line: Some(10),
        };
        manager.read_file(params1).unwrap();

        let canonical_path = wasm_safe_normalize_path(&file_path).unwrap();
        let _original_cache = manager.cache.get(&canonical_path).unwrap().clone();

        // Now read a subset that's completely contained (lines 3-7)
        let params2 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(3),
            end_line: Some(7),
        };
        manager.read_file(params2).unwrap();

        // Verify cache structure didn't change (subset was already covered)
        let cached_entry = manager.cache.get(&canonical_path).unwrap();
        if let FileContent::Partial { slices, total_lines } = &cached_entry.content {
            assert_eq!(*total_lines, 10);
            assert_eq!(slices.len(), 1); // Still one chunk
            assert_eq!(slices[0].start_line, 1);
            assert_eq!(slices[0].end_line, 10);
            assert_eq!(slices[0].lines.len(), 10); // No change in content
        } else {
            panic!("Expected partial content");
        }

        // Test another subset scenario with gaps
        manager.cache.clear();

        // Read lines 2-8
        let params3 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(2),
            end_line: Some(8),
        };
        manager.read_file(params3).unwrap();

        // Now read a subset (lines 4-6) that's within the existing range
        let params4 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(4),
            end_line: Some(6),
        };
        manager.read_file(params4).unwrap();

        // Verify no duplication or corruption
        let cached_entry = manager.cache.get(&canonical_path).unwrap();
        if let FileContent::Partial { slices, .. } = &cached_entry.content {
            assert_eq!(slices.len(), 1);
            assert_eq!(slices[0].start_line, 2);
            assert_eq!(slices[0].end_line, 8);
            assert_eq!(slices[0].lines.len(), 7); // Lines 2,3,4,5,6,7,8
            assert_eq!(slices[0].lines[2], "line 4"); // Verify line 4 is correct
            assert_eq!(slices[0].lines[4], "line 6"); // Verify line 6 is correct
        }
    }

    #[test]
    fn test_sequential_edit_line_number_changes() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("sequential_edits.txt");

        // Create initial file
        let initial_content = "line 1\nline 2\nline 3\nline 4\nline 5";
        fs::write(&file_path, initial_content).unwrap();

        // Read the file first (required for editing)
        let read_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(read_params).unwrap();

        // Test multiple edits that interact with each other
        // Edits are processed in reverse order (highest line number first)
        let edit_params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![
                // Replace line 2
                Edit {
                    start_line: 2,
                    end_line: 2,
                    new_content: "modified line 2".to_string(),
                },
                // Insert two lines after line 3
                Edit {
                    start_line: 4,
                    end_line: 3, // Insert position 
                    new_content: "inserted A\ninserted B".to_string(),
                },
                // Delete line 5 (this will be the original line 5, not affected by earlier operations since they're processed in reverse)
                Edit {
                    start_line: 5,
                    end_line: 5,
                    new_content: "".to_string(),
                },
            ],
        };

        let result = manager.edit_file(&edit_params);
        assert!(result.is_ok());

        // Verify the final content
        let content = fs::read_to_string(&file_path).unwrap();
        
        // Processing order (reverse by line number):
        // 1. Start: "line 1\nline 2\nline 3\nline 4\nline 5" 
        // 2. Delete line 5: "line 1\nline 2\nline 3\nline 4"
        // 3. Insert at line 4: "line 1\nline 2\nline 3\ninserted A\ninserted B\nline 4" 
        // 4. Replace line 2: "line 1\nmodified line 2\nline 3\ninserted A\ninserted B\nline 4"
        let expected = "line 1\nmodified line 2\nline 3\ninserted A\ninserted B\nline 4\n";
        assert_eq!(content, expected);
    }

    #[test]
    fn test_edit_insert_at_file_end() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("insert_at_end.txt");

        // Create initial file with 3 lines
        let initial_content = "line 1\nline 2\nline 3";
        fs::write(&file_path, initial_content).unwrap();

        // Read the file first
        let read_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(read_params).unwrap();

        // Test inserting at the very end of the file (line 4 = total_lines + 1)
        let edit_params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 4, // Insert after the last line (3)
                end_line: 3,   // Insert position
                new_content: "appended line 1\nappended line 2".to_string(),
            }],
        };

        let result = manager.edit_file(&edit_params);
        assert!(result.is_ok());

        // Verify the content
        let content = fs::read_to_string(&file_path).unwrap();
        let expected = "line 1\nline 2\nline 3\nappended line 1\nappended line 2\n";
        assert_eq!(content, expected);

        // Test another insert at the new end
        let edit_params2 = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 6, // Insert after line 5 (the new total)
                end_line: 5,   
                new_content: "final line".to_string(),
            }],
        };

        let result2 = manager.edit_file(&edit_params2);
        assert!(result2.is_ok());

        let content2 = fs::read_to_string(&file_path).unwrap();
        let expected2 = "line 1\nline 2\nline 3\nappended line 1\nappended line 2\nfinal line\n";
        assert_eq!(content2, expected2);

        // Test edge case: try to insert beyond the file end + 1 (should fail)
        let edit_params3 = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 8, // Too far beyond the file (current length is 6)
                end_line: 7,
                new_content: "invalid insert".to_string(),
            }],
        };

        let result3 = manager.edit_file(&edit_params3);
        assert!(result3.is_err());
        assert!(result3.unwrap_err().error_msg.contains("start_line cannot be greater than"));
    }

    #[test]
    fn test_mixed_edit_operations_single_call() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("mixed_operations.txt");

        // Create initial file with 6 lines
        let initial_content = "line 1\nline 2\nline 3\nline 4\nline 5\nline 6";
        fs::write(&file_path, initial_content).unwrap();

        // Read the file first
        let read_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(read_params).unwrap();

        // Mix of insert, replace, and delete operations in a single call
        let edit_params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![
                // Replace line 1
                Edit {
                    start_line: 1,
                    end_line: 1,
                    new_content: "REPLACED line 1".to_string(),
                },
                // Insert after line 2
                Edit {
                    start_line: 3,
                    end_line: 2,
                    new_content: "INSERTED line".to_string(),
                },
                // Delete lines 4-5 (original numbering)
                Edit {
                    start_line: 4,
                    end_line: 5,
                    new_content: "".to_string(),
                },
                // Replace line 6 (original numbering)
                Edit {
                    start_line: 6,
                    end_line: 6,
                    new_content: "REPLACED line 6".to_string(),
                },
                // Insert at the end
                Edit {
                    start_line: 7,
                    end_line: 6,
                    new_content: "APPENDED line".to_string(),
                },
            ],
        };

        let result = manager.edit_file(&edit_params);
        assert!(result.is_ok());

        // Verify the final content
        let content = fs::read_to_string(&file_path).unwrap();
        
        // Expected processing order (reverse by line number):
        // 1. Start: "line 1\nline 2\nline 3\nline 4\nline 5\nline 6"
        // 2. Insert at line 7: "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nAPPENDED line"
        // 3. Replace line 6: "line 1\nline 2\nline 3\nline 4\nline 5\nREPLACED line 6\nAPPENDED line"
        // 4. Delete lines 4-5: "line 1\nline 2\nline 3\nREPLACED line 6\nAPPENDED line"
        // 5. Insert at line 3: "line 1\nline 2\nINSERTED line\nline 3\nREPLACED line 6\nAPPENDED line"
        // 6. Replace line 1: "REPLACED line 1\nline 2\nINSERTED line\nline 3\nREPLACED line 6\nAPPENDED line"

        let expected = "REPLACED line 1\nline 2\nINSERTED line\nline 3\nREPLACED line 6\nAPPENDED line\n";
        assert_eq!(content, expected);

        // Verify the file has the expected number of lines
        let line_count = content.lines().count();
        assert_eq!(line_count, 6); // Started with 6, deleted 2, inserted 2, net = 6
    }

    #[test]
    fn test_file_modification_between_operations() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("modification_detection.txt");

        // Create initial file
        let initial_content = "line 1\nline 2\nline 3";
        fs::write(&file_path, initial_content).unwrap();

        // Read the file to cache it
        let read_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(read_params).unwrap();

        // Modify the file externally (simulate another process changing it)
        std::thread::sleep(std::time::Duration::from_millis(10)); // Ensure different timestamp
        fs::write(&file_path, "EXTERNALLY MODIFIED\nline 2\nline 3").unwrap();

        // Try to edit the file - should fail due to staleness check
        let edit_params = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 1,
                end_line: 1,
                new_content: "edited line 1".to_string(),
            }],
        };

        let result = manager.edit_file(&edit_params);
        assert!(result.is_err());
        assert!(result.unwrap_err().error_msg.contains("has been modified since last read"));

        // Read the file again to update cache
        let read_params2 = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(read_params2).unwrap();

        // Now editing should work
        let result2 = manager.edit_file(&edit_params);
        assert!(result2.is_ok());

        // Verify the edit was applied to the externally modified content
        let content = fs::read_to_string(&file_path).unwrap();
        let expected = "edited line 1\nline 2\nline 3\n";
        assert_eq!(content, expected);

        // Test the staleness check for partial reads too
        manager.cache.clear();
        fs::write(&file_path, "partial 1\npartial 2\npartial 3\npartial 4\npartial 5").unwrap();

        // Read partially
        let partial_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: Some(2),
            end_line: Some(4),
        };
        manager.read_file(partial_params).unwrap();

        // Modify file externally again
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&file_path, "MODIFIED partial 1\nMODIFIED partial 2\nMODIFIED partial 3\nMODIFIED partial 4\nMODIFIED partial 5").unwrap();

        // Edit should fail
        let edit_params3 = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 3,
                end_line: 3,
                new_content: "should fail".to_string(),
            }],
        };

        let result3 = manager.edit_file(&edit_params3);
        assert!(result3.is_err());
        assert!(result3.unwrap_err().error_msg.contains("has been modified since last read"));
    }

    #[test]
    fn test_empty_content_variations_in_edits() {
        let mut manager = FileInteractionManager::new();
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("empty_content_test.txt");

        // Create initial file
        let initial_content = "line 1\nline 2\nline 3\nline 4";
        fs::write(&file_path, initial_content).unwrap();

        // Read the file first
        let read_params = ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        };
        manager.read_file(read_params).unwrap();

        // Test 1: Delete using empty string
        let edit_params1 = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 2,
                end_line: 3,
                new_content: "".to_string(), // Empty string deletion
            }],
        };

        let result1 = manager.edit_file(&edit_params1);
        assert!(result1.is_ok());

        let content1 = fs::read_to_string(&file_path).unwrap();
        let expected1 = "line 1\nline 4\n";
        assert_eq!(content1, expected1);

        // Test 2: Insert empty content (should be a no-op?)
        let edit_params2 = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 2,
                end_line: 1, // Insert position
                new_content: "".to_string(), // Empty insert
            }],
        };

        let result2 = manager.edit_file(&edit_params2);
        assert!(result2.is_ok());

        // File should be unchanged since we're inserting nothing
        let content2 = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content2, content1); // Should be same as before

        // Test 3: Replace with empty string (delete)
        fs::write(&file_path, "test 1\ntest 2\ntest 3").unwrap();
        manager.read_file(ReadFileParams {
            path: file_path.to_string_lossy().to_string(),
            start_line: None,
            end_line: None,
        }).unwrap();

        let edit_params3 = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 2,
                end_line: 2,
                new_content: "".to_string(), // Replace with empty (delete line)
            }],
        };

        let result3 = manager.edit_file(&edit_params3);
        assert!(result3.is_ok());

        let content3 = fs::read_to_string(&file_path).unwrap();
        let expected3 = "test 1\ntest 3\n";
        assert_eq!(content3, expected3);

        // Test 4: Delete entire file content
        let edit_params4 = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 1,
                end_line: 2, // Delete all remaining lines
                new_content: "".to_string(),
            }],
        };

        let result4 = manager.edit_file(&edit_params4);
        assert!(result4.is_ok());

        let content4 = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content4, ""); // File should be completely empty

        // Test 5: Add content to empty file
        let edit_params5 = EditFileParams {
            path: file_path.to_string_lossy().to_string(),
            edits: vec![Edit {
                start_line: 1,
                end_line: 0, // Insert at beginning of empty file
                new_content: "new line 1\nnew line 2".to_string(),
            }],
        };

        let result5 = manager.edit_file(&edit_params5);
        assert!(result5.is_ok());

        let content5 = fs::read_to_string(&file_path).unwrap();
        let expected5 = "new line 1\nnew line 2\n";
        assert_eq!(content5, expected5);
    }
}
