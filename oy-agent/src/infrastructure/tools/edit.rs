use serde_json::Value;
use std::fs;
use std::path::Path;

use crate::domain::errors::AgentError;
use crate::domain::tool::Tool;

#[derive(Clone)]
pub struct EditTool;

impl EditTool {
    /// 检查文件路径是否在允许范围内（这里仅做简单安全检查，避免明显的系统文件）
    fn is_safe_path(path: &str) -> bool {
        // 拒绝空路径或明显指向系统关键目录的路径（可根据需要扩展）
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return false;
        }
        // 简单黑名单：禁止直接编辑 /etc、/dev、/sys 等关键系统目录
        let dangerous_prefixes = &["/etc/", "/dev/", "/sys/", "/proc/"];
        for prefix in dangerous_prefixes {
            if trimmed.starts_with(prefix) {
                return false;
            }
        }
        true
    }

    /// 执行文本替换：只替换第一次出现（与 `sed` 的默认行为一致）
    fn replace_first(content: &str, old: &str, new: &str) -> String {
        if let Some(pos) = content.find(old) {
            let mut result =
                String::with_capacity(content.len() + new.len().saturating_sub(old.len()));
            result.push_str(&content[..pos]);
            result.push_str(new);
            result.push_str(&content[pos + old.len()..]);
            result
        } else {
            content.to_string()
        }
    }

    /// Read file, replace first occurrence of old_text with new_text, write back.
    /// Returns a human-readable status message.
    fn apply_edit_to_file(
        path: &Path,
        old_text: &str,
        new_text: &str,
    ) -> Result<String, AgentError> {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                return Err(AgentError::ToolExecutionError(format!(
                    "Failed to read file: {}",
                    e
                )));
            },
        };

        if !content.contains(old_text) {
            return Ok(
                "No changes made: the specified text was not found in the file.".to_string(),
            );
        }

        let new_content = Self::replace_first(&content, old_text, new_text);

        match fs::write(path, new_content) {
            Ok(_) => Ok(format!(
                "Successfully replaced '{}' with '{}' in {}",
                old_text,
                new_text,
                path.display()
            )),
            Err(e) => Err(AgentError::ToolExecutionError(format!(
                "Failed to write file: {}",
                e
            ))),
        }
    }
}

impl Tool for EditTool {
    fn name(&self) -> &'static str {
        "Edit"
    }

    fn description(&self) -> &'static str {
        "Edit a file by replacing exact text (first occurrence only)."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file to edit"
                },
                "old_text": {
                    "type": "string",
                    "description": "The exact text to be replaced"
                },
                "new_text": {
                    "type": "string",
                    "description": "The new text to insert"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Optional timeout in seconds (default: 150)",
                    "default": 150
                }
            },
            "required": ["file_path", "old_text", "new_text"]
        })
    }

    fn execute(&self, args: Value) -> Result<String, AgentError> {
        // 1. 提取参数
        let file_path = args["file_path"]
            .as_str()
            .ok_or_else(|| AgentError::ToolExecutionError("Missing file_path".into()))?;
        let old_text = args["old_text"]
            .as_str()
            .ok_or_else(|| AgentError::ToolExecutionError("Missing old_text".into()))?;
        let new_text = args["new_text"]
            .as_str()
            .ok_or_else(|| AgentError::ToolExecutionError("Missing new_text".into()))?;

        // 2a. 校验 old_text 不为空（空字符串会导致意外的全局替换行为）
        if old_text.is_empty() {
            return Err(AgentError::ToolExecutionError(
                "old_text must not be empty".into(),
            ));
        }

        // 2b. 安全检查
        if !Self::is_safe_path(file_path) {
            return Ok("Edit rejected: file path is not allowed for security reasons".into());
        }

        let path = Path::new(file_path);
        if !path.exists() {
            return Ok(format!("File not found: {}", file_path));
        }

        // 3-6. Apply edit: read, replace, write back
        Self::apply_edit_to_file(path, old_text, new_text)
    }

    fn get_system_prompt(&self) -> &str {
        r#"
        - `edit`: Precise file editing through precise text replacement; Keep the old text block small and unique; Combine multiple edits in the same file into one edit call (default timeout: 150s, override via timeout param)
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
    fn test_edit_tool_name() {
        assert_eq!(EditTool.name(), "Edit");
    }

    #[test]
    fn test_edit_tool_safe_path_rejection() {
        let tool = EditTool;
        let result = tool
            .execute(json!({
                "file_path": "/etc/passwd",
                "old_text": "root",
                "new_text": "admin"
            }))
            .unwrap();
        assert!(result.contains("not allowed for security reasons"));
    }

    #[test]
    fn test_edit_tool_file_not_found() {
        let tool = EditTool;
        let result = tool
            .execute(json!({
                "file_path": "/nonexistent/path/file.txt",
                "old_text": "foo",
                "new_text": "bar"
            }))
            .unwrap();
        assert!(result.contains("File not found"));
    }

    #[test]
    fn test_edit_tool_missing_args() {
        let result = EditTool.execute(json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn test_edit_tool_schema() {
        let schema = EditTool.schema();
        assert!(schema["properties"]["file_path"].is_object());
        assert!(schema["properties"]["old_text"].is_object());
        assert!(schema["properties"]["new_text"].is_object());
    }

    #[test]
    fn test_edit_tool_success() {
        let tmp = std::env::temp_dir().join("oy_edit_test.txt");
        std::fs::write(&tmp, "hello world").unwrap();
        let result = EditTool
            .execute(json!({
                "file_path": tmp.to_string_lossy(),
                "old_text": "hello",
                "new_text": "hi"
            }))
            .unwrap();
        assert!(result.contains("Successfully replaced"));
        let content = std::fs::read_to_string(&tmp).unwrap();
        assert_eq!(content, "hi world");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_edit_tool_no_changes() {
        let tmp = std::env::temp_dir().join("oy_edit_nochange.txt");
        std::fs::write(&tmp, "hello world").unwrap();
        let result = EditTool
            .execute(json!({
                "file_path": tmp.to_string_lossy(),
                "old_text": "nonexistent",
                "new_text": "anything"
            }))
            .unwrap();
        assert!(result.contains("No changes made"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_edit_tool_system_prompt() {
        assert!(!EditTool.get_system_prompt().is_empty());
    }

    #[test]
    fn test_edit_tool_empty_old_text() {
        // empty old_text should be rejected
        let result = EditTool.execute(json!({
            "file_path": "/some/path.txt",
            "old_text": "",
            "new_text": "anything"
        }));
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("must not be empty"), "Error message: {}", msg);
    }

    #[test]
    fn test_edit_tool_empty_new_text() {
        // empty new_text is allowed (deletion)
        let tmp = std::env::temp_dir().join("oy_edit_empty_new.txt");
        std::fs::write(&tmp, "hello world").unwrap();
        let result = EditTool
            .execute(json!({
                "file_path": tmp.to_string_lossy(),
                "old_text": "hello ",
                "new_text": ""
            }))
            .unwrap();
        assert!(result.contains("Successfully replaced"));
        let content = std::fs::read_to_string(&tmp).unwrap();
        assert_eq!(content, "world");
        let _ = std::fs::remove_file(&tmp);
    }
}
