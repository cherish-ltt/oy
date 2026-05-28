use async_openai::{Client, config::OpenAIConfig};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::domain::ai_provider::AiProvider;
use crate::domain::chat_message::{ChatMessage, ToolCall};
use crate::domain::config::AiConfig;
use crate::domain::errors::AiError;

pub struct OpenCodeGoProvider {
    config: AiConfig,
    client: Client<OpenAIConfig>,
}

impl OpenCodeGoProvider {
    pub fn new(config: AiConfig) -> Self {
        let openai_config = OpenAIConfig::new()
            .with_api_base(&config.base_url)
            .with_api_key(&config.api_key);
        let client = Client::with_config(openai_config);
        Self { config, client }
    }
}

#[async_trait]
impl AiProvider for OpenCodeGoProvider {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
    ) -> Result<ChatMessage, AiError> {
        let messages_json: Vec<Value> = messages.iter().map(|m| m.to_json_value()).collect();

        let mut body = json!({
            "messages": messages_json,
            "model": self.config.model,
        });

        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }

        // Use create_byot (Bring Your Own Token) to send a raw JSON body directly.
        // This is required because OpenRouter exposes an OpenAI-compatible API but we
        // construct the request body manually to support tool calling with dynamic
        // tool schemas that are not known at compile time.
        let response: Value = self
            .client
            .chat()
            .create_byot(body)
            .await
            .map_err(|e| AiError::ApiError(e.to_string()))?;

        let message = &response["choices"][0]["message"];

        let role = match message["role"].as_str() {
            Some("assistant") => crate::domain::chat_message::Role::Assistant,
            _ => crate::domain::chat_message::Role::Assistant,
        };

        let content = message["content"].as_str().map(|s| s.to_string());
        let reasoning_content = message["reasoning_content"].as_str().map(|s| s.to_string());

        let tool_calls = message["tool_calls"].as_array().map(|calls| {
            calls
                .iter()
                .map(|tc| {
                    let arguments_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                    let arguments: Value = serde_json::from_str(arguments_str).unwrap_or(json!({}));
                    ToolCall {
                        id: tc["id"].as_str().unwrap_or_default().to_string(),
                        function_name: tc["function"]["name"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        arguments,
                    }
                })
                .collect()
        });

        Ok(ChatMessage {
            role,
            content,
            reasoning_content,
            tool_calls,
            tool_call_id: None,
        })
    }
}
