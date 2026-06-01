use crate::agent::{AgentCore, CHANNEL_SIZE, RequestAgent, ResponseAgent};
use crate::infrastructure::agents::AgentLoop;
use crate::infrastructure::tools::ToolRegistry;
use oy_ai::AiProvider;
use tokio::sync::mpsc::{Receiver, Sender, channel};
use tokio::task::JoinHandle;

pub struct Orchestrator {
    agent_loop: AgentLoop,
}

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
        let (request_tx, request_rx) = channel::<RequestAgent>(CHANNEL_SIZE);
        let (response_tx, response_rx) = channel::<ResponseAgent>(CHANNEL_SIZE);
        let agent_loop = AgentLoop::new(agent, provider, tool_registry, request_rx, response_tx);

        let orchestrator = Self { agent_loop };
        let join_handle = orchestrator.run();

        (request_tx, response_rx, join_handle)
    }

    fn run(mut self) -> JoinHandle<()> {
        tokio::spawn(async move {
            self.agent_loop.run().await;
        })
    }
}
