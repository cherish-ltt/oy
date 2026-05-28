use async_trait::async_trait;
use serde_json::Value;

use super::chat_message::ChatMessage;
use super::errors::AiError;

#[async_trait]
pub trait AiProvider: Send + Sync {
    async fn chat(&self, messages: &[ChatMessage], tools: &[Value])
    -> Result<ChatMessage, AiError>;
}
