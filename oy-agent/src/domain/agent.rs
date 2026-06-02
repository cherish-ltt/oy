use crate::AgentError;
use crate::domain::skill::SkillSummary;
use crate::domain::token_counter::TokenUsage;
use oy_ai::{AiProvider, ChatMessage};
use uuid::Uuid;

pub(crate) const CHANNEL_SIZE: usize = 64;

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
    fn save_session(&mut self, uuid: Uuid) -> Result<String, AgentError>;
    fn messages(&mut self) -> &[ChatMessage];
    fn max_iterations(&self) -> u32;
    fn get_system_prompt(&self, tools_description: &str) -> String;
    fn get_front_message(&self) -> Option<&ChatMessage>;
    fn get_back_message(&self) -> Option<&ChatMessage>;
    fn set_skills(&mut self, _skills: Vec<SkillSummary>) {}
}

pub enum RequestAgent {
    Prompt(String),
    SetProvider(Box<dyn AiProvider>),
    SetSkills(Vec<SkillSummary>),
}

pub enum ResponseAgent {
    Pause,
    Running,
    ChatMessage(ChatMessage),
    TokenUsage(TokenUsage),
    AgentError(AgentError),
}
