use serde_json::Value;
use uuid::Uuid;

use crate::Tool;

/// A tool that generates UUIDs (v4 or v7) on demand.
pub struct UuidTool;

impl Tool for UuidTool {
    fn name(&self) -> &'static str {
        "uuid"
    }

    fn description(&self) -> &'static str {
        "Generate a UUID. Specify version: \"v4\" (random) or \"v7\" (time-ordered, default)."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "version": {
                    "type": "string",
                    "description": "UUID version: \"v4\" for random, \"v7\" for time-ordered (default)",
                    "enum": ["v4", "v7"],
                    "default": "v7"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Optional timeout in seconds (default: 150)",
                    "default": 150
                }
            }
        })
    }

    fn execute(&self, args: Value) -> Result<String, crate::AgentError> {
        let version = args.get("version").and_then(|v| v.as_str()).unwrap_or("v7");

        let uuid = match version.trim().to_lowercase().as_str() {
            "v4" => Uuid::new_v4().to_string(),
            "v7" => Uuid::now_v7().to_string(),
            _ => Uuid::now_v7().to_string(),
        };

        Ok(uuid)
    }

    fn get_system_prompt(&self) -> &str {
        "## uuid\n\n\
         Generate a UUID v4 (random) or v7 (time-ordered, default).\n\n\
         Parameters:\n\
         - version: \"v4\" or \"v7\" (default: \"v7\")\n\
         - timeout (optional, default: 150s): Timeout in seconds\n\n\
         Returns the generated UUID string."
    }

    fn clone_box(&self) -> Box<dyn Tool> {
        Box::new(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_uuid_tool_name() {
        let tool = UuidTool;
        assert_eq!(tool.name(), "uuid");
    }

    #[test]
    fn test_uuid_tool_description() {
        let tool = UuidTool;
        assert!(tool.description().contains("UUID"));
    }

    #[test]
    fn test_uuid_tool_schema() {
        let tool = UuidTool;
        let schema = tool.schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["version"].is_object());
    }

    #[test]
    fn test_uuid_tool_default_v7() {
        let tool = UuidTool;
        let args = serde_json::json!({});
        let result = tool.execute(args).unwrap();
        let uuid = Uuid::parse_str(&result).unwrap();
        // v7 UUID has version nibble = 7 at position 14 (0-indexed 12th hex char, i.e. bytes[6] >> 4)
        let version_byte = uuid.as_bytes()[6] >> 4;
        assert_eq!(
            version_byte, 7,
            "expected v7 UUID, got version {}",
            version_byte
        );
    }

    #[test]
    fn test_uuid_tool_explicit_v7() {
        let tool = UuidTool;
        let args = serde_json::json!({"version": "v7"});
        let result = tool.execute(args).unwrap();
        let uuid = Uuid::parse_str(&result).unwrap();
        let version_byte = uuid.as_bytes()[6] >> 4;
        assert_eq!(version_byte, 7);
    }

    #[test]
    fn test_uuid_tool_v4() {
        let tool = UuidTool;
        let args = serde_json::json!({"version": "v4"});
        let result = tool.execute(args).unwrap();
        let uuid = Uuid::parse_str(&result).unwrap();
        let version_byte = uuid.as_bytes()[6] >> 4;
        assert_eq!(
            version_byte, 4,
            "expected v4 UUID, got version {}",
            version_byte
        );
    }

    #[test]
    fn test_uuid_tool_invalid_version_falls_back() {
        let tool = UuidTool;
        let args = serde_json::json!({"version": "v99"});
        let result = tool.execute(args).unwrap();
        let uuid = Uuid::parse_str(&result).unwrap();
        let version_byte = uuid.as_bytes()[6] >> 4;
        assert_eq!(version_byte, 7);
    }

    #[test]
    fn test_uuid_tool_system_prompt_non_empty() {
        let tool = UuidTool;
        assert!(!tool.get_system_prompt().is_empty());
    }

    #[test]
    fn test_uuid_tool_clone() {
        let tool = UuidTool;
        let cloned = tool.clone_box();
        assert_eq!(cloned.name(), "uuid");
    }
}
