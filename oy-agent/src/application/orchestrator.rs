use oy_ai::{AiProvider, ChatMessage};
use tokio::sync::mpsc::{self, channel};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::Agent;
use crate::agent::{
    CHANNEL_SIZE, InputAgentSignal, InputOrchestratorSignal, OutputAgentSignal,
    OutputOrchestratorSignal,
};
use crate::domain::errors::AgentError;
use crate::infrastructure::tools::ToolRegistry;

pub struct Orchestrator {
    agent: Box<dyn Agent>,
    provider: Box<dyn AiProvider + Send + Sync>,
    tool_registry: ToolRegistry,
    uuid: Uuid,
}

unsafe impl Send for Orchestrator {}
unsafe impl Sync for Orchestrator {}

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
            uuid: Uuid::now_v7(),
        }
    }

    pub fn init(&mut self) {
        let _ = self.agent.push_message_back(
            self.uuid,
            ChatMessage::system(
                self.agent
                    .get_system_prompt(&self.tool_registry.get_tools_system_prompt()),
            ),
        );
    }

    /// Execute the agent loop: send prompt, process tool calls, return final text.
    ///
    /// The loop terminates when the AI responds without tool_calls, or when
    /// max_iterations is reached (safety guard against infinite loops from
    /// buggy tool outputs or model misbehaviour).
    pub async fn execute(&mut self, prompt: &str) -> Result<String, AgentError> {
        let _ = self
            .agent
            .push_message_back(self.uuid, ChatMessage::user(prompt));

        for _ in 0..self.agent.max_iterations() {
            let response = self
                .provider
                .chat(self.agent.messages(), &self.tool_registry.get_schemas())
                .await?;

            let has_tool_calls = response.tool_calls.as_ref().is_some_and(|c| !c.is_empty());
            let _ = self.agent.push_message_back(self.uuid, response.clone());

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
                let _ = self
                    .agent
                    .push_message_back(self.uuid, ChatMessage::tool(result, tool_call.id));
            }
        }

        Err(AgentError::MaxIterationsReached)
    }
}

pub(crate) async fn start(
    mut orchestrator: Orchestrator,
) -> (
    mpsc::Sender<InputOrchestratorSignal>,
    mpsc::Receiver<OutputOrchestratorSignal>,
    JoinHandle<()>,
) {
    let (request_tx, mut request_rx) = channel::<InputOrchestratorSignal>(CHANNEL_SIZE);
    let (response_tx, response_rx) = channel::<OutputOrchestratorSignal>(CHANNEL_SIZE);
    let join_handle = tokio::spawn(async move {
        loop {
            let _ = response_tx.send(OutputOrchestratorSignal::Pause).await;
            if let Some(request) = request_rx.recv().await {
                match request {
                    InputOrchestratorSignal::Prompt(prompt) => {
                        let _ = response_tx.send(OutputOrchestratorSignal::Running).await;

                        let _ = orchestrator.agent.push_message_back(
                            orchestrator.uuid,
                            ChatMessage::user(prompt.clone()),
                        );
                        let _ = response_tx
                            .send(OutputOrchestratorSignal::ChatMessage(ChatMessage::user(
                                prompt.clone(),
                            )))
                            .await;

                        for _ in 0..orchestrator.agent.max_iterations() {
                            match orchestrator
                                .provider
                                .chat(
                                    orchestrator.agent.messages(),
                                    &orchestrator.tool_registry.get_schemas(),
                                )
                                .await
                            {
                                Ok(response) => {
                                    let has_tool_calls =
                                        response.tool_calls.as_ref().is_some_and(|c| !c.is_empty());
                                    let _ = orchestrator
                                        .agent
                                        .push_message_back(orchestrator.uuid, response.clone());
                                    let _ = response_tx
                                        .send(OutputOrchestratorSignal::ChatMessage(
                                            response.clone(),
                                        ))
                                        .await;

                                    if !has_tool_calls {
                                        continue;
                                    }

                                    for tool_call in response.tool_calls.unwrap() {
                                        match orchestrator
                                            .tool_registry
                                            .get(&tool_call.function_name)
                                            .ok_or_else(|| {
                                                AgentError::ToolExecutionError(format!(
                                                    "Unknown tool: {}",
                                                    tool_call.function_name
                                                ))
                                            }) {
                                            Ok(tool) => {
                                                match tool.execute(tool_call.arguments.clone()) {
                                                    Ok(result) => {
                                                        let _ =
                                                            orchestrator.agent.push_message_back(
                                                                orchestrator.uuid,
                                                                ChatMessage::tool(
                                                                    result.clone(),
                                                                    tool_call.id.clone(),
                                                                ),
                                                            );
                                                        let _ = response_tx
                                                        .send(OutputOrchestratorSignal::ChatMessage(
                                                            ChatMessage::tool(result, tool_call.id),
                                                        ))
                                                        .await;
                                                    }
                                                    Err(e) => {
                                                        let _ = response_tx
                                                        .send(OutputOrchestratorSignal::AgentError(e))
                                                        .await;
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                let _ = response_tx
                                                    .send(OutputOrchestratorSignal::AgentError(e))
                                                    .await;
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = response_tx
                                        .send(OutputOrchestratorSignal::AgentError(
                                            AgentError::AiError(e),
                                        ))
                                        .await;
                                }
                            }
                        }

                        let _ = response_tx
                            .send(OutputOrchestratorSignal::AgentError(
                                AgentError::MaxIterationsReached,
                            ))
                            .await;
                    }
                }
            }
        }
    });

    (request_tx, response_rx, join_handle)
}

pub async fn start_agent_background(
    agent: impl Agent + 'static,
    provider: impl AiProvider + 'static,
    tool_registry: ToolRegistry,
) -> (
    mpsc::Sender<InputAgentSignal>,
    mpsc::Receiver<OutputAgentSignal>,
    JoinHandle<()>,
) {
    let (signal_tx, mut signal_rx) = channel::<InputAgentSignal>(CHANNEL_SIZE);
    let (external_signal_tx, external_signal_rx) = channel::<OutputAgentSignal>(CHANNEL_SIZE);
    let join_handle = tokio::spawn(async move {
        let mut orchestrator = Orchestrator::new(agent, provider, tool_registry);
        orchestrator.init();
        let (prompt_sender, mut response_receiver, _join_handle) = start(orchestrator).await;

        loop {
            tokio::select! {
                signal = signal_rx.recv()=> {
                    match signal {
                        Some(signal) => {
                            match signal {
                                InputAgentSignal::Quit => break,
                                InputAgentSignal::UserPrompt(prompt) => {
                                    let _ = prompt_sender.send(InputOrchestratorSignal::Prompt(prompt)).await;
                                },
                                InputAgentSignal::Pause => {},
                            }
                        },
                        None => continue,
                    }
                },
                orchestrator_signal = response_receiver.recv()=>{
                    match orchestrator_signal {
                        Some(orchestrator_signal) => {
                            match orchestrator_signal {
                                OutputOrchestratorSignal::Pause => {
                                    let _ = external_signal_tx.send(OutputAgentSignal::Pause).await;
                                },
                                OutputOrchestratorSignal::Running => {
                                    let _ = external_signal_tx.send(OutputAgentSignal::Running).await;
                                },
                                OutputOrchestratorSignal::ChatMessage(chat_message) => {
                                    let _ = external_signal_tx.send(OutputAgentSignal::ChatMessage(chat_message)).await;
                                },
                                OutputOrchestratorSignal::AgentError(agent_error) => {
                                    let _ = external_signal_tx.send(OutputAgentSignal::AgentError(agent_error)).await;
                                },
                            }
                        },
                        None => continue,
                    }
                },
            }
        }
    });

    (signal_tx, external_signal_rx, join_handle)
}
