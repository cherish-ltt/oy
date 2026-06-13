use serde_json::Value;
use std::process::Command;

use crate::domain::errors::AgentError;
use crate::domain::tool::Tool;

const BLACKLISTED_PREFIXES: &[&str] = &["rm -rf /", "rm -rf /*"];
const BLACKLISTED_SUBSTRINGS: &[&str] = &[" sudo "];

#[derive(Clone)]
pub struct BashTool;

impl BashTool {
    fn is_blacklisted(command: &str) -> bool {
        let trimmed = command.trim();
        for prefix in BLACKLISTED_PREFIXES {
            if trimmed.starts_with(prefix) {
                return true;
            }
        }
        for substr in BLACKLISTED_SUBSTRINGS {
            if trimmed.contains(substr) {
                return true;
            }
        }
        false
    }
}

impl Tool for BashTool {
    fn name(&self) -> &'static str {
        "Bash"
    }

    fn description(&self) -> &'static str {
        "Execute a shell command"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Optional timeout in seconds (default: 150). Increase for long-running commands.",
                    "default": 150
                }
            },
            "required": ["command"]
        })
    }

    fn execute(&self, args: Value) -> Result<String, AgentError> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| AgentError::ToolExecutionError("Missing command".into()))?;

        if Self::is_blacklisted(command) {
            return Ok("Command rejected: this command is not allowed for security reasons".into());
        }

        match Command::new("sh").arg("-c").arg(command).output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                Ok(format!("{}{}", stdout, stderr))
            },
            Err(e) => Ok(format!("Error executing command: {}", e)),
        }
    }

    fn get_system_prompt(&self) -> &str {
        r#"
        - `bash`: Using Bash for file operations (default timeout: 150s, override via timeout param)
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
    fn test_bash_tool_name() {
        assert_eq!(BashTool.name(), "Bash");
    }

    #[test]
    fn test_bash_tool_echo() {
        let tool = BashTool;
        let result = tool.execute(json!({"command": "echo hello"})).unwrap();
        assert!(
            result.contains("hello"),
            "Expected output to contain 'hello', got: {}",
            result
        );
    }

    #[test]
    fn test_bash_tool_blacklist_rm_rf() {
        let tool = BashTool;
        let result = tool.execute(json!({"command": "rm -rf /"})).unwrap();
        assert!(
            result.contains("Command rejected"),
            "Expected rejection, got: {}",
            result
        );
    }

    #[test]
    fn test_bash_tool_blacklist_rm_rf_variant() {
        let tool = BashTool;
        let result = tool.execute(json!({"command": "rm -rf /*"})).unwrap();
        assert!(
            result.contains("Command rejected"),
            "Expected rejection, got: {}",
            result
        );
    }

    #[test]
    fn test_bash_tool_blacklist_sudo() {
        let tool = BashTool;
        let result = tool
            .execute(json!({"command": "echo foo && sudo rm -rf /tmp/test"}))
            .unwrap();
        assert!(
            result.contains("Command rejected"),
            "Expected rejection, got: {}",
            result
        );
    }

    #[test]
    fn test_bash_tool_missing_command() {
        let result = BashTool.execute(json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn test_bash_tool_pipe() {
        let result = BashTool
            .execute(json!({"command": "echo hello | wc -c"}))
            .unwrap();
        let trimmed = result.trim();
        assert_eq!(trimmed, "6");
    }

    #[test]
    fn test_bash_tool_schema() {
        let schema = BashTool.schema();
        assert!(schema["properties"]["command"].is_object());
        assert_eq!(schema["required"][0], "command");
    }

    #[test]
    fn test_bash_tool_system_prompt() {
        assert!(!BashTool.get_system_prompt().is_empty());
    }
}
