use serde_json::Value;

use crate::domain::errors::AgentError;
use crate::domain::tool::Tool;

pub struct WriteTool;

impl Tool for WriteTool {
    fn name(&self) -> &'static str {
        "Write"
    }

    fn description(&self) -> &'static str {
        "Write content to a file"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The path of the file to write to"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["file_path", "content"]
        })
    }

    fn execute(&self, args: Value) -> Result<String, AgentError> {
        let file_path = args["file_path"]
            .as_str()
            .ok_or_else(|| AgentError::ToolExecutionError("Missing file_path".into()))?;
        let content = args["content"]
            .as_str()
            .ok_or_else(|| AgentError::ToolExecutionError("Missing content".into()))?;
        match std::fs::write(file_path, content) {
            Ok(_) => Ok(format!("Successfully wrote to {}", file_path)),
            Err(e) => Ok(format!("Error writing file: {}", e)),
        }
    }

    fn get_system_prompt(&self) -> &str {
        r#"
        - `write`: Create or overwrite a file, used only for new files or for a complete rewrite
        "#
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_write_tool_name() {
        assert_eq!(WriteTool.name(), "Write");
    }

    #[test]
    fn test_write_tool_success() {
        let tmp_dir = std::env::temp_dir();
        let file_path = tmp_dir.join("oy_test_write.txt");
        let path_str = file_path.to_string_lossy().to_string();

        let tool = WriteTool;
        let result = tool
            .execute(json!({
                "file_path": path_str,
                "content": "test content"
            }))
            .unwrap();
        assert!(
            result.contains("Successfully wrote to"),
            "Expected success message, got: {}",
            result
        );
        // verify content
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "test content");
        let _ = std::fs::remove_file(&file_path);
    }

    #[test]
    fn test_write_tool_missing_file_path() {
        let result = WriteTool.execute(json!({"content": "hi"}));
        assert!(result.is_err());
    }

    #[test]
    fn test_write_tool_missing_content() {
        let result = WriteTool.execute(json!({"file_path": "/tmp/x.txt"}));
        assert!(result.is_err());
    }

    #[test]
    fn test_write_tool_schema() {
        let schema = WriteTool.schema();
        assert!(schema["properties"]["file_path"].is_object());
        assert!(schema["properties"]["content"].is_object());
    }

    #[test]
    fn test_write_tool_system_prompt() {
        assert!(!WriteTool.get_system_prompt().is_empty());
    }

    #[test]
    fn test_write_tool_flat_path() {
        let tmp = std::env::temp_dir().join("oy_test_write_flat.txt");
        let path_str = tmp.to_string_lossy().to_string();
        let result = WriteTool
            .execute(json!({
                "file_path": path_str,
                "content": "flat"
            }))
            .unwrap();
        assert!(result.contains("Successfully wrote"));
        assert!(tmp.exists());
        let _ = std::fs::remove_file(&tmp);
    }
}
