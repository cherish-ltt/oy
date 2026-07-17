use serde_json::Value;

use crate::domain::errors::AgentError;
use crate::domain::tool::Tool;

#[derive(Clone)]
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
                },
                "timeout": {
                    "type": "integer",
                    "description": "Optional timeout in seconds (default: 150)",
                    "default": 150
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

        // ── 自动创建父目录（防止因父目录不存在而写入失败）──
        let path = std::path::Path::new(file_path);
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| {
                AgentError::ToolExecutionError(format!(
                    "Failed to create parent directories: {}",
                    e
                ))
            })?;
        }

        match std::fs::write(file_path, content) {
            Ok(_) => Ok(format!("Successfully wrote to {}", file_path)),
            Err(e) => Err(AgentError::ToolExecutionError(format!(
                "Error writing file: {}",
                e
            ))),
        }
    }

    fn get_system_prompt(&self) -> &str {
        r#"
        - `write`: Create or overwrite a file, used only for new files or for a complete rewrite (default timeout: 150s, override via timeout param)
        "#
    }

    fn clone_box(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
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

    #[test]
    fn test_write_tool_creates_parent_dirs() {
        let tmp_dir = std::env::temp_dir().join("oy_test_write_parent_dirs");
        // 确保目录不存在
        let _ = std::fs::remove_dir_all(&tmp_dir);
        let file_path = tmp_dir.join("sub1").join("sub2").join("test.txt");
        let path_str = file_path.to_string_lossy().to_string();

        let result = WriteTool
            .execute(json!({
                "file_path": path_str,
                "content": "nested content"
            }))
            .unwrap();
        assert!(
            result.contains("Successfully wrote"),
            "Expected success message, got: {}",
            result
        );
        assert!(file_path.exists(), "File should exist at nested path");
        // 验证内容
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "nested content");
        // 清理
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
