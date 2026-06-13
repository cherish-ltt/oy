use std::{str::FromStr, sync::Arc};

use serde_json::Value;
use tokio::sync::mpsc;

use crate::{
    Tool,
    domain::sub_agent::{SubAgentOutput, SubAgentType},
    infrastructure::agents::sub_agent_runner::{SubAgentConfig, SubAgentEvent, run_sub_agent},
    infrastructure::tools::ToolRegistry,
};
use oy_ai::AiProvider;

/// The unified meta-tool for creating and running sub-agents.
///
/// CommanderAgent uses only this tool to delegate work to sub-agents.
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

    /// Parse and validate the tool call arguments.
    fn parse_args(
        &self,
        args: &Value,
    ) -> Result<(SubAgentType, String, Option<String>), crate::AgentError> {
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

        Ok((agent_type, task.to_string(), context))
    }

    /// Create a new current_thread runtime for executing the sub-agent.
    fn create_runtime(&self) -> Result<tokio::runtime::Runtime, crate::AgentError> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| {
                crate::AgentError::ToolExecutionError(format!(
                    "Failed to create sub-agent runtime: {}",
                    e
                ))
            })
    }
}

impl Tool for CreateSubAgentTool {
    fn name(&self) -> &'static str {
        "create_sub_agent"
    }

    fn description(&self) -> &'static str {
        "Create and run a sub-agent (planner/worker/reviewer/git_helper) for task execution"
    }

    fn default_timeout(&self) -> u64 {
        900 // 15 minutes
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_type": {
                    "type": "string",
                    "description": "Type of sub-agent to create: planner, worker, reviewer, or git_helper (commit/issue/PR)",
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
                },
                "timeout": {
                    "type": "integer",
                    "description": "Optional timeout in seconds (default: 900). Sub-agents need longer time.",
                    "default": 900
                }
            },
            "required": ["agent_type", "task"]
        })
    }

    fn execute(&self, args: Value) -> Result<String, crate::AgentError> {
        // Parse arguments
        let (agent_type, task, context) = self.parse_args(&args)?;

        // Create a dedicated current_thread runtime for this sub-agent execution.
        // Cannot use Handle::current().block_on() here because we may be inside a
        // nested tokio::spawn where Handle::current() is unavailable or panics.
        let rt = self.create_runtime()?;

        let result: SubAgentOutput = rt.block_on(run_sub_agent(SubAgentConfig {
            agent_type,
            task,
            context,
            provider: self.provider.clone(),
            tool_registry: self.tool_registry.clone(),
            progress_tx: self.progress_tx.clone(),
        }));

        Ok(format_sub_agent_result(&result, &agent_type))
    }

    fn get_system_prompt(&self) -> &str {
        "## create_sub_agent\n\
         Create a sub-agent to execute a task.\n\
         Default timeout: 900s (15 min). Increase via `timeout` parameter for complex tasks.\n\
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

/// Format the sub-agent output for CommanderAgent consumption.
fn format_sub_agent_result(result: &SubAgentOutput, agent_type: &SubAgentType) -> String {
    if result.success {
        format!(
            "[{} 完成 - {} 轮]\n{}\n{}",
            agent_type,
            result.rounds_used,
            result.summary,
            match agent_type {
                SubAgentType::Planner => "计划已创建，Worker 可引用此计划文件。",
                SubAgentType::Worker => "代码已产出，Reviewer 可审查。",
                SubAgentType::Reviewer => "审查完成，请检查 '通过: 是/否' 决定下一步。",
                SubAgentType::GitHelper => "操作已完成（commit/issue/PR）。",
            }
        )
    } else {
        let err = result.error.as_deref().unwrap_or("Unknown error");
        format!(
            "[{} 失败 - {} 轮]\n错误: {}",
            agent_type, result.rounds_used, err
        )
    }
}
