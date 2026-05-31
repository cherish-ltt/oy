use oy_agent::{
    agent::{InputAgentSignal, OutputAgentSignal},
    oy_ai::ChatMessage,
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

#[derive(Debug)]
pub struct AgentManager {
    pub name: String,
    pub _handle: JoinHandle<()>,
    pub request_sender: mpsc::Sender<InputAgentSignal>,
    pub response_receiver: Option<mpsc::Receiver<OutputAgentSignal>>,
}

impl AgentManager {
    /// Send ExtractContext signal and await the messages.
    pub async fn extract_messages(&self) -> Vec<ChatMessage> {
        let (tx, rx) = oneshot::channel();
        let _ = self
            .request_sender
            .send(InputAgentSignal::ExtractContext { tx })
            .await;
        rx.await.unwrap_or_default()
    }

    pub fn new(
        name: String,
        handle: JoinHandle<()>,
        request_sender: mpsc::Sender<InputAgentSignal>,
        response_receiver: mpsc::Receiver<OutputAgentSignal>,
    ) -> Self {
        Self {
            name,
            _handle: handle,
            request_sender,
            response_receiver: Some(response_receiver),
        }
    }
}
