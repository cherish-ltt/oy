use serde_json::Value;

use super::errors::AgentError;

pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn schema(&self) -> Value;
    fn execute(&self, args: Value) -> Result<String, AgentError>;
    fn get_system_prompt(&self) -> &str;
    fn clone_box(&self) -> Box<dyn Tool>;

    /// Default timeout in seconds for this tool.
    /// LLM can override via the optional `timeout` argument in tool calls.
    fn default_timeout(&self) -> u64 {
        150
    }
}
