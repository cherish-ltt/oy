use std::sync::Arc;

use crate::agent::{AgentCore, CHANNEL_SIZE, RequestAgent, ResponseAgent};
use crate::infrastructure::agents::Worker;
use crate::infrastructure::agents::reactor::{Reactor, WorkerCommand, WorkerEvent};
use crate::infrastructure::tools::ToolRegistry;
use oy_ai::{AiProvider, ChatMessage};
use tokio::sync::mpsc::{Receiver, Sender, channel};
use tokio::task::JoinHandle;
use uuid::Uuid;

pub struct Orchestrator;

impl Orchestrator {
    pub fn start(
        agent: impl AgentCore + 'static,
        provider: impl AiProvider + 'static,
        tool_registry: ToolRegistry,
    ) -> (
        Sender<RequestAgent>,
        Receiver<ResponseAgent>,
        JoinHandle<()>,
    ) {
        Self::start_inner(agent, provider, tool_registry, None, None, None, None)
    }

    /// Start an orchestrator that resumes an existing session with a specific UUID
    /// and pre-loaded message history.
    pub fn start_with_session(
        agent: impl AgentCore + 'static,
        provider: impl AiProvider + 'static,
        tool_registry: ToolRegistry,
        session_uuid: Uuid,
        session_messages: Vec<ChatMessage>,
    ) -> (
        Sender<RequestAgent>,
        Receiver<ResponseAgent>,
        JoinHandle<()>,
    ) {
        Self::start_inner(
            agent,
            provider,
            tool_registry,
            Some(session_uuid),
            Some(session_messages),
            None,
            None,
        )
    }

    /// Start with optional sub-agent dependencies (for CommanderAgent).
    pub fn start_commander(
        agent: impl AgentCore + 'static,
        provider: impl AiProvider + 'static,
        tool_registry: ToolRegistry,
        sub_provider: Arc<dyn AiProvider + Send + Sync>,
        sub_tool_registry: Arc<ToolRegistry>,
    ) -> (
        Sender<RequestAgent>,
        Receiver<ResponseAgent>,
        JoinHandle<()>,
    ) {
        Self::start_inner(
            agent,
            provider,
            tool_registry,
            None,
            None,
            Some(sub_provider),
            Some(sub_tool_registry),
        )
    }

    /// Start CommanderAgent with a session + sub-agent dependencies.
    #[allow(clippy::too_many_arguments)]
    pub fn start_commander_with_session(
        agent: impl AgentCore + 'static,
        provider: impl AiProvider + 'static,
        tool_registry: ToolRegistry,
        session_uuid: Uuid,
        session_messages: Vec<ChatMessage>,
        sub_provider: Arc<dyn AiProvider + Send + Sync>,
        sub_tool_registry: Arc<ToolRegistry>,
    ) -> (
        Sender<RequestAgent>,
        Receiver<ResponseAgent>,
        JoinHandle<()>,
    ) {
        Self::start_inner(
            agent,
            provider,
            tool_registry,
            Some(session_uuid),
            Some(session_messages),
            Some(sub_provider),
            Some(sub_tool_registry),
        )
    }

    /// Internal: create Worker (optionally with session and/or sub-agent deps).
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn start_inner(
        agent: impl AgentCore + 'static,
        provider: impl AiProvider + 'static,
        tool_registry: ToolRegistry,
        session_uuid: Option<Uuid>,
        session_messages: Option<Vec<ChatMessage>>,
        sub_provider: Option<Arc<dyn AiProvider + Send + Sync>>,
        sub_tool_registry: Option<Arc<ToolRegistry>>,
    ) -> (
        Sender<RequestAgent>,
        Receiver<ResponseAgent>,
        JoinHandle<()>,
    ) {
        let (request_tx, request_rx) = channel::<RequestAgent>(CHANNEL_SIZE);
        let (response_tx, response_rx) = channel::<ResponseAgent>(CHANNEL_SIZE);
        let (worker_cmd_tx, worker_cmd_rx) = channel::<WorkerCommand>(CHANNEL_SIZE);
        let (worker_event_tx, worker_event_rx) = channel::<WorkerEvent>(CHANNEL_SIZE);

        let mut worker = match (session_uuid, session_messages) {
            (Some(uuid), Some(msgs)) => Worker::with_session(
                agent,
                provider,
                tool_registry,
                worker_cmd_rx,
                worker_event_tx,
                uuid,
                msgs,
            ),
            _ => Worker::new(
                agent,
                provider,
                tool_registry,
                worker_cmd_rx,
                worker_event_tx,
            ),
        };

        // Set sub-agent dependencies if provided (CommanderAgent only)
        if let (Some(sp), Some(str)) = (sub_provider, sub_tool_registry) {
            worker.set_sub_agent_deps(sp, str);
        }

        let reactor = Reactor::new(request_rx, response_tx, worker_cmd_tx, worker_event_rx);

        let worker_handle = tokio::spawn(async move {
            let mut worker = worker;
            worker.run().await;
        });
        let reactor_handle = tokio::spawn(async move {
            reactor.run().await;
        });

        let join_handle = tokio::spawn(async move {
            let _ = worker_handle.await;
            let _ = reactor_handle.await;
        });

        (request_tx, response_rx, join_handle)
    }
}
