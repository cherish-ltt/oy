use oy_agent::agent::{InputAgentSignal, OutputAgentSignal};
use tokio::{sync::mpsc, task::JoinHandle};

#[derive(Debug)]
pub(crate) struct AgentManager {
    pub name: String,
    pub handle: JoinHandle<()>,
    pub request_sender: mpsc::Sender<InputAgentSignal>,
    pub response_receiver: Option<mpsc::Receiver<OutputAgentSignal>>,
}

impl AgentManager {
    pub fn new(
        name: String,
        handle: JoinHandle<()>,
        request_sender: mpsc::Sender<InputAgentSignal>,
        response_receiver: mpsc::Receiver<OutputAgentSignal>,
    ) -> Self {
        Self {
            name,
            handle,
            request_sender,
            response_receiver: Some(response_receiver),
        }
    }
}
