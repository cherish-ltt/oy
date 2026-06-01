use futures::{StreamExt, stream::FuturesUnordered};
use oy_ai::{AiProvider, ChatMessage};
use tokio::{
    sync::mpsc::{Receiver, Sender},
    task::JoinHandle,
};
use uuid::Uuid;

use crate::{
    AgentError,
    agent::{AgentCore, AgentEvent, RequestAgent, ResponseAgent},
    domain::token_counter::{TokenUsage, count_input_tokens, count_message_tokens},
    infrastructure::tools::ToolRegistry,
};

pub mod main_agent;

pub(crate) struct AgentLoop {
    uuid: Uuid,
    agent: Box<dyn AgentCore>,
    provider: Box<dyn AiProvider + Send + Sync>,
    tool_registry: ToolRegistry,
    current_iterations: u32,
    tool_tasks: Option<FuturesUnordered<JoinHandle<ChatMessage>>>,
    request_rx: Receiver<RequestAgent>,
    response_tx: Sender<ResponseAgent>,
    /// Cumulative token usage across the entire session
    token_usage: TokenUsage,
}

impl AgentLoop {
    pub(crate) fn new(
        agent: impl AgentCore + 'static,
        provider: impl AiProvider + 'static,
        tool_registry: ToolRegistry,
        request_rx: Receiver<RequestAgent>,
        response_tx: Sender<ResponseAgent>,
    ) -> Self {
        let uuid = Uuid::now_v7();
        Self {
            uuid,
            agent: Box::new(agent),
            provider: Box::new(provider),
            current_iterations: 0,
            tool_registry,
            tool_tasks: None,
            request_rx,
            response_tx,
            token_usage: TokenUsage::new(),
        }
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
                crate::agent::AgentState::Idle => match self.request_rx.recv().await {
                    Some(request) => match request {
                        RequestAgent::Prompt(prompt) => {
                            result = self.assembly_prompts(&prompt).await;
                        }
                        RequestAgent::SetProvider(ai_provider) => {
                            result = self.set_provider(ai_provider);
                        }
                    },
                    None => break,
                },
                crate::agent::AgentState::Thinking => result = self.thinking().await,
                crate::agent::AgentState::Acting => result = self.acting().await,
                crate::agent::AgentState::ToolCall => result = self.tool_call().await,
                crate::agent::AgentState::Observing => result = self.observing().await,
            }

            match result {
                Ok(event) => {
                    if let Some(agent_state) =
                        self.agent.next_state(self.agent.current_state(), &event)
                    {
                        self.agent.set_state(agent_state);
                    }
                }
                Err(e) => {
                    if let Some(agent_state) = self
                        .agent
                        .next_state(self.agent.current_state(), &self.handle_error(e).await)
                    {
                        self.agent.set_state(agent_state);
                    }
                }
            }
        }
    }

    async fn handle_error(&self, error: AgentError) -> AgentEvent {
        let _ = self
            .response_tx
            .send(ResponseAgent::AgentError(error))
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
        let _ = self
            .response_tx
            .send(ResponseAgent::ChatMessage(ChatMessage::user(prompt)))
            .await;

        Ok(AgentEvent::Start)
    }

    async fn thinking(&mut self) -> Result<AgentEvent, AgentError> {
        let _ = self.response_tx.send(ResponseAgent::Running).await;

        // Count input tokens from all messages before sending to provider
        let input_tokens = count_input_tokens(self.agent.messages());
        self.token_usage.add_input(input_tokens);

        // Track current conversation context size (for context usage display)
        self.token_usage.context_tokens = count_input_tokens(self.agent.messages());

        let response = self
            .provider
            .chat(self.agent.messages(), &self.tool_registry.get_schemas())
            .await?;

        // Count output tokens from the response (content + reasoning_content)
        let output_tokens = count_message_tokens(&response);
        self.token_usage.add_output(output_tokens);

        // Send updated token usage to the UI
        let _ = self
            .response_tx
            .send(ResponseAgent::TokenUsage(self.token_usage))
            .await;

        self.agent.push_message_back(self.uuid, response.clone())?;
        let _ = self
            .response_tx
            .send(ResponseAgent::ChatMessage(response))
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
                                tasks.push(tokio::spawn(async move {
                                    let result = match t.execute(tool_call.arguments.clone()) {
                                        Ok(r) => r,
                                        Err(e) => format!("Error: {}", e),
                                    };
                                    ChatMessage::tool(
                                        result,
                                        tool_call.id,
                                        Some(tool_call.function_name),
                                        Some(tool_call.arguments),
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
                        let _ = self
                            .response_tx
                            .send(ResponseAgent::ChatMessage(chat_message))
                            .await;
                    }
                    Err(_e) => {}
                }
            }
        }

        Ok(AgentEvent::ToolCallCompleted)
    }

    async fn observing(&mut self) -> Result<AgentEvent, AgentError> {
        if self.tool_tasks.is_some() {
            self.tool_tasks = None;
        }
        let _ = self.response_tx.send(ResponseAgent::Pause).await;

        Ok(AgentEvent::Reset)
    }
}
