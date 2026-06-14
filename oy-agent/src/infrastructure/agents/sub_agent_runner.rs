use std::sync::Arc;

use oy_ai::{AiProvider, ChatMessage, ToolCall};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    domain::sub_agent::{SubAgentOutput, SubAgentStatus, SubAgentType},
    infrastructure::{persistence::save_session, tools::ToolRegistry},
};
use log::warn;

/// Events emitted during sub-agent execution for UI progress reporting.
#[derive(Debug, Clone)]
pub enum SubAgentEvent {
    Status(SubAgentStatus),
    RoundComplete {
        round: u32,
        max: u32,
        summary: String,
    },
    Output(String),
}

/// Configuration for running a sub-agent.
///
/// Aggregates all parameters to keep the public API concise and
/// avoid clippy::too_many_arguments warnings.
#[derive(Clone)]
pub struct SubAgentConfig {
    pub agent_type: SubAgentType,
    pub task: String,
    pub context: Option<String>,
    pub provider: Arc<dyn AiProvider + Send + Sync>,
    pub tool_registry: Arc<ToolRegistry>,
    pub progress_tx: Option<mpsc::UnboundedSender<SubAgentEvent>>,
}

/// Run a sub-agent with bounded iterations.
///
/// This is a self-contained async function that:
/// 1. Creates a fresh message history (system prompt + task)
/// 2. Runs an LLM loop with tool access
/// 3. Enforces iteration limits
/// 4. Returns the final output or error
pub async fn run_sub_agent(config: SubAgentConfig) -> SubAgentOutput {
    let uuid = Uuid::now_v7();
    let max_rounds = config.agent_type.max_rounds();
    let mut messages;

    // 1. Send progress: Pending
    if let Some(ref tx) = config.progress_tx {
        let _ = tx.send(SubAgentEvent::Status(SubAgentStatus::Pending));
    }

    // 2. Build messages
    messages = build_sub_agent_messages(
        &config.agent_type,
        &config.task,
        &config.context,
        &config.tool_registry,
    );

    // 3. Bounded LLM loop (extracted to separate function)
    run_sub_agent_loop(uuid, &config, max_rounds, &mut messages).await
}

async fn run_sub_agent_loop(
    uuid: Uuid,
    config: &SubAgentConfig,
    max_rounds: u32,
    messages: &mut Vec<ChatMessage>,
) -> SubAgentOutput {
    for round in 0..max_rounds {
        report_sub_agent_progress(&config.progress_tx, round + 1, max_rounds);
        let response = match call_sub_agent_llm(config, uuid, round + 1, messages).await {
            Ok(r) => r,
            Err(e) => return e,
        };
        let has_content = response.content.as_ref().is_some_and(|c| !c.is_empty());
        let has_tool_calls = response.tool_calls.as_ref().is_some_and(|c| !c.is_empty());
        messages.push(response.clone());
        if !has_tool_calls && has_content {
            return complete_sub_agent_success(config, uuid, response, round + 1, messages);
        }
        if !has_tool_calls {
            continue;
        }
        execute_sub_agent_tool_calls(&response, &config.tool_registry, messages);
        report_sub_agent_round_complete(&config.progress_tx, &response, round + 1, max_rounds);
    }
    build_sub_agent_max_rounds_output(config, uuid, max_rounds, messages)
}

// ---------------------------------------------------------------------------
// Helper functions — extracted to keep each function under the
// clippy::too_many_lines threshold (≤ 30 lines).
// ---------------------------------------------------------------------------

/// Call the LLM and handle errors, returning early on failure.
async fn call_sub_agent_llm(
    config: &SubAgentConfig,
    uuid: Uuid,
    round: u32,
    messages: &[ChatMessage],
) -> Result<ChatMessage, SubAgentOutput> {
    match config
        .provider
        .chat(messages, &config.tool_registry.get_schemas())
        .await
    {
        Ok(resp) => Ok(resp),
        Err(e) => {
            let err_msg = format!("AI error at round {}: {}", round, e);
            report_sub_agent_failure(&config.progress_tx, &err_msg);
            save_sub_agent_session(uuid, messages);
            Err(SubAgentOutput {
                agent_type: config.agent_type,
                success: false,
                summary: String::new(),
                rounds_used: round,
                error: Some(err_msg),
            })
        },
    }
}

/// Complete a sub-agent successfully: build output, report, save session.
fn complete_sub_agent_success(
    config: &SubAgentConfig,
    uuid: Uuid,
    response: ChatMessage,
    round: u32,
    messages: &[ChatMessage],
) -> SubAgentOutput {
    let output = build_sub_agent_success_output(&config.agent_type, response, round);
    report_sub_agent_success(&config.progress_tx, &output, &output.summary);
    save_sub_agent_session(uuid, messages);
    output
}

/// Build output for max rounds reached and report failure.
fn build_sub_agent_max_rounds_output(
    config: &SubAgentConfig,
    uuid: Uuid,
    max_rounds: u32,
    messages: &[ChatMessage],
) -> SubAgentOutput {
    let err_msg = format!(
        "Max rounds ({}) reached without completing the task",
        max_rounds
    );
    report_sub_agent_failure(&config.progress_tx, &err_msg);
    save_sub_agent_session(uuid, messages);
    SubAgentOutput {
        agent_type: config.agent_type,
        success: false,
        summary: String::new(),
        rounds_used: max_rounds,
        error: Some(err_msg),
    }
}

fn report_sub_agent_progress(
    progress_tx: &Option<mpsc::UnboundedSender<SubAgentEvent>>,
    round: u32,
    max_rounds: u32,
) {
    if let Some(tx) = progress_tx {
        let _ = tx.send(SubAgentEvent::Status(SubAgentStatus::Running {
            round,
            max_rounds,
        }));
    }
}

fn report_sub_agent_failure(
    progress_tx: &Option<mpsc::UnboundedSender<SubAgentEvent>>,
    err_msg: &str,
) {
    if let Some(tx) = progress_tx {
        let _ = tx.send(SubAgentEvent::Status(SubAgentStatus::Failed(
            err_msg.to_string(),
        )));
    }
}

fn report_sub_agent_success(
    progress_tx: &Option<mpsc::UnboundedSender<SubAgentEvent>>,
    output: &SubAgentOutput,
    summary: &str,
) {
    if let Some(tx) = progress_tx {
        let _ = tx.send(SubAgentEvent::Status(SubAgentStatus::Completed(
            output.clone(),
        )));
        let _ = tx.send(SubAgentEvent::Output(summary.to_string()));
    }
}

fn report_sub_agent_round_complete(
    progress_tx: &Option<mpsc::UnboundedSender<SubAgentEvent>>,
    response: &ChatMessage,
    round: u32,
    max_rounds: u32,
) {
    if let Some(tx) = progress_tx {
        let summary = response
            .content
            .as_deref()
            .unwrap_or("<tool call>")
            .chars()
            .take(80)
            .collect();
        let _ = tx.send(SubAgentEvent::RoundComplete {
            round,
            max: max_rounds,
            summary,
        });
    }
}

fn build_sub_agent_success_output(
    agent_type: &SubAgentType,
    response: ChatMessage,
    round: u32,
) -> SubAgentOutput {
    let final_output = response.content.unwrap_or_default();
    SubAgentOutput {
        agent_type: *agent_type,
        success: true,
        summary: final_output,
        rounds_used: round,
        error: None,
    }
}

/// Execute a single tool call with panic protection.
/// Returns the result string or a formatted error/panic message.
fn execute_single_tool_call(tool_registry: &Arc<ToolRegistry>, tool_call: &ToolCall) -> String {
    match tool_registry.get_clone(&tool_call.function_name) {
        Some(tool) => {
            let fn_name = tool_call.function_name.clone();
            let args = tool_call.arguments.clone();
            let execute_result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tool.execute(args)));
            match execute_result {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => format!("Error: {}", e),
                Err(panic_info) => {
                    let panic_msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                        s.to_string()
                    } else if let Some(s) = panic_info.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "Unknown panic reason".to_string()
                    };
                    format!("Panic in tool '{}': {}", fn_name, panic_msg)
                },
            }
        },
        None => format!("Error: Unknown tool: {}", tool_call.function_name),
    }
}

fn execute_sub_agent_tool_calls(
    response: &ChatMessage,
    tool_registry: &Arc<ToolRegistry>,
    messages: &mut Vec<ChatMessage>,
) {
    if let Some(tool_calls) = response.tool_calls.as_ref() {
        for tool_call in tool_calls {
            let result = execute_single_tool_call(tool_registry, tool_call);
            let tool_msg = ChatMessage::tool(
                result,
                tool_call.id.clone(),
                Some(tool_call.function_name.clone()),
                Some(tool_call.arguments.clone()),
            );
            messages.push(tool_msg);
        }
    }
}

/// Save a sub-agent session to disk for debugging.
fn save_sub_agent_session(uuid: Uuid, messages: &[ChatMessage]) {
    if let Ok(project_dir) = std::env::current_dir() {
        let dir_name = format!(
            "{}/sub_agents",
            project_dir
                .to_string_lossy()
                .replace(['/', '\\'], "-")
                .replace(':', "")
        );
        if let Err(e) = save_session(uuid, messages.iter().collect(), &dir_name) {
            warn!("Failed to save sub-agent session: {}", e);
        }
    }
}

/// Build the initial messages for a sub-agent: system prompt + user message.
fn build_sub_agent_messages(
    agent_type: &SubAgentType,
    task: &str,
    context: &Option<String>,
    tool_registry: &ToolRegistry,
) -> Vec<ChatMessage> {
    let mut messages = Vec::new();

    let mut system_prompt = agent_type.system_prompt().to_string();

    // Append tool descriptions so LLM knows how to use the available tools
    let tools_desc = tool_registry.get_tools_system_prompt();
    if !tools_desc.is_empty() {
        system_prompt.push_str("\n\n## 可用工具\n");
        system_prompt.push_str(&tools_desc);
    }

    // Append environment context (workspace dir + current time) — same as MainAgent
    system_prompt.push_str("\n\n## 环境上下文\n");
    if let Ok(path) = std::env::current_dir() {
        system_prompt.push_str(&format!("- 工作目录: {}\n", path.to_string_lossy()));
    }
    let now = chrono::Utc::now();
    system_prompt.push_str(&format!(
        "- 当前时间 (UTC): {}\n",
        now.format("%Y-%m-%d %H:%M")
    ));

    messages.push(ChatMessage::system(system_prompt));

    // Build the user message: context + task (separate from system prompt)
    let mut user_content = String::new();
    if let Some(ctx) = context {
        user_content.push_str("## 附加上下文\n");
        user_content.push_str(ctx);
        user_content.push_str("\n\n");
    }
    user_content.push_str("## 任务\n");
    user_content.push_str(task);
    messages.push(ChatMessage::user(user_content));

    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::errors::AgentError;
    use crate::domain::tool::Tool;
    use serde_json::Value;

    /// A tool that panics when executed.
    struct PanicTool;
    impl Tool for PanicTool {
        fn name(&self) -> &'static str {
            "PanicTool"
        }
        fn description(&self) -> &'static str {
            "A tool that panics"
        }
        fn schema(&self) -> Value {
            serde_json::json!({})
        }
        fn execute(&self, _args: Value) -> Result<String, AgentError> {
            panic!("intentional panic for testing");
        }
        fn get_system_prompt(&self) -> &str {
            ""
        }
        fn clone_box(&self) -> Box<dyn Tool> {
            Box::new(Self)
        }
    }

    /// A tool that returns a normal result.
    struct NormalTool;
    impl Tool for NormalTool {
        fn name(&self) -> &'static str {
            "NormalTool"
        }
        fn description(&self) -> &'static str {
            "A normal tool"
        }
        fn schema(&self) -> Value {
            serde_json::json!({})
        }
        fn execute(&self, _args: Value) -> Result<String, AgentError> {
            Ok("normal result".to_string())
        }
        fn get_system_prompt(&self) -> &str {
            ""
        }
        fn clone_box(&self) -> Box<dyn Tool> {
            Box::new(Self)
        }
    }

    /// A tool that returns an error.
    struct ErrorTool;
    impl Tool for ErrorTool {
        fn name(&self) -> &'static str {
            "ErrorTool"
        }
        fn description(&self) -> &'static str {
            "A tool that errors"
        }
        fn schema(&self) -> Value {
            serde_json::json!({})
        }
        fn execute(&self, _args: Value) -> Result<String, AgentError> {
            Err(AgentError::ToolExecutionError(
                "something went wrong".to_string(),
            ))
        }
        fn get_system_prompt(&self) -> &str {
            ""
        }
        fn clone_box(&self) -> Box<dyn Tool> {
            Box::new(Self)
        }
    }

    #[test]
    fn test_panic_in_tool_is_caught() {
        let mut registry = ToolRegistry::new();
        registry.register(PanicTool);
        let registry = Arc::new(registry);

        let tool_call = oy_ai::ToolCall {
            id: "call_1".into(),
            function_name: "PanicTool".into(),
            arguments: serde_json::json!({}),
        };
        let response = ChatMessage::assistant(None, None, Some(vec![tool_call]));
        let mut messages = vec![];

        // Should NOT panic
        execute_sub_agent_tool_calls(&response, &registry, &mut messages);

        assert_eq!(messages.len(), 1);
        let content = messages[0].content.as_deref().unwrap_or("");
        assert!(
            content.contains("Panic in tool 'PanicTool'"),
            "Expected panic message, got: {}",
            content
        );
        assert!(
            content.contains("intentional panic for testing"),
            "Expected original panic message, got: {}",
            content
        );
    }

    #[test]
    fn test_normal_tool_execution() {
        let mut registry = ToolRegistry::new();
        registry.register(NormalTool);
        let registry = Arc::new(registry);

        let tool_call = oy_ai::ToolCall {
            id: "call_2".into(),
            function_name: "NormalTool".into(),
            arguments: serde_json::json!({}),
        };
        let response = ChatMessage::assistant(None, None, Some(vec![tool_call]));
        let mut messages = vec![];

        execute_sub_agent_tool_calls(&response, &registry, &mut messages);

        assert_eq!(messages.len(), 1);
        let content = messages[0].content.as_deref().unwrap_or("");
        assert_eq!(content, "normal result");
    }

    #[test]
    fn test_tool_returns_error() {
        let mut registry = ToolRegistry::new();
        registry.register(ErrorTool);
        let registry = Arc::new(registry);

        let tool_call = oy_ai::ToolCall {
            id: "call_3".into(),
            function_name: "ErrorTool".into(),
            arguments: serde_json::json!({}),
        };
        let response = ChatMessage::assistant(None, None, Some(vec![tool_call]));
        let mut messages = vec![];

        execute_sub_agent_tool_calls(&response, &registry, &mut messages);

        assert_eq!(messages.len(), 1);
        let content = messages[0].content.as_deref().unwrap_or("");
        assert!(
            content.contains("Error:"),
            "Expected Error prefix, got: {}",
            content
        );
        assert!(
            content.contains("something went wrong"),
            "Expected error message, got: {}",
            content
        );
    }

    #[test]
    fn test_unknown_tool() {
        let registry = Arc::new(ToolRegistry::new());

        let tool_call = oy_ai::ToolCall {
            id: "call_4".into(),
            function_name: "UnknownTool".into(),
            arguments: serde_json::json!({}),
        };
        let response = ChatMessage::assistant(None, None, Some(vec![tool_call]));
        let mut messages = vec![];

        execute_sub_agent_tool_calls(&response, &registry, &mut messages);

        assert_eq!(messages.len(), 1);
        let content = messages[0].content.as_deref().unwrap_or("");
        assert!(
            content.contains("Unknown tool"),
            "Expected 'Unknown tool' message, got: {}",
            content
        );
        assert!(
            content.contains("UnknownTool"),
            "Expected tool name in message, got: {}",
            content
        );
    }
}
