use std::collections::HashMap;

use serde_json::Value;

use crate::Tool;

pub mod bash;
pub mod edit;
pub mod read;
pub mod sub_agent_tool;
pub mod uuid_tool;
pub mod write;

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool + Send + Sync>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: impl Tool + 'static) {
        self.tools.insert(tool.name().to_string(), Box::new(tool));
    }

    pub(crate) fn get_clone(&self, name: &str) -> Option<Box<dyn Tool>> {
        self.tools.get(name).map(|tool| tool.clone_box())
    }

    pub fn get_schemas(&self) -> Vec<Value> {
        self.tools
            .values()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name(),
                        "description": t.description(),
                        "parameters": t.schema(),
                    }
                })
            })
            .collect()
    }

    pub fn get_tools_system_prompt(&self) -> String {
        let mut result = String::new();
        self.tools.iter().for_each(|tool| {
            result.push_str(tool.1.get_system_prompt());
        });
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::tools::{
        bash::BashTool, edit::EditTool, read::ReadTool, write::WriteTool,
    };

    fn sample_registry() -> ToolRegistry {
        let mut r = ToolRegistry::new();
        r.register(ReadTool);
        r.register(WriteTool);
        r.register(BashTool);
        r.register(EditTool);
        r
    }

    #[test]
    fn test_registry_new_is_empty() {
        let r = ToolRegistry::new();
        assert!(r.get_clone("Read").is_none());
    }

    #[test]
    fn test_register_and_get() {
        let mut r = ToolRegistry::new();
        r.register(ReadTool);
        let tool = r.get_clone("Read");
        assert!(tool.is_some());
        assert_eq!(tool.unwrap().name(), "Read");
    }

    #[test]
    fn test_get_nonexistent() {
        let r = sample_registry();
        assert!(r.get_clone("Nonexistent").is_none());
    }

    #[test]
    fn test_register_overwrites() {
        let mut r = ToolRegistry::new();
        r.register(ReadTool);
        r.register(ReadTool); // same name, overwrites
        assert!(r.get_clone("Read").is_some());
    }

    #[test]
    fn test_get_schemas_count() {
        let r = sample_registry();
        let schemas = r.get_schemas();
        assert_eq!(schemas.len(), 4);
    }

    #[test]
    fn test_schema_structure() {
        let r = sample_registry();
        let schemas = r.get_schemas();
        for s in &schemas {
            assert_eq!(s["type"], "function");
            assert!(s["function"]["name"].as_str().unwrap().len() > 0);
            assert!(s["function"]["description"].as_str().unwrap().len() > 0);
            assert!(s["function"]["parameters"].is_object());
        }
    }

    #[test]
    fn test_system_prompt_contains_tool_names() {
        let r = sample_registry();
        let prompt = r.get_tools_system_prompt();
        assert!(prompt.contains("read"));
        assert!(prompt.contains("write"));
        assert!(prompt.contains("bash"));
        assert!(prompt.contains("edit"));
    }

    #[test]
    fn test_default_trait() {
        let r = ToolRegistry::default();
        assert!(r.get_clone("Read").is_none());
    }
}
