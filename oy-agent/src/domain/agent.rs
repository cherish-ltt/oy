use std::collections::VecDeque;

use crate::AgentError;
use crate::domain::skill::SkillSummary;
use crate::domain::token_counter::TokenUsage;
use oy_ai::{AiProvider, ChatMessage, Role};
use tokio::sync::oneshot;
use uuid::Uuid;

pub(crate) const CHANNEL_SIZE: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum AgentState {
    Idle,
    Thinking,
    Acting,
    ToolCall,
    Observing,
}

pub enum AgentEvent {
    Start,
    ThinkingCompleted,
    ToolCallsRequired,
    ToolCallCompleted,
    TaskCompleted,
    Reset,
    Error,
    StartOver,
}

pub trait AgentCore: Agent + AgentStateTransition {}

pub trait AgentStateTransition {
    fn next_state(&self, current: &AgentState, event: &AgentEvent) -> Option<AgentState>;
    fn current_state(&self) -> &AgentState;
    fn set_state(&mut self, state: AgentState);
}

pub trait Agent: Send + Sync {
    fn push_message_back(&mut self, uuid: Uuid, msg: ChatMessage) -> Result<(), AgentError>;
    fn insert_message_front(&mut self, uuid: Uuid, msg: ChatMessage) -> Result<(), AgentError> {
        // 默认空实现（RequestAgent/ResponseAgent 不需要此功能）
        let _ = (uuid, msg);
        Ok(())
    }
    fn save_session(&mut self, uuid: Uuid) -> Result<String, AgentError>;
    fn messages(&mut self) -> &[ChatMessage];
    fn max_iterations(&self) -> u32;
    fn get_system_prompt(&self, tools_description: &str) -> String;
    fn get_front_message(&self) -> Option<&ChatMessage>;
    fn get_back_message(&self) -> Option<&ChatMessage>;
    fn set_skills(&mut self, _skills: Vec<SkillSummary>) {}
    /// Replace internal message list (used when restoring history via channel).
    fn replace_messages(&mut self, _msgs: Vec<ChatMessage>) {}
}

/// Preserve own system prompt and skip source's system prompt in replace_messages.
///
/// - If the agent has its own System prompt (messages.front is Some with Role::System):
///   keep it, skip the source's first System message (replaced by own).
/// - If the agent has NO messages (empty): keep all source messages as-is,
///   do NOT skip the source's first System message.
pub(crate) fn replace_messages_preserve_system_prompt(
    messages: &mut VecDeque<ChatMessage>,
    msgs: Vec<ChatMessage>,
) {
    let system_prompt = messages.front().cloned();
    messages.clear();
    let mut iter = msgs.into_iter();

    if let Some(sys) = system_prompt {
        // We have our own System prompt: skip source's first System (ours replaces it)
        if let Some(msg) = iter.next()
            && msg.role != Role::System
        {
            messages.push_back(msg);
        }
        messages.push_front(sys);
    }
    // No own System prompt: keep all source messages (including any System),
    // the first source System is NOT skipped.

    messages.extend(iter);
}

#[derive(Debug, Clone)]
pub enum PromptKind {
    Enter,
    AltEnter,
}

#[derive(Debug, Clone)]
pub struct PromptRequest {
    pub text: String,
    pub id: Uuid,
    pub kind: PromptKind,
}

pub enum RequestAgent {
    Prompt {
        text: String,
        id: Uuid,
        kind: PromptKind,
    },
    CancelPrompt {
        id: Uuid,
    },
    SetProvider(Box<dyn AiProvider>),
    SetSkills(Vec<SkillSummary>),
    /// Request the agent's current message list, sent back via the oneshot.
    GetMessages {
        tx: oneshot::Sender<Vec<ChatMessage>>,
    },
    /// Replace the agent's message list with this history.
    SetMessages(Vec<ChatMessage>),
}

pub enum ResponseAgent {
    Pause,
    Running,
    ChatMessage(ChatMessage),
    TokenUsage(TokenUsage),
    AgentError(AgentError),
    PromptConsumed { id: Uuid },
    PromptQueued { id: Uuid },
}
