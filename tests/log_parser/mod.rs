/// Log Parser for HIVE Integration Tests
/// 
/// Provides structured parsing and analysis of HIVE system logs
/// for comprehensive testing and verification.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};

// Import the Message types for deserialization
use crate::docker_sandbox::*; // This will give us access to Message types through the parent module

/// Represents a single structured log entry
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub target: String,
    pub span_name: Option<String>,
    pub message: String,
    pub fields: HashMap<String, serde_json::Value>,
    pub thread_id: Option<String>,
    // Parsed message objects
    pub hive_message: Option<serde_json::Value>, // Serialized Message
    pub inter_agent_message: Option<serde_json::Value>, // Serialized InterAgentMessage
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl std::str::FromStr for LogLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "TRACE" => Ok(LogLevel::Trace),
            "DEBUG" => Ok(LogLevel::Debug),
            "INFO" => Ok(LogLevel::Info),
            "WARN" => Ok(LogLevel::Warn),
            "ERROR" => Ok(LogLevel::Error),
            _ => Err(format!("Unknown log level: {}", s)),
        }
    }
}

/// Parser for HIVE system logs
pub struct LogParser {
    entries: Vec<LogEntry>,
}

impl LogParser {
    /// Create a new log parser
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Parse log content from raw text output
    pub fn parse_log_content(content: &str) -> Result<Self, String> {
        let mut parser = Self::new();
        
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(entry) = parser.parse_log_line(line) {
                parser.entries.push(entry);
            }
            // Don't fail on unparseable lines - they might be continuations or other content
        }
        
        Ok(parser)
    }

    /// Parse a single log line - handles both JSON and standard tracing format
    fn parse_log_line(&self, line: &str) -> Result<LogEntry, String> {
        // Try JSON format first
        if line.trim().starts_with('{') {
            return self.parse_json_log_line(line);
        }
        
        // Fall back to parsing standard tracing format
        self.parse_standard_log_line(line)
    }

    /// Parse JSON structured log line
    fn parse_json_log_line(&self, line: &str) -> Result<LogEntry, String> {
        let json: serde_json::Value = serde_json::from_str(line)
            .map_err(|e| format!("Failed to parse JSON log line: {}", e))?;
            
        let timestamp = json.get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);
            
        let level = json.get("level")
            .and_then(|l| l.as_str())
            .unwrap_or("INFO")
            .parse()
            .unwrap_or(LogLevel::Info);
            
        let target = json.get("target")
            .and_then(|t| t.as_str())
            .unwrap_or("unknown")
            .to_string();
            
        let span_name = json.get("span")
            .and_then(|s| s.get("name"))
            .and_then(|n| n.as_str())
            .map(String::from);
            
        let message = json.get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
            
        let mut fields = HashMap::new();
        if let Some(fields_obj) = json.get("fields").and_then(|f| f.as_object()) {
            for (key, value) in fields_obj {
                fields.insert(key.clone(), value.clone());
            }
        }
        
        let thread_id = json.get("thread_id")
            .and_then(|t| t.as_str())
            .map(String::from);

        // Try to parse message content if it contains serialized messages
        let (hive_message, inter_agent_message) = Self::extract_message_objects(&fields);

        Ok(LogEntry {
            timestamp,
            level,
            target,
            span_name,
            message,
            fields,
            thread_id,
            hive_message,
            inter_agent_message,
        })
    }

    /// Parse standard tracing log line format
    /// Format: 2025-06-04T18:52:02.648550Z DEBUG ThreadId(01) span_name: target: message
    fn parse_standard_log_line(&self, line: &str) -> Result<LogEntry, String> {
        let line = line.trim();
        if line.is_empty() {
            return Err("Empty line".to_string());
        }

        // Split into components, but handle multiple spaces
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            return Err("Invalid log line format".to_string());
        }

        let timestamp = parts[0].parse::<DateTime<Utc>>()
            .unwrap_or_else(|_| Utc::now());

        let level = parts[1].parse::<LogLevel>()
            .unwrap_or(LogLevel::Info);

        let thread_id = Some(parts[2].to_string());

        // Everything after thread_id contains span_name: target: message
        let remaining = parts[3..].join(" ");

        let (span_name, target, message) = self.extract_span_target_message(&remaining);
        
        // Extract structured fields from message
        let fields = self.extract_fields_from_message(&message);
        
        // Try to parse message content if it contains serialized messages
        let (hive_message, inter_agent_message) = Self::extract_message_objects(&fields);

        Ok(LogEntry {
            timestamp,
            level,
            target,
            span_name,
            message,
            fields,
            thread_id,
            hive_message,
            inter_agent_message,
        })
    }

    /// Extract span name, target, and message from the remaining log content
    /// Handles format: span_name: target: message OR target: message
    fn extract_span_target_message(&self, remaining: &str) -> (Option<String>, String, String) {
        // Strategy: Look for the first colon followed by space, then look for patterns
        // that indicate whether this is span:target: or just target:
        
        if let Some(first_colon) = remaining.find(": ") {
            let before_first = &remaining[..first_colon];
            let after_first = &remaining[first_colon + 2..]; // +2 for ": "
            
            // Check if there's another ": " in the remainder
            if let Some(second_colon_in_remainder) = after_first.find(": ") {
                // We have span: target: message format
                let target_end = first_colon + 2 + second_colon_in_remainder;
                let target = remaining[first_colon + 2..target_end].trim();
                let message = remaining[target_end + 2..].trim(); // +2 for ": "
                
                // Additional validation: span names are usually identifiers or contain underscores
                // If before_first contains "::" it's more likely a target (like hive::config)
                if before_first.contains("::") {
                    // This is likely target: message, with the second ":" being in the message
                    (None, before_first.to_string(), after_first.to_string())
                } else {
                    // This is likely span: target: message
                    (Some(before_first.to_string()), target.to_string(), message.to_string())
                }
            } else {
                // Only one ": " found, this is target: message
                (None, before_first.to_string(), after_first.to_string())
            }
        } else {
            // No ": " found, treat everything as message with unknown target
            (None, "unknown".to_string(), remaining.to_string())
        }
    }

    /// Extract span name from target or message (kept for compatibility)
    fn extract_span_name(&self, target: &str, message: &str) -> Option<String> {
        // Look for span patterns in the message
        if message.contains("enter span") || message.contains("exit span") {
            // Extract span name from patterns like "enter span: span_name"
            if let Some(start) = message.find("span: ") {
                let span_part = &message[start + 6..];
                if let Some(end) = span_part.find(' ') {
                    return Some(span_part[..end].to_string());
                } else {
                    return Some(span_part.to_string());
                }
            }
        }
        
        // Look for known span patterns in target
        if target.contains("::") {
            target.split("::").last().map(String::from)
        } else {
            None
        }
    }

    /// Extract structured fields from message content
    fn extract_fields_from_message(&self, message: &str) -> HashMap<String, serde_json::Value> {
        let mut fields = HashMap::new();
        
        // Look for key=value patterns
        for part in message.split_whitespace() {
            if let Some(eq_pos) = part.find('=') {
                let key = &part[..eq_pos];
                let value = &part[eq_pos + 1..];
                
                // Try to parse as different types
                if let Ok(num) = value.parse::<i64>() {
                    fields.insert(key.to_string(), serde_json::Value::Number(num.into()));
                } else if let Ok(boolean) = value.parse::<bool>() {
                    fields.insert(key.to_string(), serde_json::Value::Bool(boolean));
                } else {
                    fields.insert(key.to_string(), serde_json::Value::String(value.to_string()));
                }
            }
        }
        
        fields
    }

    /// Extract and parse message objects from log fields
    fn extract_message_objects(fields: &HashMap<String, serde_json::Value>) -> (Option<serde_json::Value>, Option<serde_json::Value>) {
        let hive_message = fields.get("message")
            .and_then(|v| v.as_str())
            .and_then(|s| {
                // Try to parse as JSON
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(s) {
                    // Check if it looks like a Message by checking for common Message variants
                    if parsed.is_object() {
                        let obj = parsed.as_object().unwrap();
                        if obj.contains_key("Action") || obj.contains_key("AssistantResponse") || 
                           obj.contains_key("TaskCompleted") || obj.contains_key("AgentSpawned") {
                            return Some(parsed);
                        }
                    }
                }
                None
            });

        let inter_agent_message = fields.get("message")
            .and_then(|v| v.as_str())
            .and_then(|s| {
                // Try to parse as JSON
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(s) {
                    // Check if it looks like an InterAgentMessage
                    if parsed.is_object() {
                        let obj = parsed.as_object().unwrap();
                        if obj.contains_key("TaskStatusUpdate") || obj.contains_key("PlanApproved") || 
                           obj.contains_key("PlanRejected") {
                            return Some(parsed);
                        }
                    }
                }
                None
            });

        (hive_message, inter_agent_message)
    }

    /// Get all parsed log entries
    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    /// Get entries by log level
    pub fn entries_by_level(&self, level: LogLevel) -> Vec<&LogEntry> {
        self.entries.iter().filter(|e| e.level == level).collect()
    }

    /// Get entries by target pattern
    pub fn entries_by_target(&self, pattern: &str) -> Vec<&LogEntry> {
        self.entries.iter().filter(|e| e.target.contains(pattern)).collect()
    }

    /// Get entries by span name
    pub fn entries_by_span(&self, span_name: &str) -> Vec<&LogEntry> {
        self.entries.iter()
            .filter(|e| e.span_name.as_ref().map_or(false, |s| s.contains(span_name)))
            .collect()
    }

    /// Get entries containing message pattern
    pub fn entries_with_message(&self, pattern: &str) -> Vec<&LogEntry> {
        self.entries.iter()
            .filter(|e| e.message.contains(pattern))
            .collect()
    }

    /// Get entries with specific field
    pub fn entries_with_field(&self, field_name: &str) -> Vec<&LogEntry> {
        self.entries.iter()
            .filter(|e| e.fields.contains_key(field_name))
            .collect()
    }

    /// Get entries with field matching value
    pub fn entries_with_field_value(&self, field_name: &str, expected_value: &serde_json::Value) -> Vec<&LogEntry> {
        self.entries.iter()
            .filter(|e| e.fields.get(field_name) == Some(expected_value))
            .collect()
    }

    /// Check if logs contain specific sequence of events
    pub fn contains_sequence(&self, patterns: &[&str]) -> bool {
        let mut pattern_index = 0;
        
        for entry in &self.entries {
            if pattern_index < patterns.len() && entry.message.contains(patterns[pattern_index]) {
                pattern_index += 1;
                if pattern_index == patterns.len() {
                    return true;
                }
            }
        }
        
        false
    }

    /// Get entries that contain HIVE messages
    pub fn entries_with_hive_messages(&self) -> Vec<&LogEntry> {
        self.entries.iter().filter(|e| e.hive_message.is_some()).collect()
    }

    /// Get entries that contain InterAgent messages
    pub fn entries_with_inter_agent_messages(&self) -> Vec<&LogEntry> {
        self.entries.iter().filter(|e| e.inter_agent_message.is_some()).collect()
    }

    /// Get entries containing TaskCompleted messages
    pub fn entries_with_task_completed(&self) -> Vec<&LogEntry> {
        self.entries.iter()
            .filter(|e| {
                e.hive_message.as_ref()
                    .and_then(|msg| msg.as_object())
                    .map_or(false, |obj| obj.contains_key("TaskCompleted"))
            })
            .collect()
    }

    /// Get entries containing AssistantToolCall messages
    pub fn entries_with_assistant_tool_calls(&self) -> Vec<&LogEntry> {
        self.entries.iter()
            .filter(|e| {
                e.hive_message.as_ref()
                    .and_then(|msg| msg.as_object())
                    .map_or(false, |obj| obj.contains_key("AssistantToolCall"))
            })
            .collect()
    }

    /// Get entries containing specific tool calls by name
    pub fn entries_with_tool_call(&self, tool_name: &str) -> Vec<&LogEntry> {
        self.entries.iter()
            .filter(|e| {
                e.hive_message.as_ref()
                    .and_then(|msg| msg.as_object())
                    .and_then(|obj| obj.get("AssistantToolCall"))
                    .and_then(|tool_call| tool_call.as_object())
                    .and_then(|tc| tc.get("fn_name"))
                    .and_then(|name| name.as_str())
                    .map_or(false, |name| name == tool_name)
            })
            .collect()
    }

    /// Check if logs contain sequence of message types
    pub fn contains_message_sequence(&self, message_patterns: &[&str]) -> bool {
        let mut pattern_index = 0;
        
        for entry in &self.entries {
            if pattern_index < message_patterns.len() {
                let pattern = message_patterns[pattern_index];
                let matches = entry.hive_message.as_ref()
                    .and_then(|msg| msg.as_object())
                    .map_or(false, |obj| obj.contains_key(pattern));
                    
                if matches {
                    pattern_index += 1;
                    if pattern_index == message_patterns.len() {
                        return true;
                    }
                }
            }
        }
        
        false
    }

    /// Get statistics about the logs
    pub fn stats(&self) -> LogStats {
        let mut stats = LogStats::new();
        
        for entry in &self.entries {
            stats.total_entries += 1;
            match entry.level {
                LogLevel::Trace => stats.trace_count += 1,
                LogLevel::Debug => stats.debug_count += 1,
                LogLevel::Info => stats.info_count += 1,
                LogLevel::Warn => stats.warn_count += 1,
                LogLevel::Error => stats.error_count += 1,
            }
            
            *stats.targets.entry(entry.target.clone()).or_insert(0) += 1;
            
            if let Some(span) = &entry.span_name {
                *stats.spans.entry(span.clone()).or_insert(0) += 1;
            }
        }
        
        stats
    }
}

/// Statistics about parsed logs
#[derive(Debug)]
pub struct LogStats {
    pub total_entries: usize,
    pub trace_count: usize,
    pub debug_count: usize,
    pub info_count: usize,
    pub warn_count: usize,
    pub error_count: usize,
    pub targets: HashMap<String, usize>,
    pub spans: HashMap<String, usize>,
}

impl LogStats {
    pub fn new() -> Self {
        Self {
            total_entries: 0,
            trace_count: 0,
            debug_count: 0,
            info_count: 0,
            warn_count: 0,
            error_count: 0,
            targets: HashMap::new(),
            spans: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_standard_log_line_simple() {
        let parser = LogParser::new();
        let line = "2024-01-01T12:00:00.000000Z DEBUG ThreadId(01) hive::agent: Agent started successfully";
        
        let entry = parser.parse_standard_log_line(line).unwrap();
        assert_eq!(entry.level, LogLevel::Debug);
        assert_eq!(entry.target, "hive::agent");
        assert_eq!(entry.message, "Agent started successfully");
        assert_eq!(entry.thread_id, Some("ThreadId(01)".to_string()));
        assert_eq!(entry.span_name, None);
    }

    #[test]
    fn test_parse_standard_log_line_with_span() {
        let parser = LogParser::new();
        let line = "2025-06-04T18:52:02.651712Z  INFO ThreadId(01) start_headless_hive: hive::hive: enter initial_prompt=\"test\" prompt_length=4";
        
        let entry = parser.parse_standard_log_line(line).unwrap();
        assert_eq!(entry.level, LogLevel::Info);
        assert_eq!(entry.target, "hive::hive");
        assert_eq!(entry.message, "enter initial_prompt=\"test\" prompt_length=4");
        assert_eq!(entry.thread_id, Some("ThreadId(01)".to_string()));
        assert_eq!(entry.span_name, Some("start_headless_hive".to_string()));
    }

    #[test]
    fn test_parse_standard_log_line_with_complex_span() {
        let parser = LogParser::new();
        let line = "2025-06-04T18:52:02.655136Z  INFO ThreadId(10) actor_lifecycle:agent_run:start_actors:actor_lifecycle: hive::actors: Actor ready, sending ready signal actor_id=\"assistant\"";
        
        let entry = parser.parse_standard_log_line(line).unwrap();
        assert_eq!(entry.level, LogLevel::Info);
        assert_eq!(entry.target, "hive::actors");
        assert_eq!(entry.message, "Actor ready, sending ready signal actor_id=\"assistant\"");
        assert_eq!(entry.thread_id, Some("ThreadId(10)".to_string()));
        assert_eq!(entry.span_name, Some("actor_lifecycle:agent_run:start_actors:actor_lifecycle".to_string()));
    }

    #[test]
    fn test_extract_span_target_message() {
        let parser = LogParser::new();
        
        // Test with span and target (two colons)
        let (span, target, message) = parser.extract_span_target_message("start_headless_hive: hive::hive: enter initial_prompt=\"test\"");
        assert_eq!(span, Some("start_headless_hive".to_string()));
        assert_eq!(target, "hive::hive");
        assert_eq!(message, "enter initial_prompt=\"test\"");
        
        // Test with only target (one colon)
        let (span, target, message) = parser.extract_span_target_message("hive::config: Looking for config file");
        assert_eq!(span, None);
        assert_eq!(target, "hive::config");
        assert_eq!(message, "Looking for config file");
        
        // Test with no colons
        let (span, target, message) = parser.extract_span_target_message("Simple message");
        assert_eq!(span, None);
        assert_eq!(target, "unknown");
        assert_eq!(message, "Simple message");
    }

    #[test]
    fn test_extract_fields_from_message() {
        let parser = LogParser::new();
        let message = "enter initial_prompt=\"test\" prompt_length=4 success=true";
        
        let fields = parser.extract_fields_from_message(message);
        
        assert_eq!(fields.get("prompt_length").unwrap().as_i64().unwrap(), 4);
        assert_eq!(fields.get("success").unwrap().as_bool().unwrap(), true);
        assert_eq!(fields.get("initial_prompt").unwrap().as_str().unwrap(), "\"test\"");
    }

    #[test]
    fn test_parse_real_log_format() {
        let parser = LogParser::new();
        
        // Test actual log lines from the sample
        let lines = vec![
            "2025-06-04T18:52:02.648550Z DEBUG ThreadId(01) hive::config: Looking for config file at: \"/Users/silasmarvin/.config/hive/config.toml\"",
            "2025-06-04T18:52:02.651712Z  INFO ThreadId(01) start_headless_hive: hive::hive: enter initial_prompt=\"test\" prompt_length=4",
            "2025-06-04T18:52:02.652855Z  INFO ThreadId(10) actor_lifecycle: hive::actors: enter actor_id=\"context\""
        ];
        
        for line in lines {
            let entry = parser.parse_standard_log_line(line).unwrap();
            
            // All entries should have thread_id
            assert!(entry.thread_id.is_some());
            assert!(entry.thread_id.as_ref().unwrap().starts_with("ThreadId("));
            
            // Target should be properly extracted
            assert!(entry.target.contains("hive::"));
            
            // Message should not be empty
            assert!(!entry.message.is_empty());
        }
    }

    #[test]
    fn test_log_level_parsing() {
        let test_cases = vec![
            ("TRACE", LogLevel::Trace),
            ("DEBUG", LogLevel::Debug),
            ("INFO", LogLevel::Info),
            ("WARN", LogLevel::Warn),
            ("ERROR", LogLevel::Error),
            ("trace", LogLevel::Trace), // lowercase
            ("info", LogLevel::Info),
        ];
        
        for (input, expected) in test_cases {
            let level: LogLevel = input.parse().unwrap();
            assert_eq!(level, expected);
        }
        
        // Test invalid level
        assert!("INVALID".parse::<LogLevel>().is_err());
    }

    #[test]
    fn test_parse_log_content() {
        let log_content = r#"
2025-06-04T18:52:02.648550Z DEBUG ThreadId(01) hive::config: Looking for config file
2025-06-04T18:52:02.651712Z  INFO ThreadId(01) start_headless_hive: hive::hive: enter

2025-06-04T18:52:02.652855Z  INFO ThreadId(10) actor_lifecycle: hive::actors: enter
"#;
        
        let parser = LogParser::parse_log_content(log_content).unwrap();
        let entries = parser.entries();
        
        assert_eq!(entries.len(), 3); // Should parse 3 valid lines, skip empty line
        
        // Check first entry
        assert_eq!(entries[0].level, LogLevel::Debug);
        assert_eq!(entries[0].target, "hive::config");
        assert_eq!(entries[0].thread_id, Some("ThreadId(01)".to_string()));
        
        // Check entry with span
        assert_eq!(entries[1].span_name, Some("start_headless_hive".to_string()));
        assert_eq!(entries[1].target, "hive::hive");
    }

    #[test]
    fn test_filtering_methods() {
        let mut parser = LogParser::new();
        parser.entries = vec![
            LogEntry {
                timestamp: chrono::Utc::now(),
                level: LogLevel::Debug,
                target: "hive::agent".to_string(),
                span_name: Some("agent_run".to_string()),
                message: "Agent started".to_string(),
                fields: HashMap::new(),
                thread_id: Some("ThreadId(01)".to_string()),
                hive_message: None,
                inter_agent_message: None,
            },
            LogEntry {
                timestamp: chrono::Utc::now(),
                level: LogLevel::Info,
                target: "hive::config".to_string(),
                span_name: None,
                message: "Config loaded".to_string(),
                fields: HashMap::new(),
                thread_id: Some("ThreadId(02)".to_string()),
                hive_message: None,
                inter_agent_message: None,
            },
        ];
        
        // Test filtering by level
        let debug_entries = parser.entries_by_level(LogLevel::Debug);
        assert_eq!(debug_entries.len(), 1);
        assert_eq!(debug_entries[0].message, "Agent started");
        
        // Test filtering by target
        let agent_entries = parser.entries_by_target("agent");
        assert_eq!(agent_entries.len(), 1);
        
        // Test filtering by span
        let span_entries = parser.entries_by_span("agent_run");
        assert_eq!(span_entries.len(), 1);
        
        // Test filtering by message
        let message_entries = parser.entries_with_message("Config");
        assert_eq!(message_entries.len(), 1);
    }

    #[test]
    fn test_message_object_extraction() {
        let parser = LogParser::new();
        let mut fields = HashMap::new();
        
        // Test TaskCompleted message extraction
        fields.insert("message".to_string(), serde_json::Value::String(r#"{"TaskCompleted":{"summary":"File read successfully","success":true}}"#.to_string()));
        
        let (hive_message, inter_agent_message) = LogParser::extract_message_objects(&fields);
        
        assert!(hive_message.is_some());
        let hive_msg = hive_message.unwrap();
        assert!(hive_msg.as_object().unwrap().contains_key("TaskCompleted"));
        assert!(inter_agent_message.is_none());
    }

    #[test]
    fn test_sequence_detection() {
        let mut parser = LogParser::new();
        parser.entries = vec![
            LogEntry {
                timestamp: chrono::Utc::now(),
                level: LogLevel::Info,
                target: "test".to_string(),
                span_name: None,
                message: "Starting spawn_agent_and_assign_task".to_string(),
                fields: HashMap::new(),
                thread_id: None,
                hive_message: None,
                inter_agent_message: None,
            },
            LogEntry {
                timestamp: chrono::Utc::now(),
                level: LogLevel::Info,
                target: "test".to_string(),
                span_name: None,
                message: "Calling complete_tool_call".to_string(),
                fields: HashMap::new(),
                thread_id: None,
                hive_message: None,
                inter_agent_message: None,
            },
        ];

        assert!(parser.contains_sequence(&["spawn_agent_and_assign_task", "complete_tool_call"]));
        assert!(!parser.contains_sequence(&["complete_tool_call", "spawn_agent_and_assign_task"]));
    }

    #[test]
    fn test_edge_cases() {
        let parser = LogParser::new();
        
        // Test empty line
        assert!(parser.parse_standard_log_line("").is_err());
        
        // Test line with insufficient parts
        assert!(parser.parse_standard_log_line("2024-01-01T12:00:00Z DEBUG").is_err());
        
        // Test malformed timestamp (should still work with fallback)
        let line = "invalid-timestamp DEBUG ThreadId(01) hive::test: Test message";
        let entry = parser.parse_standard_log_line(line).unwrap();
        assert_eq!(entry.level, LogLevel::Debug);
        assert_eq!(entry.message, "Test message");
    }

    #[test]
    fn test_complex_message_parsing() {
        let parser = LogParser::new();
        let line = r#"2025-06-04T18:52:02.655846Z DEBUG ThreadId(13) actor_lifecycle:agent_run: hive::actors::agent: name="agent_received_internal_message" {"ToolsAvailable":[{"name":"spawn_agent_and_assign_task","description":"Spawn a new agent"}]} message_type="hive::actors::Message""#;
        
        let entry = parser.parse_standard_log_line(line).unwrap();
        assert_eq!(entry.level, LogLevel::Debug);
        assert_eq!(entry.target, "hive::actors::agent");
        assert_eq!(entry.span_name, Some("actor_lifecycle:agent_run".to_string()));
        assert!(entry.message.contains("ToolsAvailable"));
    }

    #[test]
    fn test_stats_generation() {
        let mut parser = LogParser::new();
        parser.entries = vec![
            LogEntry {
                timestamp: chrono::Utc::now(),
                level: LogLevel::Debug,
                target: "hive::agent".to_string(),
                span_name: Some("agent_run".to_string()),
                message: "Debug message".to_string(),
                fields: HashMap::new(),
                thread_id: Some("ThreadId(01)".to_string()),
                hive_message: None,
                inter_agent_message: None,
            },
            LogEntry {
                timestamp: chrono::Utc::now(),
                level: LogLevel::Info,
                target: "hive::config".to_string(),
                span_name: None,
                message: "Info message".to_string(),
                fields: HashMap::new(),
                thread_id: Some("ThreadId(02)".to_string()),
                hive_message: None,
                inter_agent_message: None,
            },
        ];
        
        let stats = parser.stats();
        assert_eq!(stats.total_entries, 2);
        assert_eq!(stats.debug_count, 1);
        assert_eq!(stats.info_count, 1);
        assert_eq!(stats.targets.get("hive::agent"), Some(&1));
        assert_eq!(stats.spans.get("agent_run"), Some(&1));
    }

    #[test]
    fn test_parse_json_log_line() {
        let parser = LogParser::new();
        let line = r#"{"timestamp":"2024-01-01T12:00:00.000000Z","level":"DEBUG","target":"hive::agent","message":"Agent started","fields":{"agent_id":"123"}}"#;
        
        let entry = parser.parse_json_log_line(line).unwrap();
        assert_eq!(entry.level, LogLevel::Debug);
        assert_eq!(entry.target, "hive::agent");
        assert_eq!(entry.message, "Agent started");
        assert_eq!(entry.fields.get("agent_id").unwrap().as_str().unwrap(), "123");
    }
}