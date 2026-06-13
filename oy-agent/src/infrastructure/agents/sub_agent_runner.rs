use std::sync::Arc;

use chrono::Utc;
use oy_ai::{AiProvider, ChatMessage};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    domain::sub_agent::{SubAgentOutput, SubAgentStatus, SubAgentType},
    infrastructure::{persistence::save_session, tools::ToolRegistry},
};

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

/// Run a sub-agent with bounded iterations.
///
/// This is a self-contained async function that:
/// 1. Creates a fresh message history (system prompt + task)
/// 2. Runs an LLM loop with tool access
/// 3. Enforces iteration limits
/// 4. Returns the final output or error
pub async fn run_sub_agent(
    agent_type: SubAgentType,
    task: String,
    context: Option<String>,
    provider: Arc<dyn AiProvider + Send + Sync>,
    tool_registry: Arc<ToolRegistry>,
    progress_tx: Option<mpsc::UnboundedSender<SubAgentEvent>>,
) -> SubAgentOutput {
    let uuid = Uuid::now_v7();
    let max_rounds = agent_type.max_rounds();
    let mut messages: Vec<ChatMessage> = Vec::new();

    // 1. Send progress: Pending
    if let Some(ref tx) = progress_tx {
        let _ = tx.send(SubAgentEvent::Status(SubAgentStatus::Pending));
    }

    // 2. Build messages: [System(prompt + tool_desc + env), User(task + context)]
    // Match MainAgent's pattern exactly — inject workspace dir and current time.
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
    let now = Utc::now();
    system_prompt.push_str(&format!(
        "- 当前时间 (UTC): {}\n",
        now.format("%Y-%m-%d %H:%M")
    ));

    messages.push(ChatMessage::system(system_prompt));

    // Build the user message: context + task (separate from system prompt)
    let mut user_content = String::new();
    if let Some(ctx) = &context {
        user_content.push_str("## 附加上下文\n");
        user_content.push_str(ctx);
        user_content.push_str("\n\n");
    }
    user_content.push_str("## 任务\n");
    user_content.push_str(&task);
    messages.push(ChatMessage::user(user_content));

    // 3. Bounded LLM loop
    let mut final_output = String::new();
    for round in 0..max_rounds {
        let current_round = round + 1;

        // Report progress: Running
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(SubAgentEvent::Status(SubAgentStatus::Running {
                round: current_round,
                max_rounds,
            }));
        }

        // 3a. Call LLM
        let response = match provider.chat(&messages, &tool_registry.get_schemas()).await {
            Ok(resp) => resp,
            Err(e) => {
                let err_msg = format!("AI error at round {}: {}", current_round, e);
                if let Some(ref tx) = progress_tx {
                    let _ = tx.send(SubAgentEvent::Status(SubAgentStatus::Failed(
                        err_msg.clone(),
                    )));
                }
                // Save sub-agent session for debugging
                if let Ok(project_dir) = std::env::current_dir() {
                    let dir_name = format!(
                        "{}/sub_agents",
                        project_dir
                            .to_string_lossy()
                            .replace(['/', '\\'], "-")
                            .replace(':', "")
                    );
                    let _ = save_session(uuid, messages.iter().collect(), &dir_name);
                }
                return SubAgentOutput {
                    agent_type,
                    success: false,
                    summary: String::new(),
                    rounds_used: current_round,
                    error: Some(err_msg),
                };
            }
        };

        // 3b. Check if the response has content (final answer without tool calls)
        let has_content = response.content.as_ref().is_some_and(|c| !c.is_empty());
        let has_tool_calls = response.tool_calls.as_ref().is_some_and(|c| !c.is_empty());

        // Push assistant response to history
        messages.push(response.clone());

        // 3c. If no tool calls and has content → we're done
        if !has_tool_calls && has_content {
            final_output = response.content.unwrap_or_default();

            let output = SubAgentOutput {
                agent_type,
                success: true,
                summary: final_output.clone(),
                rounds_used: current_round,
                error: None,
            };

            if let Some(ref tx) = progress_tx {
                let _ = tx.send(SubAgentEvent::Status(SubAgentStatus::Completed(
                    output.clone(),
                )));
                let _ = tx.send(SubAgentEvent::Output(final_output.clone()));
            }

            // Save sub-agent session for debugging
            if let Ok(project_dir) = std::env::current_dir() {
                let dir_name = format!(
                    "{}/sub_agents",
                    project_dir
                        .to_string_lossy()
                        .replace(['/', '\\'], "-")
                        .replace(':', "")
                );
                let _ = save_session(uuid, messages.iter().collect(), &dir_name);
            }

            return output;
        }

        // 3d. If no tool calls and no content → empty response, try again
        if !has_tool_calls && !has_content {
            continue;
        }

        // 3e. Execute tool calls sequentially (no tokio::spawn inside block_on)
        if let Some(tool_calls) = response.tool_calls {
            for tool_call in tool_calls {
                let result = match tool_registry.get_clone(&tool_call.function_name) {
                    Some(tool) => match tool.execute(tool_call.arguments.clone()) {
                        Ok(r) => r,
                        Err(e) => format!("Error: {}", e),
                    },
                    None => format!("Error: Unknown tool: {}", tool_call.function_name),
                };
                let tool_msg = ChatMessage::tool(
                    result,
                    tool_call.id,
                    Some(tool_call.function_name),
                    Some(tool_call.arguments),
                );
                messages.push(tool_msg);
            }
        }

        // Report round complete
        if let Some(ref tx) = progress_tx {
            let summary = response
                .content
                .as_deref()
                .unwrap_or("<tool call>")
                .chars()
                .take(80)
                .collect();
            let _ = tx.send(SubAgentEvent::RoundComplete {
                round: current_round,
                max: max_rounds,
                summary,
            });
        }
    }

    // 4. Max rounds reached without final answer
    let err_msg = format!(
        "Max rounds ({}) reached without completing the task",
        max_rounds
    );
    if let Some(ref tx) = progress_tx {
        let _ = tx.send(SubAgentEvent::Status(SubAgentStatus::Failed(
            err_msg.clone(),
        )));
    }

    // Save sub-agent session for debugging
    if let Ok(project_dir) = std::env::current_dir() {
        let dir_name = format!(
            "{}/sub_agents",
            project_dir
                .to_string_lossy()
                .replace(['/', '\\'], "-")
                .replace(':', "")
        );
        let _ = save_session(uuid, messages.iter().collect(), &dir_name);
    }

    SubAgentOutput {
        agent_type,
        success: false,
        summary: final_output,
        rounds_used: max_rounds,
        error: Some(err_msg),
    }
}
