use std::collections::HashMap;

use serde_json::Value;

use crate::Tool;

pub mod bash;
pub mod edit;
pub mod read;
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

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref() as &dyn Tool)
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
