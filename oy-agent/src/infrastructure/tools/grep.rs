use serde_json::Value;

use crate::Tool;

/// A tool that searches file contents via the system `grep` command.
///
/// Only available on Unix platforms (macOS, Linux). On Windows, the tool still
/// exists but returns a helpful message directing users to alternatives.
pub struct GrepTool;

impl Tool for GrepTool {
    fn name(&self) -> &'static str {
        "grep"
    }

    fn description(&self) -> &'static str {
        "Search file contents using system grep. Supports regex patterns, path filtering, and extension filtering."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The regex pattern to search for (required)"
                },
                "path": {
                    "type": "string",
                    "description": "The directory path to search in (default: current directory)",
                    "default": "."
                },
                "extension": {
                    "type": "string",
                    "description": "Filter by file extension (e.g. \"rs\" for Rust files)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 50)",
                    "default": 50
                }
            },
            "required": ["pattern"]
        })
    }

    fn execute(&self, args: Value) -> Result<String, crate::AgentError> {
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| crate::AgentError::ToolExecutionError("Missing pattern".into()))?;

        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let extension = args.get("extension").and_then(|v| v.as_str());
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as usize;

        self.inner_execute(pattern, path, extension, max_results)
    }

    fn get_system_prompt(&self) -> &str {
        "## grep\n\n\
         Search file contents using the system `grep` command. Supports regex patterns, \
         path filtering, and extension filtering.\n\n\
         Parameters:\n\
         - pattern (required): The regex pattern to search for\n\
         - path (optional, default: \".\"): The directory path to search in\n\
         - extension (optional): Filter by file extension (e.g. \"rs\" for Rust files)\n\
         - max_results (optional, default: 50): Maximum number of results to return\n\n\
         Returns matching lines with file paths and line numbers."
    }

    fn clone_box(&self) -> Box<dyn Tool> {
        Box::new(Self)
    }
}

// Unix implementation: delegates to system `grep` command.
#[cfg(not(windows))]
impl GrepTool {
    fn inner_execute(
        &self,
        pattern: &str,
        path: &str,
        extension: Option<&str>,
        max_results: usize,
    ) -> Result<String, crate::AgentError> {
        let output = self.run_grep(pattern, path, extension)?;

        // grep exit code: 0 = matches found, 1 = no matches, >1 = error
        if !output.status.success() && output.status.code() != Some(1) {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::AgentError::ToolExecutionError(format!(
                "grep failed: {}",
                stderr.trim()
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.is_empty() {
            return Ok("No matches found.".to_string());
        }

        Ok(format_grep_output(&stdout, max_results))
    }

    /// Run the grep command with the given parameters.
    fn run_grep(
        &self,
        pattern: &str,
        path: &str,
        extension: Option<&str>,
    ) -> Result<std::process::Output, crate::AgentError> {
        let mut cmd = std::process::Command::new("grep");
        cmd.arg("-rn").arg("--binary-files=without-match");

        if let Some(ext) = extension {
            cmd.arg("--include").arg(format!("*.{ext}"));
        }

        cmd.arg(pattern).arg(path);

        cmd.output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                crate::AgentError::ToolExecutionError(
                    "grep command not found on this system".into(),
                )
            } else {
                crate::AgentError::ToolExecutionError(format!("Failed to execute grep: {e}"))
            }
        })
    }
}

// Windows stub: tool exists in schema but returns a helpful message.
#[cfg(windows)]
impl GrepTool {
    fn inner_execute(
        &self,
        _pattern: &str,
        _path: &str,
        _extension: Option<&str>,
        _max_results: usize,
    ) -> Result<String, crate::AgentError> {
        Ok(
            "Grep tool requires the system `grep` command which is not available on Windows. \
            Use the Bash tool with `findstr /s /n \"pattern\" *.*` as an alternative, \
            or install Git Bash / WSL for full grep support."
                .to_string(),
        )
    }
}

/// Format grep output, truncating to max_results lines.
fn format_grep_output(stdout: &str, max_results: usize) -> String {
    let lines: Vec<&str> = stdout.lines().collect();
    let total = lines.len();

    if total == 0 {
        return "No matches found.".to_string();
    }

    let mut result = String::new();
    let count = total.min(max_results);

    for line in lines.iter().take(count) {
        result.push_str(line);
        result.push('\n');
    }

    if total > max_results {
        result.push_str(&format!(
            "\n... and {} more result(s) (showing {} of {})",
            total - max_results,
            max_results,
            total
        ));
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grep_tool_name() {
        let tool = GrepTool;
        assert_eq!(tool.name(), "grep");
    }

    #[test]
    fn test_grep_tool_schema() {
        let tool = GrepTool;
        let schema = tool.schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["required"].is_array());
        assert_eq!(schema["required"][0], "pattern");
        assert!(schema["properties"]["pattern"].is_object());
        assert!(schema["properties"]["path"].is_object());
        assert!(schema["properties"]["extension"].is_object());
        assert!(schema["properties"]["max_results"].is_object());
    }

    #[test]
    fn test_grep_tool_system_prompt() {
        let tool = GrepTool;
        let prompt = tool.get_system_prompt();
        assert!(!prompt.is_empty());
        assert!(prompt.contains("grep"));
        assert!(prompt.contains("pattern"));
        assert!(prompt.contains("max_results"));
    }

    #[test]
    fn test_grep_tool_clone() {
        let tool = GrepTool;
        let cloned = tool.clone_box();
        assert_eq!(cloned.name(), "grep");
    }

    #[test]
    fn test_grep_tool_missing_pattern() {
        let tool = GrepTool;
        let args = serde_json::json!({});
        let result = tool.execute(args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Missing pattern"));
    }

    #[cfg(not(windows))]
    #[test]
    fn test_grep_tool_default_path() {
        let tool = GrepTool;
        let args = serde_json::json!({"pattern": "GrepTool"});
        let result = tool.execute(args);
        // This should find the GrepTool struct in the current project source
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("GrepTool"));
    }

    #[cfg(not(windows))]
    #[test]
    fn test_grep_tool_self_search() {
        let tool = GrepTool;
        let args = serde_json::json!({"pattern": "struct GrepTool", "path": "."});
        let result = tool.execute(args);
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("struct GrepTool"));
    }

    #[cfg(not(windows))]
    #[test]
    fn test_grep_tool_extension_filter() {
        let tool = GrepTool;
        let args = serde_json::json!({
            "pattern": "GrepTool",
            "extension": "rs",
            "path": "."
        });
        let result = tool.execute(args);
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("GrepTool"));
    }

    #[cfg(not(windows))]
    #[test]
    fn test_grep_tool_max_results() {
        let tool = GrepTool;
        let args = serde_json::json!({
            "pattern": "fn",
            "max_results": 3,
            "path": "."
        });
        let result = tool.execute(args);
        assert!(result.is_ok());
        let output = result.unwrap();
        // Should have at most 3 results (or show truncated message)
        let lines: Vec<&str> = output.lines().collect();
        let non_empty: Vec<&str> = lines.iter().filter(|l| !l.is_empty()).copied().collect();
        assert!(non_empty.len() <= 5); // 3 matches + possible truncation message + extra
    }

    #[cfg(not(windows))]
    #[test]
    fn test_grep_tool_no_matches() {
        // Create a temp directory with a known file to avoid self-matching
        let dir = std::env::temp_dir().join(format!("oy_grep_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test.txt"), b"hello world").unwrap();

        let tool = GrepTool;
        let args = serde_json::json!({
            "pattern": "XYZZYX_NONEXISTENT",
            "path": dir.to_str().unwrap()
        });
        let result = tool.execute(args);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("No matches found"));
    }
}
