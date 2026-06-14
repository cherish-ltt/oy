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

        // Add reasoning_effort if configured (none/low/medium/high/xhigh)
        if let Some(ref effort) = self.config.reasoning_effort {
            body["reasoning_effort"] = json!(effort);
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

        // Check for API-level error in response body even when HTTP status is 200
        if let Some(error_val) = response.get("error") {
            let error_msg = error_val
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown API error");
            return Err(AiError::ApiError(format!(
                "API returned error: {}",
                error_msg
            )));
        }

        parse_chat_response(&response)
    }
}

/// Parse the OpenRouter API response into a ChatMessage.
///
/// Returns `AiError::ApiError` if the response is missing required fields
/// such as `choices` array or `message` in the first choice.
fn parse_chat_response(response: &Value) -> Result<ChatMessage, AiError> {
    let message = extract_message(response)?;

    let content = message
        .get("content")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let reasoning_content = message
        .get("reasoning_content")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let tool_calls = parse_tool_calls(message);

    Ok(ChatMessage {
        role: crate::domain::chat_message::Role::Assistant,
        content,
        reasoning_content,
        tool_calls,
        tool_call_id: None,
        function_name: None,
        tool_call_arguments: None,
    })
}

/// Extract the first message from a valid API response.
fn extract_message(response: &Value) -> Result<&Value, AiError> {
    let choices = response
        .get("choices")
        .and_then(|c| c.as_array())
        .filter(|c| !c.is_empty())
        .ok_or_else(|| {
            AiError::ApiError(format!(
                "API response missing valid 'choices' array: {}",
                serde_json::to_string(response).unwrap_or_default()
            ))
        })?;
    choices[0].get("message").ok_or_else(|| {
        AiError::ApiError(format!(
            "API response missing 'message' in choices[0]: {}",
            serde_json::to_string(response).unwrap_or_default()
        ))
    })
}

/// Extract tool calls from a message value, if present.
fn parse_tool_calls(message: &Value) -> Option<Vec<ToolCall>> {
    message
        .get("tool_calls")
        .and_then(|v| v.as_array())
        .map(|calls| {
            calls
                .iter()
                .map(|tc| {
                    let args: Value = tc
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(|a| a.as_str())
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or(json!({}));
                    ToolCall {
                        id: tc
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        function_name: tc
                            .get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        arguments: args,
                    }
                })
                .collect()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_valid_response_with_content() {
        let resp = json!({
            "choices": [{
                "message": {
                    "content": "Hello!",
                    "reasoning_content": "Let me think..."
                }
            }]
        });
        let result = parse_chat_response(&resp);
        assert!(result.is_ok());
        let msg = result.unwrap();
        assert_eq!(msg.content.as_deref(), Some("Hello!"));
        assert_eq!(msg.reasoning_content.as_deref(), Some("Let me think..."));
        assert!(msg.tool_calls.is_none());
    }

    #[test]
    fn test_parse_missing_choices() {
        let resp = json!({});
        let result = parse_chat_response(&resp);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            format!("{}", err).contains("choices"),
            "Error should mention 'choices': {}",
            err
        );
    }

    #[test]
    fn test_parse_empty_choices() {
        let resp = json!({"choices": []});
        let result = parse_chat_response(&resp);
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("choices"));
    }

    #[test]
    fn test_parse_missing_message() {
        let resp = json!({"choices": [{}]});
        let result = parse_chat_response(&resp);
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("message"));
    }

    #[test]
    fn test_parse_with_tool_calls() {
        let resp = json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "Read",
                            "arguments": "{\"file_path\": \"/tmp/x.txt\"}"
                        }
                    }]
                }
            }]
        });
        let result = parse_chat_response(&resp);
        assert!(result.is_ok());
        let msg = result.unwrap();
        assert!(msg.content.is_none());
        assert!(msg.tool_calls.is_some());
        let calls = msg.tool_calls.unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_name, "Read");
        assert_eq!(calls[0].id, "call_1");
    }

    #[test]
    fn test_error_response_field_in_parse() {
        // Even if error is present, parse_chat_response only looks at choices.
        // This tests that a valid response with error field still parses correctly
        // (the error check happens in chat() before calling parse_chat_response).
        let resp = json!({
            "error": {"message": "some error"},
            "choices": [{
                "message": {
                    "content": "valid content"
                }
            }]
        });
        let result = parse_chat_response(&resp);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().content.as_deref(), Some("valid content"));
    }
}
