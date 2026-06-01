use oy_agent::agent::{RequestAgent, ResponseAgent};
use tokio::{sync::mpsc, task::JoinHandle};

#[derive(Debug)]
pub struct AgentManager {
    pub name: String,
    pub _handle: JoinHandle<()>,
    pub request_sender: mpsc::Sender<RequestAgent>,
    pub response_receiver: Option<mpsc::Receiver<ResponseAgent>>,
}

impl AgentManager {
    pub fn new(
        name: String,
        handle: JoinHandle<()>,
        request_sender: mpsc::Sender<RequestAgent>,
        response_receiver: mpsc::Receiver<ResponseAgent>,
    ) -> Self {
        Self {
            name,
            _handle: handle,
            request_sender,
            response_receiver: Some(response_receiver),
        }
    }
}
