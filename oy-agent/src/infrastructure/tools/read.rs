use serde_json::Value;

use crate::domain::errors::AgentError;
use crate::domain::tool::Tool;

pub struct ReadTool;

impl Tool for ReadTool {
    fn name(&self) -> &'static str {
        "Read"
    }

    fn description(&self) -> &'static str {
        "Read and return the contents of a file"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The path to the file to read"
                }
            },
            "required": ["file_path"]
        })
    }

    fn execute(&self, args: Value) -> Result<String, AgentError> {
        let file_path = args["file_path"]
            .as_str()
            .ok_or_else(|| AgentError::ToolExecutionError("Missing file_path".into()))?;
        match std::fs::read_to_string(file_path) {
            Ok(content) => Ok(content),
            Err(e) => Ok(format!("Error reading file: {}", e)),
        }
    }

    fn get_system_prompt(&self) -> &str {
        r#"
        - `read`: Read the file content; use `read` instead of `cat` or `sed`
        "#
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_read_tool_nonexistent_file() {
        let tool = ReadTool;
        let result = tool
            .execute(json!({"file_path": "/tmp/nonexistent_file_oy_test"}))
            .unwrap();
        assert!(
            result.contains("Error reading file"),
            "Expected error message, got: {}",
            result
        );
    }

    #[test]
    fn test_read_tool_schema() {
        let tool = ReadTool;
        let schema = tool.schema();
        assert!(schema["properties"]["file_path"].is_object());
        assert_eq!(schema["required"][0], "file_path");
    }
}
