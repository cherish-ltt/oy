use std::str::FromStr;
use std::sync::Arc;

use futures::{StreamExt, stream::FuturesUnordered};
use oy_ai::{AiProvider, ChatMessage, Role};
use tokio::{
    sync::mpsc::{Receiver, Sender},
    task::JoinHandle,
};
use uuid::Uuid;

use crate::{
    AgentError,
    agent::{AgentCore, AgentEvent, AgentState, PromptRequest, ResponseAgent},
    domain::{
        sub_agent::SubAgentType,
        token_counter::{TokenUsage, count_input_side_tokens, count_output_side_tokens},
    },
    infrastructure::tools::ToolRegistry,
};

use self::{
    reactor::{WorkerCommand, WorkerEvent},
    sub_agent_runner::{SubAgentConfig, run_sub_agent},
};

pub mod commander_agent;
pub mod main_agent;
pub mod reactor;
pub mod sub_agent_runner;

pub(crate) struct Worker {
    uuid: Uuid,
    agent: Box<dyn AgentCore>,
    provider: Box<dyn AiProvider + Send + Sync>,
    tool_registry: ToolRegistry,
    current_iterations: u32,
    tool_tasks: Option<FuturesUnordered<JoinHandle<ChatMessage>>>,
    cmd_rx: Receiver<WorkerCommand>,
    event_tx: Sender<WorkerEvent>,
    /// Cumulative token usage across the entire session
    token_usage: TokenUsage,
    /// Provider for sub-agent execution (Arc'd for sharing, None for MainAgent)
    sub_provider: Option<Arc<dyn AiProvider + Send + Sync>>,
    /// File-only tool registry for sub-agents (None for MainAgent)
    sub_tool_registry: Option<Arc<ToolRegistry>>,
}

/// Parameters for resuming an existing Worker session.
pub(crate) struct SessionConfig {
    pub uuid: Uuid,
    pub initial_messages: Vec<ChatMessage>,
}

impl Worker {
    pub(crate) fn new(
        agent: impl AgentCore + 'static,
        provider: impl AiProvider + 'static,
        tool_registry: ToolRegistry,
        cmd_rx: Receiver<WorkerCommand>,
        event_tx: Sender<WorkerEvent>,
    ) -> Self {
        let uuid = Uuid::now_v7();
        Self {
            uuid,
            agent: Box::new(agent),
            provider: Box::new(provider),
            current_iterations: 0,
            tool_registry,
            tool_tasks: None,
            cmd_rx,
            event_tx,
            token_usage: TokenUsage::new(),
            sub_provider: None,
            sub_tool_registry: None,
        }
    }

    /// Create a Worker that resumes an existing session with a specific UUID
    /// and pre-loaded message history.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with_session(
        agent: impl AgentCore + 'static,
        provider: impl AiProvider + 'static,
        tool_registry: ToolRegistry,
        cmd_rx: Receiver<WorkerCommand>,
        event_tx: Sender<WorkerEvent>,
        session: SessionConfig,
    ) -> Self {
        let uuid = session.uuid;
        let mut worker = Self {
            uuid,
            agent: Box::new(agent),
            provider: Box::new(provider),
            current_iterations: 0,
            tool_registry,
            tool_tasks: None,
            cmd_rx,
            event_tx,
            token_usage: TokenUsage::new(),
            sub_provider: None,
            sub_tool_registry: None,
        };
        // Pre-load historical messages — push_message_back handles
        // persisting to the session file with the correct uuid.
        for msg in session.initial_messages {
            let _ = worker.agent.push_message_back(worker.uuid, msg);
        }
        worker
    }

    async fn send_event_async(&self, event: WorkerEvent) {
        let _ = self.event_tx.send(event).await;
    }

    async fn notify_state(&self, state: AgentState) {
        let _ = self.event_tx.send(WorkerEvent::StateChanged(state)).await;
    }

    pub(crate) fn set_provider(
        &mut self,
        provider: Box<dyn AiProvider + Send + Sync>,
    ) -> Result<AgentEvent, AgentError> {
        self.provider = provider;
        Ok(AgentEvent::StartOver)
    }

    /// Set sub-agent dependencies (provider + file tools) for CommanderAgent.
    pub(crate) fn set_sub_agent_deps(
        &mut self,
        provider: Arc<dyn AiProvider + Send + Sync>,
        registry: Arc<ToolRegistry>,
    ) {
        self.sub_provider = Some(provider);
        self.sub_tool_registry = Some(registry);
    }

    pub(crate) async fn run(&mut self) {
        loop {
            // Drain instant commands only in non-Idle states.
            // In Idle state, commands must go through recv() in the Idle branch
            // to properly trigger state machine transitions via assembly_prompts.
            if *self.agent.current_state() != AgentState::Idle {
                self.try_drain_commands().await;
            }

            let result = match self.agent.current_state() {
                AgentState::Idle => match self.cmd_rx.recv().await {
                    Some(cmd) => match self.handle_idle_cmd(cmd).await {
                        Some(r) => r,
                        None => continue,
                    },
                    None => return,
                },
                AgentState::Thinking => self.thinking().await,
                AgentState::Acting => self.acting().await,
                AgentState::ToolCall => {
                    let r = self.tool_call().await;
                    // After collecting tool results, drain any queued Enter prompts.
                    // They get injected as user messages alongside tool results,
                    // before the next Thinking (LLM call).
                    if r.is_ok() {
                        self.drain_pending_commands().await;
                    }
                    r
                },
                AgentState::Observing => self.observing().await,
            };

            self.process_result(result).await;
        }
    }

    /// Drain queued commands without blocking (non-Idle states).
    async fn try_drain_commands(&mut self) {
        loop {
            match self.cmd_rx.try_recv() {
                Ok(WorkerCommand::GetMessages { tx }) => {
                    let msgs = self.agent.messages().to_vec();
                    let _ = tx.send(msgs);
                },
                Ok(WorkerCommand::SetMessages(msgs)) => {
                    self.agent.replace_messages(msgs);
                },
                Ok(WorkerCommand::SetProvider(provider)) => {
                    self.provider = provider;
                },
                Ok(WorkerCommand::SetSkills(skills)) => {
                    self.agent.set_skills(skills);
                },
                Ok(WorkerCommand::Prompt { text, .. }) => {
                    // Inject as user message immediately so nothing is lost.
                    let _ = self.inject_user_message(&text).await;
                },
                Ok(WorkerCommand::FlushEnterQueue(requests)) => {
                    for pr in &requests {
                        if self.inject_user_message(&pr.text).await.is_err() {
                            break;
                        }
                    }
                },
                Err(_) => break,
            }
        }
    }

    /// Handle a single command received in Idle state.
    ///
    /// Returns `Some(result)` if the command should be processed through the state machine,
    /// or `None` if the command was handled and the outer loop should continue without
    /// a state transition (e.g. SetSkills, GetMessages, SetMessages).
    async fn handle_idle_cmd(
        &mut self,
        cmd: WorkerCommand,
    ) -> Option<Result<AgentEvent, AgentError>> {
        match cmd {
            WorkerCommand::Prompt { text, .. } => Some(self.assembly_prompts(&text).await),
            WorkerCommand::FlushEnterQueue(requests) => {
                self.handle_flush_enter_queue(requests).await
            },
            WorkerCommand::SetProvider(ai_provider) => Some(self.set_provider(ai_provider)),
            WorkerCommand::SetSkills(skills) => {
                self.agent.set_skills(skills);
                None
            },
            WorkerCommand::GetMessages { tx } => {
                let msgs = self.agent.messages().to_vec();
                let _ = tx.send(msgs);
                None
            },
            WorkerCommand::SetMessages(msgs) => {
                self.agent.replace_messages(msgs);
                None
            },
        }
    }

    /// Process all queued prompts from a FlushEnterQueue command.
    /// Returns the last error encountered, or Ok(Start) if all succeeded.
    async fn handle_flush_enter_queue(
        &mut self,
        requests: Vec<PromptRequest>,
    ) -> Option<Result<AgentEvent, AgentError>> {
        let mut last_error = None;
        for pr in &requests {
            if let Err(e) = self.assembly_prompts(&pr.text).await {
                last_error = Some(e);
            }
        }
        Some(match last_error {
            Some(e) => Err(e),
            None => Ok(AgentEvent::Start),
        })
    }

    /// Process a state machine result: transition to the next state and notify.
    async fn process_result(&mut self, result: Result<AgentEvent, AgentError>) {
        match result {
            Ok(event) => {
                if let Some(agent_state) = self.agent.next_state(self.agent.current_state(), &event)
                {
                    let old_state = self.agent.current_state().clone();
                    self.agent.set_state(agent_state);
                    let new_state = self.agent.current_state();
                    if *new_state != old_state {
                        self.notify_state(new_state.clone()).await;
                    }
                }
            },
            Err(e) => {
                if let Some(agent_state) = self
                    .agent
                    .next_state(self.agent.current_state(), &self.handle_error(e).await)
                {
                    let old_state = self.agent.current_state().clone();
                    self.agent.set_state(agent_state);
                    let new_state = self.agent.current_state();
                    if *new_state != old_state {
                        self.notify_state(new_state.clone()).await;
                    }
                }
            },
        }
    }

    async fn handle_error(&self, error: AgentError) -> AgentEvent {
        self.send_event_async(WorkerEvent::Response(ResponseAgent::AgentError(error)))
            .await;
        AgentEvent::Error
    }

    async fn assembly_prompts(&mut self, prompt: &String) -> Result<AgentEvent, AgentError> {
        let needs_system_prompt = match self.agent.get_front_message() {
            None => true,
            Some(msg) => msg.role != Role::System,
        };
        if needs_system_prompt {
            let system_msg = ChatMessage::system(
                self.agent
                    .get_system_prompt(&self.tool_registry.get_tools_system_prompt()),
            );
            self.agent.insert_message_front(self.uuid, system_msg)?;
        }
        self.agent
            .push_message_back(self.uuid, ChatMessage::user(prompt.clone()))?;
        self.send_event_async(WorkerEvent::Response(ResponseAgent::ChatMessage(
            ChatMessage::user(prompt),
        )))
        .await;

        Ok(AgentEvent::Start)
    }

    async fn thinking(&mut self) -> Result<AgentEvent, AgentError> {
        self.send_event_async(WorkerEvent::Response(ResponseAgent::Running))
            .await;

        let mut response = self
            .provider
            .chat(self.agent.messages(), &self.tool_registry.get_schemas())
            .await?;

        // Inject default_timeout into each tool call's arguments so the TUI
        // can display the correct timeout (instead of a hardcoded fallback).
        self.inject_tool_default_timeouts(&mut response);

        // Push response to messages first so all counts are accurate
        self.agent.push_message_back(self.uuid, response.clone())?;

        // Role-based token breakdown
        self.token_usage.input_tokens = count_input_side_tokens(self.agent.messages());
        self.token_usage.output_tokens = count_output_side_tokens(self.agent.messages());
        self.token_usage.context_tokens =
            self.token_usage.input_tokens + self.token_usage.output_tokens;

        // Send updated token usage to the UI
        self.send_event_async(WorkerEvent::Response(ResponseAgent::TokenUsage(
            self.token_usage,
        )))
        .await;

        self.send_event_async(WorkerEvent::Response(ResponseAgent::ChatMessage(response)))
            .await;

        self.current_iterations += 1;
        if self.current_iterations >= self.agent.max_iterations() {
            return Err(AgentError::MaxIterationsReached);
        }

        Ok(AgentEvent::ThinkingCompleted)
    }

    async fn acting(&mut self) -> Result<AgentEvent, AgentError> {
        match self.agent.get_back_message() {
            Some(chat_message) => {
                if let Some(tool_calls) = chat_message.tool_calls.clone().filter(|c| !c.is_empty())
                {
                    let tasks = FuturesUnordered::new();
                    for tool_call in tool_calls {
                        // Special handling: create_sub_agent runs as async sub-agent
                        // via tokio::spawn + .await, not via sync Tool::execute.
                        if tool_call.function_name == "create_sub_agent"
                            && self.sub_provider.is_some()
                        {
                            tasks.push(self.spawn_sub_agent_task(tool_call));
                        } else {
                            tasks.push(self.spawn_regular_tool_task(tool_call));
                        }
                    }
                    self.tool_tasks = Some(tasks);
                    return Ok(AgentEvent::ToolCallsRequired);
                }
                // 如果没有工具调用，继续执行最后的 TaskCompleted
            },
            None => {
                return Err(AgentError::ChatMessageError(
                    "Chat Message cannot be extracted".to_owned(),
                ));
            },
        }

        Ok(AgentEvent::TaskCompleted)
    }

    /// Format the sub-agent execution output into a ChatMessage for the LLM.
    fn format_sub_agent_result(
        tc_id: String,
        tc_name: String,
        tc_args: serde_json::Value,
        output: crate::domain::sub_agent::SubAgentOutput,
    ) -> ChatMessage {
        let result_str = crate::domain::sub_agent::format_sub_agent_output(&output);
        ChatMessage::tool(result_str, tc_id, Some(tc_name), Some(tc_args))
    }

    /// Spawn a tokio task that runs a sub-agent for the create_sub_agent tool call.
    fn spawn_sub_agent_task(
        &self,
        tool_call: oy_ai::ToolCall,
    ) -> tokio::task::JoinHandle<ChatMessage> {
        let timeout_secs = tool_call
            .arguments
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(900);

        if let (Some(provider), Some(registry)) =
            (self.sub_provider.clone(), self.sub_tool_registry.clone())
        {
            spawn_sub_agent_inner(tool_call, provider, registry, timeout_secs)
        } else {
            let tc_id = tool_call.id.clone();
            let tc_name = tool_call.function_name.clone();
            tokio::spawn(async move {
                ChatMessage::tool(
                    "Error: Sub-agent dependencies not configured. Call set_sub_agent_deps first."
                        .to_string(),
                    tc_id,
                    Some(tc_name),
                    None,
                )
            })
        }
    }

    /// Inject each tool's `default_timeout()` into tool call arguments if not
    /// already set by the LLM. This lets the TUI display the correct timeout
    /// for every tool instead of falling back to a hardcoded value.
    fn inject_tool_default_timeouts(&self, response: &mut ChatMessage) {
        let Some(tool_calls) = &mut response.tool_calls else {
            return;
        };
        for tc in tool_calls {
            if tc
                .arguments
                .get("timeout")
                .and_then(|v| v.as_u64())
                .is_none()
                && let Some(tool) = self.tool_registry.get_clone(&tc.function_name)
                && tc.arguments.is_object()
            {
                tc.arguments["timeout"] = serde_json::json!(tool.default_timeout());
            }
        }
    }

    /// Execute a tool synchronously (via catch_unwind) and produce a ChatMessage.
    fn execute_tool(
        t: Box<dyn crate::domain::tool::Tool + Send>,
        tc_args: serde_json::Value,
        tc_id: String,
        tc_name: String,
    ) -> ChatMessage {
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| t.execute(tc_args.clone())));
        let output = match result {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => format!("Error: {}", e),
            Err(panic) => {
                let msg = panic
                    .downcast_ref::<String>()
                    .map(|s| s.as_str())
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown panic");
                format!("Internal error: {}", msg)
            },
        };
        ChatMessage::tool(output, tc_id, Some(tc_name), Some(tc_args))
    }

    /// Spawn a tokio task that executes a regular tool call.
    fn spawn_regular_tool_task(
        &self,
        tool_call: oy_ai::ToolCall,
    ) -> tokio::task::JoinHandle<ChatMessage> {
        match self.tool_registry.get_clone(&tool_call.function_name) {
            Some(t) => spawn_known_tool_task(t, tool_call),
            None => spawn_unknown_tool_task(tool_call),
        }
    }

    async fn tool_call(&mut self) -> Result<AgentEvent, AgentError> {
        if let Some(mut tasks) = self.tool_tasks.take() {
            while let Some(res) = tasks.next().await {
                match res {
                    Ok(chat_message) => {
                        self.agent
                            .push_message_back(self.uuid, chat_message.clone())?;
                        self.send_event_async(WorkerEvent::Response(ResponseAgent::ChatMessage(
                            chat_message,
                        )))
                        .await;
                    },
                    Err(e) => {
                        // JoinError: spawned task panicked despite catch_unwind.
                        // This should be rare, but don't silently swallow it.
                        let err_msg = format!(
                            "Tool execution task panicked and could not be recovered: {}",
                            e
                        );
                        self.send_event_async(WorkerEvent::Response(ResponseAgent::AgentError(
                            AgentError::ToolExecutionError(err_msg),
                        )))
                        .await;
                    },
                }
            }
        }

        Ok(AgentEvent::ToolCallCompleted)
    }

    /// Inject a user message into history without triggering state transition.
    async fn inject_user_message(&mut self, text: &str) -> Result<(), AgentError> {
        let needs_system_prompt = match self.agent.get_front_message() {
            None => true,
            Some(msg) => msg.role != Role::System,
        };
        if needs_system_prompt {
            let system_msg = ChatMessage::system(
                self.agent
                    .get_system_prompt(&self.tool_registry.get_tools_system_prompt()),
            );
            self.agent.insert_message_front(self.uuid, system_msg)?;
        }
        self.agent
            .push_message_back(self.uuid, ChatMessage::user(text))?;
        self.send_event_async(WorkerEvent::Response(ResponseAgent::ChatMessage(
            ChatMessage::user(text),
        )))
        .await;
        Ok(())
    }

    /// After tool_call, drain any queued commands from cmd_rx (Prompt, FlushEnterQueue,
    /// SetProvider, SetSkills, GetMessages, SetMessages) and process them so they are
    /// included before the next state transition.
    async fn drain_pending_commands(&mut self) {
        loop {
            match self.cmd_rx.try_recv() {
                Ok(WorkerCommand::Prompt { text, .. }) => {
                    let _ = self.inject_user_message(&text).await;
                },
                Ok(WorkerCommand::FlushEnterQueue(requests)) => {
                    for pr in &requests {
                        if self.inject_user_message(&pr.text).await.is_err() {
                            break;
                        }
                    }
                },
                Ok(WorkerCommand::SetProvider(provider)) => {
                    self.provider = provider;
                },
                Ok(WorkerCommand::SetSkills(skills)) => {
                    self.agent.set_skills(skills);
                },
                Ok(WorkerCommand::GetMessages { tx }) => {
                    let msgs = self.agent.messages().to_vec();
                    let _ = tx.send(msgs);
                },
                Ok(WorkerCommand::SetMessages(msgs)) => {
                    self.agent.replace_messages(msgs);
                },
                Err(_) => break,
            }
        }
    }

    async fn observing(&mut self) -> Result<AgentEvent, AgentError> {
        if self.tool_tasks.is_some() {
            self.tool_tasks = None;
        }
        self.send_event_async(WorkerEvent::Response(ResponseAgent::Pause))
            .await;

        Ok(AgentEvent::Reset)
    }
}

// ── Free helper functions ──────────────────────────────────────────────

/// Spawn a sub-agent execution task with a timeout.
fn spawn_sub_agent_inner(
    tool_call: oy_ai::ToolCall,
    provider: Arc<dyn AiProvider + Send + Sync>,
    registry: Arc<ToolRegistry>,
    timeout_secs: u64,
) -> tokio::task::JoinHandle<ChatMessage> {
    let id2 = tool_call.id.clone();
    let name2 = tool_call.function_name.clone();

    tokio::spawn(async move {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            run_sub_agent_async(tool_call, provider, registry),
        )
        .await;

        result.unwrap_or_else(|_| {
            ChatMessage::tool(
                format!(
                    "[Timeout] Sub-agent execution exceeded {} seconds",
                    timeout_secs
                ),
                id2,
                Some(name2),
                None,
            )
        })
    })
}

/// Run a sub-agent asynchronously and return the result ChatMessage.
async fn run_sub_agent_async(
    tc: oy_ai::ToolCall,
    provider: Arc<dyn AiProvider + Send + Sync>,
    registry: Arc<ToolRegistry>,
) -> ChatMessage {
    let agent_type_str = tc
        .arguments
        .get("agent_type")
        .and_then(|v| v.as_str())
        .unwrap_or("planner");
    let task = tc
        .arguments
        .get("task")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let context = tc
        .arguments
        .get("context")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let agent_type = SubAgentType::from_str(agent_type_str).unwrap_or(SubAgentType::Planner);

    let output = run_sub_agent(SubAgentConfig {
        agent_type,
        task: task.to_string(),
        context,
        provider,
        tool_registry: registry,
        progress_tx: None,
    })
    .await;

    Worker::format_sub_agent_result(tc.id, tc.function_name, tc.arguments, output)
}

/// Spawn a tokio task that runs a known tool with a timeout.
fn spawn_known_tool_task(
    t: Box<dyn crate::domain::tool::Tool + Send>,
    tool_call: oy_ai::ToolCall,
) -> tokio::task::JoinHandle<ChatMessage> {
    let default_timeout = t.default_timeout();
    tokio::spawn(
        async move { execute_known_tool_with_timeout(t, tool_call, default_timeout).await },
    )
}

/// Execute a known tool with timeout, producing a ChatMessage.
async fn execute_known_tool_with_timeout(
    t: Box<dyn crate::domain::tool::Tool + Send>,
    tool_call: oy_ai::ToolCall,
    default_timeout: u64,
) -> ChatMessage {
    let tc_args = tool_call.arguments.clone();
    let timeout_secs = tc_args
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(default_timeout);
    let id2 = tool_call.id.clone();
    let name2 = tool_call.function_name.clone();
    match tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        tokio::task::spawn_blocking(move || Worker::execute_tool(t, tc_args, id2, name2)),
    )
    .await
    {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => ChatMessage::tool(
            format!("Internal error: tool execution failed: {}", e),
            tool_call.id,
            Some(tool_call.function_name),
            None,
        ),
        Err(_) => ChatMessage::tool(
            format!("[Timeout] Tool execution exceeded {} seconds", timeout_secs),
            tool_call.id,
            Some(tool_call.function_name),
            None,
        ),
    }
}

/// Spawn a tokio task that reports an unknown tool error.
fn spawn_unknown_tool_task(tool_call: oy_ai::ToolCall) -> tokio::task::JoinHandle<ChatMessage> {
    let tc_id = tool_call.id.clone();
    let tc_name = tool_call.function_name.clone();
    let tc_args = tool_call.arguments.clone();
    tokio::spawn(async move {
        ChatMessage::tool(
            format!("Error: Unknown tool: {}", tc_name),
            tc_id,
            Some(tc_name),
            Some(tc_args),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::Worker;
    use crate::domain::errors::AgentError;
    use crate::domain::sub_agent::{SubAgentOutput, SubAgentType};
    use crate::domain::tool::Tool;
    use oy_ai::Role;
    use serde_json::{Value, json};

    struct MockTool {
        should_fail: bool,
        should_panic: bool,
    }

    impl Tool for MockTool {
        fn name(&self) -> &'static str {
            "MockTool"
        }

        fn description(&self) -> &'static str {
            "A mock tool for unit testing"
        }

        fn schema(&self) -> Value {
            json!({})
        }

        fn execute(&self, _args: Value) -> Result<String, AgentError> {
            if self.should_panic {
                panic!("mock panic");
            }
            if self.should_fail {
                Err(AgentError::ToolExecutionError("mock error".into()))
            } else {
                Ok("mock success".into())
            }
        }

        fn get_system_prompt(&self) -> &str {
            ""
        }

        fn clone_box(&self) -> Box<dyn Tool> {
            Box::new(MockTool {
                should_fail: self.should_fail,
                should_panic: self.should_panic,
            })
        }
    }

    #[test]
    fn test_execute_tool_success() {
        let tool = MockTool {
            should_fail: false,
            should_panic: false,
        };
        let tc_args = json!({"key": "value"});
        let result = Worker::execute_tool(
            Box::new(tool),
            tc_args.clone(),
            "call_001".to_string(),
            "MockTool".to_string(),
        );
        assert_eq!(result.role, Role::Tool);
        assert_eq!(result.content.as_deref(), Some("mock success"));
        assert_eq!(result.tool_call_id.as_deref(), Some("call_001"));
        assert_eq!(result.function_name.as_deref(), Some("MockTool"));
        assert_eq!(result.tool_call_arguments.as_ref(), Some(&tc_args));
    }

    #[test]
    fn test_execute_tool_error() {
        let tool = MockTool {
            should_fail: true,
            should_panic: false,
        };
        let tc_args = json!({});
        let result = Worker::execute_tool(
            Box::new(tool),
            tc_args.clone(),
            "call_002".to_string(),
            "MockTool".to_string(),
        );
        assert_eq!(result.role, Role::Tool);
        assert_eq!(
            result.content.as_deref(),
            Some("Error: Tool execution error: mock error")
        );
        assert_eq!(result.tool_call_id.as_deref(), Some("call_002"));
        assert_eq!(result.function_name.as_deref(), Some("MockTool"));
        assert_eq!(result.tool_call_arguments.as_ref(), Some(&tc_args));
    }

    #[test]
    fn test_execute_tool_panic() {
        let tool = MockTool {
            should_fail: false,
            should_panic: true,
        };
        let tc_args = json!({"panic": true});
        let result = Worker::execute_tool(
            Box::new(tool),
            tc_args.clone(),
            "call_003".to_string(),
            "MockTool".to_string(),
        );
        assert_eq!(result.role, Role::Tool);
        assert_eq!(
            result.content.as_deref(),
            Some("Internal error: mock panic")
        );
        assert_eq!(result.tool_call_id.as_deref(), Some("call_003"));
        assert_eq!(result.function_name.as_deref(), Some("MockTool"));
        assert_eq!(result.tool_call_arguments.as_ref(), Some(&tc_args));
    }

    // ── format_sub_agent_result tests ────────────────────────────────────
    // These tests verify ChatMessage wrapping only.
    // Formatting content correctness is tested in domain::sub_agent tests.

    #[test]
    fn test_format_sub_agent_result_wraps_as_tool_message() {
        let output = SubAgentOutput {
            agent_type: SubAgentType::Planner,
            success: true,
            summary: "test summary".to_string(),
            rounds_used: 3,
            error: None,
        };
        let tc_args = json!({"agent_type": "planner", "task": "test"});
        let msg = Worker::format_sub_agent_result(
            "call_001".to_string(),
            "create_sub_agent".to_string(),
            tc_args.clone(),
            output,
        );

        assert_eq!(msg.role, Role::Tool);
        assert_eq!(msg.tool_call_id.as_deref(), Some("call_001"));
        assert_eq!(msg.function_name.as_deref(), Some("create_sub_agent"));
        assert_eq!(msg.tool_call_arguments.as_ref(), Some(&tc_args));
        // Formatting content is validated by domain::sub_agent::format_sub_agent_output tests
        assert!(msg.content.as_deref().unwrap().contains("Planner"));
    }

    #[test]
    fn test_format_sub_agent_result_failure_wraps_as_tool_message() {
        let output = SubAgentOutput {
            agent_type: SubAgentType::Worker,
            success: false,
            summary: "".to_string(),
            rounds_used: 10,
            error: Some("Something went wrong".to_string()),
        };
        let tc_args = json!({"agent_type": "worker", "task": "fail"});
        let msg = Worker::format_sub_agent_result(
            "call_002".to_string(),
            "create_sub_agent".to_string(),
            tc_args.clone(),
            output,
        );

        assert_eq!(msg.role, Role::Tool);
        assert!(msg.content.as_deref().unwrap().contains("失败"));
    }
}
