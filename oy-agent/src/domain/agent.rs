use oy_ai::ChatMessage;
use uuid::Uuid;

use crate::AgentError;

pub trait Agent: Send + Sync {
    fn push_message_back(&mut self, uuid: Uuid, msg: ChatMessage) -> Result<(), AgentError>;
    fn save_session(&mut self, uuid: Uuid) -> Result<String, AgentError>;
    fn messages(&mut self) -> &[ChatMessage];
    fn max_iterations(&self) -> u32;
    fn clear_messages(&mut self);
    fn get_system_prompt(&self, tools_description: &str) -> String;
}
