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
        // Channels between TUI and Reactor
        let (request_tx, request_rx) = channel::<RequestAgent>(CHANNEL_SIZE);
        let (response_tx, response_rx) = channel::<ResponseAgent>(CHANNEL_SIZE);

        // Internal channels between Reactor and Worker
        let (worker_cmd_tx, worker_cmd_rx) = channel::<WorkerCommand>(CHANNEL_SIZE);
        let (worker_event_tx, worker_event_rx) = channel::<WorkerEvent>(CHANNEL_SIZE);

        // Create Worker (pure state machine)
        let worker = Worker::new(
            agent,
            provider,
            tool_registry,
            worker_cmd_rx,
            worker_event_tx,
        );

        // Create Reactor (scheduling layer)
        let reactor = Reactor::new(request_rx, response_tx, worker_cmd_tx, worker_event_rx);

        // Spawn both tasks
        let worker_handle = tokio::spawn(async move {
            let mut worker = worker;
            worker.run().await;
        });
        let reactor_handle = tokio::spawn(async move {
            reactor.run().await;
        });

        // Return the reactor's channels to the TUI, and a join handle that waits for both
        let join_handle = tokio::spawn(async move {
            let _ = worker_handle.await;
            let _ = reactor_handle.await;
        });

        (request_tx, response_rx, join_handle)
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
        let (request_tx, request_rx) = channel::<RequestAgent>(CHANNEL_SIZE);
        let (response_tx, response_rx) = channel::<ResponseAgent>(CHANNEL_SIZE);
        let (worker_cmd_tx, worker_cmd_rx) = channel::<WorkerCommand>(CHANNEL_SIZE);
        let (worker_event_tx, worker_event_rx) = channel::<WorkerEvent>(CHANNEL_SIZE);

        let worker = Worker::with_session(
            agent,
            provider,
            tool_registry,
            worker_cmd_rx,
            worker_event_tx,
            session_uuid,
            session_messages,
        );

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
