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

/// Returns the JSON Schema for the `create_sub_agent` meta-tool.
///
/// This schema is registered in CommanderAgent's tool registry so that the LLM
/// can see `create_sub_agent` as an available tool and learn its parameters.
/// The schema is extracted to a standalone function so that it can be reused
/// or referenced without instantiating a `CreateSubAgentTool`.
pub fn create_sub_agent_schema() -> Value {
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

/// The unified meta-tool for creating and running sub-agents.
///
/// # Design Intent
///
/// `CreateSubAgentTool` is registered in **CommanderAgent's tool registry** so that
/// the LLM can see the `create_sub_agent` function's **JSON Schema** and learn its
/// parameters (`agent_type`, `task`, `context`, `timeout`).  The schema is what
/// drives the model to emit the correct tool-call.
///
/// # Why `execute()` Is Never Called in Production
///
/// In `Worker::acting()` (`oy-agent/src/infrastructure/agents/mod.rs`), there is a
/// **hard-coded special case** before the normal `Tool::execute` path:
///
/// ```ignore
/// if tool_call.function_name == "create_sub_agent" && self.sub_provider.is_some() {
///     tasks.push(self.spawn_sub_agent_task(tool_call));
/// } else {
///     tasks.push(self.spawn_regular_tool_task(tool_call));
/// }
/// ```
///
/// This means:
/// - **`CreateSubAgentTool::execute()` is NOT called** during normal CommanderAgent
///   operation — the `create_sub_agent` tool-call is intercepted and processed
///   asynchronously via `spawn_sub_agent_task` → `run_sub_agent_async`.
/// - The `execute()` method exists **only to satisfy the `Tool` trait contract**
///   (every registered tool must implement `execute`).  It is a synchronous fallback
///   that creates a one-shot `current_thread` runtime and blocks on it.
///
/// # Code Path Summary
///
/// | Scenario | Path | Notes |
/// |---|---|---|
/// | **Normal CommanderAgent** | `acting()` → `spawn_sub_agent_task()` → async `run_sub_agent` | `execute()` NOT called |
/// | **Direct `Tool::execute` call** (tests, future edge cases) | `execute()` → sync `run_sub_agent` | Creates ad-hoc runtime |
///
/// # Fallback Safety
///
/// The `execute()` implementation is kept **functionally identical** to the async
/// path so that it serves as a reliable fallback for any code path that calls
/// `Tool::execute` directly (e.g., unit tests, future refactoring, or non-Worker
/// environments).
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

    /// Sub-agents need longer time than regular tools. Default: 900s (15 min).
    /// LLM can override via the optional `timeout` parameter in tool-call arguments.
    fn default_timeout(&self) -> u64 {
        900 // 15 minutes
    }

    fn schema(&self) -> Value {
        // Delegates to the standalone [`create_sub_agent_schema()`] function.
        // See that function for the full JSON Schema definition.
        create_sub_agent_schema()
    }

    /// ⚠ NOTE: This method is NOT called during normal CommanderAgent operation.
    /// See the struct-level docs for the full design rationale.
    /// The implementation below is a synchronous fallback that creates a dedicated
    /// current_thread runtime — kept identical to the async path for consistency.
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

        Ok(crate::domain::sub_agent::format_sub_agent_output(&result))
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
