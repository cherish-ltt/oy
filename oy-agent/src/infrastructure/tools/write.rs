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

        // Clean up
        let _ = std::fs::remove_file(&file_path);
    }
}
