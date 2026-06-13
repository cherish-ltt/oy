use std::str::FromStr;
use std::sync::Arc;

use futures::{StreamExt, stream::FuturesUnordered};
use oy_ai::{AiProvider, ChatMessage};
use tokio::{
    sync::mpsc::{Receiver, Sender},
    task::JoinHandle,
};
use uuid::Uuid;

use crate::{
    AgentError,
    agent::{AgentCore, AgentEvent, AgentState, ResponseAgent},
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
    pub(crate) fn with_session(
        agent: impl AgentCore + 'static,
        provider: impl AiProvider + 'static,
        tool_registry: ToolRegistry,
        cmd_rx: Receiver<WorkerCommand>,
        event_tx: Sender<WorkerEvent>,
        session_uuid: Uuid,
        initial_messages: Vec<ChatMessage>,
    ) -> Self {
        let uuid = session_uuid;
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
        for msg in initial_messages {
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
                // Process every prompt so none are lost; collect last error.
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
        if self.agent.get_front_message().is_none() {
            self.agent.push_message_back(
                self.uuid,
                ChatMessage::system(
                    self.agent
                        .get_system_prompt(&self.tool_registry.get_tools_system_prompt()),
                ),
            )?;
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

        let response = self
            .provider
            .chat(self.agent.messages(), &self.tool_registry.get_schemas())
            .await?;

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
                if chat_message
                    .tool_calls
                    .as_ref()
                    .is_some_and(|c| !c.is_empty())
                {
                    let tasks = FuturesUnordered::new();
                    for tool_call in chat_message.tool_calls.clone().unwrap() {
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

    /// Spawn a tokio task that runs a sub-agent for the create_sub_agent tool call.
    fn spawn_sub_agent_task(
        &self,
        tool_call: oy_ai::ToolCall,
    ) -> tokio::task::JoinHandle<ChatMessage> {
        let tc_id = tool_call.id.clone();
        let tc_name = tool_call.function_name.clone();
        let tc_args = tool_call.arguments.clone();
        let provider = self.sub_provider.clone().unwrap();
        let registry = self.sub_tool_registry.clone().unwrap();

        tokio::spawn(async move {
            // Parse arguments
            let agent_type_str = tc_args
                .get("agent_type")
                .and_then(|v| v.as_str())
                .unwrap_or("planner");
            let task = tc_args.get("task").and_then(|v| v.as_str()).unwrap_or("");
            let context = tc_args
                .get("context")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());

            let agent_type =
                SubAgentType::from_str(agent_type_str).unwrap_or(SubAgentType::Planner);

            let output = run_sub_agent(SubAgentConfig {
                agent_type,
                task: task.to_string(),
                context,
                provider,
                tool_registry: registry,
                progress_tx: None,
            })
            .await;

            let result_str = if output.success {
                format!(
                    "[{} 完成 - {} 轮]\n{}\n{}",
                    agent_type,
                    output.rounds_used,
                    output.summary,
                    match agent_type {
                        SubAgentType::Planner => "计划已创建，Worker 可引用此计划文件。",
                        SubAgentType::Worker => "代码已产出，Reviewer 可审查。",
                        SubAgentType::Reviewer => {
                            "审查完成，请检查 '通过: 是/否' 决定下一步。"
                        },
                        SubAgentType::GitHelper => "操作已完成（commit/issue/PR）。",
                    }
                )
            } else {
                let err = output.error.unwrap_or_default();
                format!(
                    "[{} 失败 - {} 轮]\n错误: {}",
                    agent_type, output.rounds_used, err
                )
            };

            ChatMessage::tool(result_str, tc_id, Some(tc_name), Some(tc_args))
        })
    }

    /// Spawn a tokio task that executes a regular tool call.
    fn spawn_regular_tool_task(
        &self,
        tool_call: oy_ai::ToolCall,
    ) -> tokio::task::JoinHandle<ChatMessage> {
        match self.tool_registry.get_clone(&tool_call.function_name) {
            Some(t) => {
                // Clone metadata BEFORE the async move so we can
                // produce a ChatMessage::tool even if the task panics.
                let tc_id = tool_call.id.clone();
                let tc_name = tool_call.function_name.clone();
                let tc_args = tool_call.arguments.clone();
                tokio::spawn(async move {
                    // catch_unwind prevents panics from becoming
                    // JoinErrors that would lose the tool_call metadata.
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        t.execute(tc_args.clone())
                    }));
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
                })
            },
            None => {
                let tc_id = tool_call.id.clone();
                let tc_name = tool_call.function_name.clone();
                let tc_args = tool_call.arguments.clone();
                tokio::spawn(async move {
                    ChatMessage::tool(
                        format!("Error: Unknown tool: {}", tool_call.function_name),
                        tc_id,
                        Some(tc_name),
                        Some(tc_args),
                    )
                })
            },
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
        if self.agent.get_front_message().is_none() {
            self.agent.push_message_back(
                self.uuid,
                ChatMessage::system(
                    self.agent
                        .get_system_prompt(&self.tool_registry.get_tools_system_prompt()),
                ),
            )?;
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
