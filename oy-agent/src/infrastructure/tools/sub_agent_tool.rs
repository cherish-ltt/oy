use std::{str::FromStr, sync::Arc};

use serde_json::Value;
use tokio::sync::mpsc;

use crate::{
    Tool,
    domain::sub_agent::{SubAgentOutput, SubAgentType},
    infrastructure::agents::sub_agent_runner::{SubAgentEvent, run_sub_agent},
    infrastructure::tools::ToolRegistry,
};
use oy_ai::AiProvider;

/// The unified meta-tool for creating and running sub-agents.
///
/// CommanderAgent uses only this tool to delegate work to sub-agents.
/// The `agent_type` parameter selects which sub-agent to run:
/// - `planner` (≤25 rounds)
/// - `worker` (≤50 rounds)
/// - `reviewer` (≤15 rounds)
/// - `git_helper` (≤10 rounds)
pub struct CreateSubAgentTool {
    provider: Arc<dyn AiProvider + Send + Sync>,
    tool_registry: Arc<ToolRegistry>,
    /// Optional channel for UI progress updates
    progress_tx: Option<mpsc::UnboundedSender<SubAgentEvent>>,
}

impl CreateSubAgentTool {
    pub fn new(
        provider: Arc<dyn AiProvider + Send + Sync>,
        tool_registry: Arc<ToolRegistry>,
    ) -> Self {
        Self {
            provider,
            tool_registry,
            progress_tx: None,
        }
    }

    /// Attach a progress channel for UI updates.
    pub fn with_progress_channel(mut self, tx: mpsc::UnboundedSender<SubAgentEvent>) -> Self {
        self.progress_tx = Some(tx);
        self
    }
}

impl Tool for CreateSubAgentTool {
    fn name(&self) -> &'static str {
        "create_sub_agent"
    }

    fn description(&self) -> &'static str {
        "Create and run a sub-agent (planner/worker/reviewer/git_helper) for task execution"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_type": {
                    "type": "string",
                    "description": "Type of sub-agent to create: planner, worker, reviewer, or git_helper",
                    "enum": ["planner", "worker", "reviewer", "git_helper"]
                },
                "task": {
                    "type": "string",
                    "description": "The task description for the sub-agent"
                },
                "context": {
                    "type": "string",
                    "description": "Optional context (e.g., plan file path for worker)",
                    "default": ""
                }
            },
            "required": ["agent_type", "task"]
        })
    }

    fn execute(&self, args: Value) -> Result<String, crate::AgentError> {
        // Parse arguments
        let agent_type_str = args
            .get("agent_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                crate::AgentError::ToolExecutionError(
                    "Missing or invalid 'agent_type' argument".into(),
                )
            })?;

        let agent_type = SubAgentType::from_str(agent_type_str).map_err(|e| {
            crate::AgentError::ToolExecutionError(format!(
                "Unknown agent_type: {}. Expected: planner, worker, reviewer, or git_helper",
                e
            ))
        })?;

        let task = args.get("task").and_then(|v| v.as_str()).ok_or_else(|| {
            crate::AgentError::ToolExecutionError("Missing 'task' argument".into())
        })?;

        let context = args
            .get("context")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        // We are inside a tokio::spawn task. Use block_on to run the async sub-agent loop.
        let handle = tokio::runtime::Handle::current();
        let result: SubAgentOutput = handle.block_on(run_sub_agent(
            agent_type,
            task.to_string(),
            context,
            self.provider.clone(),
            self.tool_registry.clone(),
            self.progress_tx.clone(),
        ));

        // Format the output for CommanderAgent consumption
        if result.success {
            Ok(format!(
                "[{} 完成 - {} 轮]\n{}\n{}",
                agent_type,
                result.rounds_used,
                result.summary,
                match agent_type {
                    SubAgentType::Planner => "计划已创建，Worker 可引用此计划文件。",
                    SubAgentType::Worker => "代码已产出，Reviewer 可审查。",
                    SubAgentType::Reviewer => "审查完成，请检查 '通过: 是/否' 决定下一步。",
                    SubAgentType::GitHelper => "代码已提交。",
                }
            ))
        } else {
            let err = result.error.unwrap_or_else(|| "Unknown error".into());
            Ok(format!(
                "[{} 失败 - {} 轮]\n错误: {}",
                agent_type, result.rounds_used, err
            ))
        }
    }

    fn get_system_prompt(&self) -> &str {
        "## create_sub_agent\n\n\
         Create a sub-agent to execute a task.\n\n\
         Parameters:\n\
         - agent_type: \"planner\" | \"worker\" | \"reviewer\" | \"git_helper\"\n\
         - task: Description of the task for the sub-agent\n\
         - context: (Optional) Additional context (e.g., plan file path)\n\n\
         The sub-agent runs with its own system prompt and iteration limit, and returns the result."
    }

    fn clone_box(&self) -> Box<dyn Tool> {
        Box::new(Self {
            provider: self.provider.clone(),
            tool_registry: self.tool_registry.clone(),
            progress_tx: self.progress_tx.clone(),
        })
    }
}
