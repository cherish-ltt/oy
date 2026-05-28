use std::sync::Mutex;

use async_trait::async_trait;
use oy_agent::infrastructure::agents::main_agent::MainAgent;
use oy_agent::infrastructure::tools::ToolRegistry;
use oy_agent::infrastructure::tools::read::ReadTool;
use oy_agent::{AgentError, Orchestrator};
use oy_ai::{AiProvider, ChatMessage, ToolCall};
use serde_json::Value;

/// A mock provider that returns pre-defined responses in sequence.
struct MockProvider {
    responses: Mutex<Vec<ChatMessage>>,
}

impl MockProvider {
    fn new(responses: Vec<ChatMessage>) -> Self {
        Self {
            responses: Mutex::new(responses),
        }
    }
}

#[async_trait]
impl AiProvider for MockProvider {
    async fn chat(
        &self,
        _messages: &[ChatMessage],
        _tools: &[Value],
    ) -> Result<ChatMessage, oy_ai::AiError> {
        let mut guard = self.responses.lock().unwrap();
        if guard.is_empty() {
            return Err(oy_ai::AiError::ApiError("No more mock responses".into()));
        }
        Ok(guard.remove(0))
    }
}

#[tokio::test]
async fn test_orchestrator_direct_response() {
    let provider = MockProvider::new(vec![ChatMessage::assistant(
        Some("Hello, world!".into()),
        None,
        None,
    )]);
    let registry = ToolRegistry::new();
    let main_agent = MainAgent::new_with_max_iterations(Some(100));
    let mut orchestrator = Orchestrator::new(main_agent, provider, registry);

    let result = orchestrator.execute("Say hi").await.unwrap();
    assert_eq!(result, "Hello, world!");
}

#[tokio::test]
async fn test_orchestrator_single_tool_call() {
    let provider = MockProvider::new(vec![
        ChatMessage::assistant(
            None,
            None,
            Some(vec![ToolCall {
                id: "call_1".into(),
                function_name: "Read".into(),
                arguments: serde_json::json!({"file_path": "/nonexistent/file.txt"}),
            }]),
        ),
        ChatMessage::assistant(Some("File content retrieved".into()), None, None),
    ]);
    let mut registry = ToolRegistry::new();
    registry.register(ReadTool);
    let main_agent = MainAgent::new_with_max_iterations(Some(100));
    let mut orchestrator = Orchestrator::new(main_agent, provider, registry);

    let result = orchestrator.execute("Read a file").await.unwrap();
    assert_eq!(result, "File content retrieved");
}

#[tokio::test]
async fn test_orchestrator_max_iterations() {
    let provider = MockProvider::new(vec![
        ChatMessage::assistant(
            None,
            None,
            Some(vec![ToolCall {
                id: "call_1".into(),
                function_name: "Read".into(),
                arguments: serde_json::json!({"file_path": "/nonexistent/file.txt"}),
            }]),
        ),
        ChatMessage::assistant(
            None,
            None,
            Some(vec![ToolCall {
                id: "call_2".into(),
                function_name: "Read".into(),
                arguments: serde_json::json!({"file_path": "/nonexistent/file.txt"}),
            }]),
        ),
    ]);
    let mut registry = ToolRegistry::new();
    registry.register(ReadTool);
    let main_agent = MainAgent::new_with_max_iterations(Some(2));
    let mut orchestrator = Orchestrator::new(main_agent, provider, registry);

    let err = orchestrator.execute("Loop").await.unwrap_err();
    assert!(matches!(err, AgentError::MaxIterationsReached));
}
