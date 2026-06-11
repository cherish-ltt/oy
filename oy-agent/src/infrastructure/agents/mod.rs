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
    domain::token_counter::{TokenUsage, count_input_side_tokens, count_output_side_tokens},
    infrastructure::tools::ToolRegistry,
};

use self::reactor::{WorkerCommand, WorkerEvent};

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

    pub(crate) async fn run(&mut self) {
        let mut result;
        loop {
            match self.agent.current_state() {
                AgentState::Idle => match self.cmd_rx.recv().await {
                    Some(cmd) => match cmd {
                        WorkerCommand::Prompt { text, .. } => {
                            result = self.assembly_prompts(&text).await;
                        }
                        WorkerCommand::FlushEnterQueue(requests) => {
                            // Process every prompt so none are lost; collect last error.
                            let mut last_error = None;
                            for pr in &requests {
                                if let Err(e) = self.assembly_prompts(&pr.text).await {
                                    last_error = Some(e);
                                }
                            }
                            result = match last_error {
                                Some(e) => Err(e),
                                None => Ok(AgentEvent::Start),
                            };
                        }
                        WorkerCommand::SetProvider(ai_provider) => {
                            result = self.set_provider(ai_provider);
                        }
                        WorkerCommand::SetSkills(skills) => {
                            self.agent.set_skills(skills);
                            continue;
                        }
                    },
                    None => break,
                },
                AgentState::Thinking => result = self.thinking().await,
                AgentState::Acting => result = self.acting().await,
                AgentState::ToolCall => {
                    result = self.tool_call().await;
                    // After collecting tool results, drain any queued Enter prompts.
                    // They get injected as user messages alongside tool results,
                    // before the next Thinking (LLM call).
                    if result.is_ok() {
                        self.drain_pending_enter_prompts().await;
                    }
                }
                AgentState::Observing => result = self.observing().await,
            }

            match result {
                Ok(event) => {
                    if let Some(agent_state) =
                        self.agent.next_state(self.agent.current_state(), &event)
                    {
                        let old_state = self.agent.current_state().clone();
                        self.agent.set_state(agent_state);
                        let new_state = self.agent.current_state();
                        if *new_state != old_state {
                            self.notify_state(new_state.clone()).await;
                        }
                    }
                }
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
                }
            }
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
                        match self.tool_registry.get_clone(&tool_call.function_name) {
                            Some(t) => {
                                // Clone metadata BEFORE the async move so we can
                                // produce a ChatMessage::tool even if the task panics.
                                let tc_id = tool_call.id.clone();
                                let tc_name = tool_call.function_name.clone();
                                let tc_args = tool_call.arguments.clone();
                                tasks.push(tokio::spawn(async move {
                                    // catch_unwind prevents panics from becoming
                                    // JoinErrors that would lose the tool_call metadata.
                                    let result = std::panic::catch_unwind(
                                        std::panic::AssertUnwindSafe(|| {
                                            t.execute(tc_args.clone())
                                        }),
                                    );
                                    let output = match result {
                                        Ok(Ok(r)) => r,
                                        Ok(Err(e)) => format!("Error: {}", e),
                                        Err(panic) => {
                                            let msg = panic
                                                .downcast_ref::<String>()
                                                .map(|s| s.as_str())
                                                .or_else(|| {
                                                    panic.downcast_ref::<&str>().copied()
                                                })
                                                .unwrap_or("unknown panic");
                                            format!("Internal error: {}", msg)
                                        }
                                    };
                                    ChatMessage::tool(
                                        output,
                                        tc_id,
                                        Some(tc_name),
                                        Some(tc_args),
                                    )
                                }));
                            }
                            None => {
                                tasks.push(tokio::spawn(async move {
                                    ChatMessage::tool(
                                        format!("Error: Unknown tool: {}", tool_call.function_name),
                                        tool_call.id,
                                        Some(tool_call.function_name),
                                        Some(tool_call.arguments),
                                    )
                                }));
                            }
                        };
                    }
                    self.tool_tasks = Some(tasks);
                    return Ok(AgentEvent::ToolCallsRequired);
                }
                // 如果没有工具调用，继续执行最后的 TaskCompleted
            }
            None => {
                return Err(AgentError::ChatMessageError(
                    "Chat Message cannot be extracted".to_owned(),
                ));
            }
        }

        Ok(AgentEvent::TaskCompleted)
    }

    async fn tool_call(&mut self) -> Result<AgentEvent, AgentError> {
        if let Some(mut tasks) = self.tool_tasks.take() {
            while let Some(res) = tasks.next().await {
                match res {
                    Ok(chat_message) => {
                        self.agent
                            .push_message_back(self.uuid, chat_message.clone())?;
                        self.send_event_async(WorkerEvent::Response(
                            ResponseAgent::ChatMessage(chat_message),
                        ))
                        .await;
                    }
                    Err(e) => {
                        // JoinError: spawned task panicked despite catch_unwind.
                        // This should be rare, but don't silently swallow it.
                        let err_msg = format!(
                            "Tool execution task panicked and could not be recovered: {}",
                            e
                        );
                        self.send_event_async(WorkerEvent::Response(
                            ResponseAgent::AgentError(AgentError::ToolExecutionError(
                                err_msg,
                            )),
                        ))
                        .await;
                    }
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

    /// After tool_call, drain any queued Enter prompts from cmd_rx and inject
    /// them as user messages so they're included in the next LLM call.
    async fn drain_pending_enter_prompts(&mut self) {
        loop {
            match self.cmd_rx.try_recv() {
                Ok(WorkerCommand::Prompt { text, .. }) => {
                    let _ = self.inject_user_message(&text).await;
                }
                Ok(WorkerCommand::FlushEnterQueue(requests)) => {
                    for pr in &requests {
                        if self.inject_user_message(&pr.text).await.is_err() {
                            break;
                        }
                    }
                }
                Ok(WorkerCommand::SetProvider(provider)) => {
                    self.provider = provider;
                }
                Ok(WorkerCommand::SetSkills(skills)) => {
                    self.agent.set_skills(skills);
                }
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
