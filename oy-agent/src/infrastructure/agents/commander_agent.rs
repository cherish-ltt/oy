use std::{collections::VecDeque, env};
use uuid::Uuid;

use oy_ai::ChatMessage;

use crate::{
    Agent, AgentError,
    agent::{AgentCore, AgentEvent, AgentState, AgentStateTransition},
    domain::sub_agent::COMMANDER_SYSTEM_PROMPT,
    infrastructure::persistence::save_session,
};

/// CommanderAgent — the top-level orchestration agent.
///
/// Unlike MainAgent which directly uses file tools (Read/Write/Bash/Edit),
/// CommanderAgent only uses meta-tools (create_sub_agent) to delegate work
/// to sub-agents (Planner/Worker/Reviewer/GitHelper).
pub struct CommanderAgent {
    agent_state: AgentState,
    messages: VecDeque<ChatMessage>,
    max_iterations: u32,
}

impl CommanderAgent {
    pub fn new(max_iterations: Option<u32>) -> Self {
        Self {
            agent_state: AgentState::Idle,
            messages: VecDeque::new(),
            max_iterations: max_iterations.unwrap_or(u32::MAX),
        }
    }
}

impl AgentCore for CommanderAgent {}

impl AgentStateTransition for CommanderAgent {
    fn next_state(&self, current: &AgentState, event: &AgentEvent) -> Option<AgentState> {
        match (current, event) {
            (AgentState::Idle, AgentEvent::Start) => Some(AgentState::Thinking),
            (AgentState::Thinking, AgentEvent::ThinkingCompleted) => Some(AgentState::Acting),
            (AgentState::Thinking, AgentEvent::Error) => Some(AgentState::Idle),
            (AgentState::Thinking, AgentEvent::StartOver) => Some(AgentState::Idle),
            (AgentState::Acting, AgentEvent::ToolCallsRequired) => Some(AgentState::ToolCall),
            (AgentState::Acting, AgentEvent::TaskCompleted) => Some(AgentState::Observing),
            (AgentState::Acting, AgentEvent::Error) => Some(AgentState::Idle),
            (AgentState::Acting, AgentEvent::StartOver) => Some(AgentState::Idle),
            (AgentState::ToolCall, AgentEvent::ToolCallCompleted) => Some(AgentState::Thinking),
            (AgentState::ToolCall, AgentEvent::Error) => Some(AgentState::Idle),
            (AgentState::ToolCall, AgentEvent::StartOver) => Some(AgentState::Idle),
            (AgentState::Observing, AgentEvent::Reset) => Some(AgentState::Idle),
            (AgentState::Observing, AgentEvent::Error) => Some(AgentState::Idle),
            (AgentState::Observing, AgentEvent::StartOver) => Some(AgentState::Idle),
            _ => None,
        }
    }

    fn current_state(&self) -> &AgentState {
        &self.agent_state
    }

    fn set_state(&mut self, state: AgentState) {
        self.agent_state = state;
    }
}

impl Agent for CommanderAgent {
    fn push_message_back(&mut self, uuid: Uuid, msg: ChatMessage) -> Result<(), AgentError> {
        self.messages.push_back(msg);
        match self.save_session(uuid) {
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn messages(&mut self) -> &[ChatMessage] {
        self.messages.make_contiguous()
    }

    fn max_iterations(&self) -> u32 {
        self.max_iterations
    }

    fn get_system_prompt(&self, tools_description: &str) -> String {
        let mut system_prompt = COMMANDER_SYSTEM_PROMPT.to_string();

        // Append tool descriptions
        if !tools_description.is_empty() {
            system_prompt.push_str("\n\n## 可用工具\n");
            system_prompt.push_str(tools_description);
        }

        system_prompt
    }

    fn save_session(&mut self, uuid: Uuid) -> Result<String, AgentError> {
        match env::current_dir() {
            Ok(path) => {
                let path = path.to_string_lossy().to_string().replace("/", "-");
                save_session(uuid, self.messages().iter().collect(), &path)
            }
            Err(e) => Err(AgentError::ToolExecutionError(e.to_string())),
        }
    }

    fn get_front_message(&self) -> Option<&ChatMessage> {
        self.messages.front()
    }

    fn get_back_message(&self) -> Option<&ChatMessage> {
        self.messages.back()
    }

    fn replace_messages(&mut self, msgs: Vec<ChatMessage>) {
        self.messages.clear();
        for msg in msgs {
            self.messages.push_back(msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_agent() -> CommanderAgent {
        CommanderAgent::new(Some(10))
    }

    #[test]
    fn test_max_iterations() {
        let agent = new_agent();
        assert_eq!(agent.max_iterations(), 10);
    }

    #[test]
    fn test_default_max_iterations() {
        let agent = CommanderAgent::new(None);
        assert_eq!(agent.max_iterations(), u32::MAX);
    }

    #[test]
    fn test_initial_messages_empty() {
        let mut agent = new_agent();
        assert!(agent.messages().is_empty());
    }

    #[test]
    fn test_push_message() {
        let mut agent = new_agent();
        let uuid = Uuid::now_v7();
        let msg = ChatMessage::user("hello");
        let _ = agent.push_message_back(uuid, msg);
        assert_eq!(agent.messages().len(), 1);
    }

    #[test]
    fn test_system_prompt_contains_tools() {
        let agent = new_agent();
        let prompt = agent.get_system_prompt("create_sub_agent");
        assert!(prompt.contains("create_sub_agent"));
        assert!(prompt.contains("CommanderAgent"));
    }
}
