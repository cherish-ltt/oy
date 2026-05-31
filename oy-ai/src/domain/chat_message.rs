use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "function_name")]
    pub function_name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// The name of the tool that produced this message (for Tool role messages).
    /// Used by the TUI to format display per tool type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_name: Option<String>,
    /// The arguments passed to the tool (for Tool role messages).
    /// Used by the TUI to show contextual information (e.g., old/new text for Edit).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_arguments: Option<Value>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: Some(content.into()),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            function_name: None,
            tool_call_arguments: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: Some(content.into()),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            function_name: None,
            tool_call_arguments: None,
        }
    }

    pub fn assistant(
        content: Option<String>,
        reasoning_content: Option<String>,
        tool_calls: Option<Vec<ToolCall>>,
    ) -> Self {
        Self {
            role: Role::Assistant,
            content,
            reasoning_content,
            tool_calls,
            tool_call_id: None,
            function_name: None,
            tool_call_arguments: None,
        }
    }

    pub fn tool(
        content: impl Into<String>,
        tool_call_id: String,
        function_name: Option<String>,
        tool_call_arguments: Option<Value>,
    ) -> Self {
        Self {
            role: Role::Tool,
            content: Some(content.into()),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: Some(tool_call_id),
            function_name,
            tool_call_arguments,
        }
    }

    /// Serialize to the JSON wire format expected by OpenAI-compatible APIs.
    ///
    /// Key detail: `function.arguments` must be a JSON **string** (not a JSON object)
    /// per the OpenAI function calling protocol. We call `to_string()` on the Value
    /// to produce the stringified JSON that the API expects.
    pub fn to_json_value(&self) -> Value {
        let mut msg = json!({
            "role": self.role,
        });

        if let Some(ref content) = self.content {
            msg["content"] = json!(content);
        }

        if let Some(ref reasoning_content) = self.reasoning_content {
            msg["reasoning_content"] = json!(reasoning_content);
        }

        if let Some(ref tool_calls) = self.tool_calls {
            let calls: Vec<Value> = tool_calls
                .iter()
                .map(|tc| {
                    json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {
                            "name": tc.function_name,
                            "arguments": tc.arguments.to_string(),
                        }
                    })
                })
                .collect();
            msg["tool_calls"] = json!(calls);
        }

        if let Some(ref tool_call_id) = self.tool_call_id {
            msg["tool_call_id"] = json!(tool_call_id);
        }

        msg
    }

    #[cfg(test)]
    pub fn dummy() -> Self {
        Self {
            role: Role::Assistant,
            content: None,
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            function_name: None,
            tool_call_arguments: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_message_user_creation() {
        let msg = ChatMessage::user("hello");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, Some("hello".to_string()));
        assert!(msg.tool_calls.is_none());
        assert!(msg.tool_call_id.is_none());
        assert!(msg.function_name.is_none());
        assert!(msg.tool_call_arguments.is_none());
    }

    #[test]
    fn test_chat_message_system_creation() {
        let msg = ChatMessage::system("You are a helpful assistant.");
        assert_eq!(msg.role, Role::System);
        assert_eq!(msg.content.as_deref(), Some("You are a helpful assistant."));
        assert!(msg.reasoning_content.is_none());
    }

    #[test]
    fn test_chat_message_assistant_with_all_fields() {
        let msg = ChatMessage::assistant(
            Some("Hello".into()),
            Some("Let me think...".into()),
            Some(vec![ToolCall {
                id: "call_1".into(),
                function_name: "Read".into(),
                arguments: json!({"file_path": "/tmp/x.txt"}),
            }]),
        );
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.content.as_deref(), Some("Hello"));
        assert_eq!(msg.reasoning_content.as_deref(), Some("Let me think..."));
        assert_eq!(msg.tool_calls.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_chat_message_assistant_no_tool_calls() {
        let msg = ChatMessage::assistant(Some("Hi".into()), None, None);
        assert_eq!(msg.tool_calls, None);
    }

    #[test]
    fn test_chat_message_to_json_value() {
        let msg = ChatMessage::user("hello");
        let json = msg.to_json_value();
        assert_eq!(json["role"], "user");
        assert_eq!(json["content"], "hello");
        // verify tool_call_id not present when None
        assert!(json.get("tool_call_id").is_none());
    }

    #[test]
    fn test_to_json_value_with_reasoning() {
        let msg = ChatMessage::assistant(Some("answer".into()), Some("thinking...".into()), None);
        let json = msg.to_json_value();
        assert_eq!(json["reasoning_content"], "thinking...");
    }

    #[test]
    fn test_to_json_value_with_tool_call_id() {
        let msg = ChatMessage::tool("result", "call_99".to_string(), Some("Read".into()), None);
        let json = msg.to_json_value();
        assert_eq!(json["tool_call_id"], "call_99");
        assert!(json.get("content").is_some());
    }

    #[test]
    fn test_tool_call_to_json() {
        let tool_call = ToolCall {
            id: "call_123".into(),
            function_name: "Read".into(),
            arguments: json!({"file_path": "/tmp/test.txt"}),
        };
        let msg = ChatMessage::assistant(None, None, Some(vec![tool_call]));
        let json = msg.to_json_value();
        assert_eq!(json["role"], "assistant");
        let calls = json["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["id"], "call_123");
        assert_eq!(calls[0]["function"]["name"], "Read");
        // arguments must be a JSON string per OpenAI protocol
        assert_eq!(
            calls[0]["function"]["arguments"],
            "{\"file_path\":\"/tmp/test.txt\"}"
        );
    }

    #[test]
    fn test_to_json_value_reasoning_not_present() {
        let msg = ChatMessage::user("no reasoning");
        let json = msg.to_json_value();
        assert!(json.get("reasoning_content").is_none());
    }

    #[test]
    fn test_to_json_value_tool_calls_not_present() {
        let msg = ChatMessage::user("no tool calls");
        let json = msg.to_json_value();
        assert!(json.get("tool_calls").is_none());
    }

    #[test]
    fn test_chat_message_tool_creation() {
        let msg = ChatMessage::tool("file contents", "call_456".to_string(), None, None);
        assert_eq!(msg.role, Role::Tool);
        assert_eq!(msg.content, Some("file contents".to_string()));
        assert_eq!(msg.tool_call_id, Some("call_456".to_string()));
        assert_eq!(msg.function_name, None);
        assert_eq!(msg.tool_call_arguments, None);
    }

    #[test]
    fn test_chat_message_tool_with_args() {
        let msg = ChatMessage::tool(
            "Replaced 2 blocks",
            "call_789".to_string(),
            Some("Edit".into()),
            Some(json!({"old_text": "foo", "new_text": "bar"})),
        );
        assert_eq!(msg.role, Role::Tool);
        assert_eq!(msg.function_name.as_deref(), Some("Edit"));
        assert_eq!(
            msg.tool_call_arguments
                .as_ref()
                .and_then(|v| v.get("old_text")),
            Some(&json!("foo"))
        );
    }

    #[test]
    fn test_serde_roundtrip_user() {
        let original = ChatMessage::user("hello");
        let json_str = serde_json::to_string(&original).unwrap();
        let deserialized: ChatMessage = serde_json::from_str(&json_str).unwrap();
        assert_eq!(original.role, deserialized.role);
        assert_eq!(original.content, deserialized.content);
    }

    #[test]
    fn test_serde_roundtrip_with_tool_calls() {
        let original = ChatMessage::assistant(
            None,
            None,
            Some(vec![ToolCall {
                id: "call_x".into(),
                function_name: "Bash".into(),
                arguments: json!({"command": "ls"}),
            }]),
        );
        let json_str = serde_json::to_string(&original).unwrap();
        let deserialized: ChatMessage = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.tool_calls.as_ref().unwrap().len(), 1);
        assert_eq!(
            deserialized.tool_calls.as_ref().unwrap()[0].function_name,
            "Bash"
        );
    }

    #[test]
    fn test_tool_call_creation() {
        let tc = ToolCall {
            id: "call_1".into(),
            function_name: "Read".into(),
            arguments: json!({"file_path": "/tmp/x.txt"}),
        };
        assert_eq!(tc.id, "call_1");
        assert_eq!(tc.function_name, "Read");
        assert_eq!(tc.arguments["file_path"], "/tmp/x.txt");
    }
}
