use oy_ai::{AiProvider, ChatMessage};

use crate::Agent;
use crate::domain::errors::AgentError;
use crate::infrastructure::tools::ToolRegistry;

pub struct Orchestrator {
    agent: Box<dyn Agent>,
    provider: Box<dyn AiProvider + Send + Sync>,
    tool_registry: ToolRegistry,
}

impl Orchestrator {
    pub fn new(
        agent: impl Agent + 'static,
        provider: impl AiProvider + 'static,
        tool_registry: ToolRegistry,
    ) -> Self {
        Self {
            agent: Box::new(agent),
            provider: Box::new(provider),
            tool_registry,
        }
    }

    /// Execute the agent loop: send prompt, process tool calls, return final text.
    ///
    /// The loop terminates when the AI responds without tool_calls, or when
    /// max_iterations is reached (safety guard against infinite loops from
    /// buggy tool outputs or model misbehaviour).
    pub async fn execute(&mut self, prompt: &str) -> Result<String, AgentError> {
        self.agent.push_message_back(ChatMessage::system(
            self.agent
                .get_system_prompt(&self.tool_registry.get_tools_system_prompt()),
        ));
        self.agent.push_message_back(ChatMessage::user(prompt));

        for _ in 0..self.agent.max_iterations() {
            let response = self
                .provider
                .chat(self.agent.messages(), &self.tool_registry.get_schemas())
                .await?;

            let has_tool_calls = response.tool_calls.as_ref().is_some_and(|c| !c.is_empty());
            self.agent.push_message_back(response.clone());

            if !has_tool_calls {
                return Ok(response.content.unwrap_or_default());
            }

            for tool_call in response.tool_calls.unwrap() {
                let tool = self
                    .tool_registry
                    .get(&tool_call.function_name)
                    .ok_or_else(|| {
                        AgentError::ToolExecutionError(format!(
                            "Unknown tool: {}",
                            tool_call.function_name
                        ))
                    })?;
                let result = tool.execute(tool_call.arguments.clone())?;
                self.agent
                    .push_message_back(ChatMessage::tool(result, tool_call.id));
            }
        }

        Err(AgentError::MaxIterationsReached)
    }
}
