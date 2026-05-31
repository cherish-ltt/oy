use oy_ai::ChatMessage;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::AgentError;

pub const CHANNEL_SIZE: usize = 64;

pub trait Agent: Send + Sync {
    fn push_message_back(&mut self, uuid: Uuid, msg: ChatMessage) -> Result<(), AgentError>;
    fn save_session(&mut self, uuid: Uuid) -> Result<String, AgentError>;
    fn messages(&mut self) -> &[ChatMessage];
    fn max_iterations(&self) -> u32;
    fn clear_messages(&mut self);
    fn get_system_prompt(&self, tools_description: &str) -> String;
}

pub enum InputAgentSignal {
    Quit,
    UserPrompt(String),
    Pause,
    ExtractContext { tx: oneshot::Sender<Vec<ChatMessage>> },
}

pub enum OutputAgentSignal {
    Pause,
    Running,
    ChatMessage(ChatMessage),
    AgentError(AgentError),
}

pub(crate) enum InputOrchestratorSignal {
    Prompt(String),
    ExtractContext { tx: oneshot::Sender<Vec<ChatMessage>> },
}

pub(crate) enum OutputOrchestratorSignal {
    Pause,
    Running,
    ChatMessage(ChatMessage),
    AgentError(AgentError),
}
