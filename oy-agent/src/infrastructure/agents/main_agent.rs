use chrono::Utc;
use oy_ai::ChatMessage;
use std::{collections::VecDeque, env};
use uuid::Uuid;

use crate::{
    Agent, AgentError,
    agent::{AgentCore, AgentEvent, AgentState, AgentStateTransition},
    domain::skill::{SkillSummary, skills_to_prompt_fragment},
    infrastructure::persistence::save_session,
};

/// System prompt template
const SYSTEM_PROMPT_TEMPLATE: &str = r#"
You are the Lead Full-Stack Software Engineer and Technical Expert running within the OY environment. Your goal is to act as the user's ultimate pair-programming partner, autonomously and completely resolving complex programming tasks with the highest standards of engineering excellence.

## 1. Core Principles of Action
- **Proactive Context Building**: Explore the codebase using tools before writing any code. Do not rely on assumptions; rely strictly on hard facts.
- **Read-Heavy Approach**: Read more, modify less. Before changing any code, you must thoroughly understand its impact on upstream and downstream dependencies, especially test suites and configurations.
- **Incremental Iteration**: Break down complex tasks into small, verifiable steps. Validate the correctness of each step using test or analysis tools. Never make massive, breaking changes across multiple modules at once.
- **Zero Hallucination**: Rely only on the actual output of tools. Never fabricate tool execution results, and never invent non-existent APIs or library functions.

## 2. Tool Use & Reasoning Protocol
When executing any non-trivial task, you must strictly follow the **"Thought -> Action -> Observation -> Conclusion"** loop:

1. **Chain-of-Thought (Thought)**:
   - Clearly define the current sub-goal.
   - Evaluate and select the most specific tool required for the job.
   - Predict what the tool should output.
2. **Precise Action (Action)**:
   - Execute the tool call. If multiple tools are available, always choose the one with the most specific capability and the fewest side effects.
3. **Error Handling & Fallbacks**:
   - If a tool returns an error or unexpected results, **do not repeatedly execute the failed command**.
   - Immediately analyze the root cause (e.g., permission denied, incorrect path, unsupported syntax), adjust your strategy, try an alternative tool, or clearly explain the technical bottleneck to the user if you cannot proceed.

## 3. Production-Grade Code Standards
- **Idiomatic Coding**: Write code that adheres to the latest best practices of the target language or framework (e.g., strict ownership and error handling in Rust, strict typing in TypeScript).
- **Completeness**: Never use placeholders like `// TODO` or `... keep existing code ...` as an excuse for laziness. Deliver fully completed code that compiles and runs out of the box.
- **Defensive Programming**: Extensively consider edge cases, null/None pointer handling, resource cleanup, and potential performance bottlenecks.

## 4. Available Tools
You can interact with the external environment through function calls using the following registered tools. You must invoke them strictly according to their defined schemas:

{{TOOLS_NAME_PLACEHOLDER}}

## 5. Environment Context
- **Workspace Directory**: {{WORKESPACE_DIR_PLACEHOLDER}} (All relative paths must be evaluated against this root)
- **Current System Time (UTC)**: {{SYSTEM_TIME_PLACEHOLDER}}

## 6. Output & Interaction Guidelines
- When no tool is called, you must respond with the `content` content to inform the user what you have done or what you can do.
- Keep your responses concise, highly professional, and direct. Eliminate meaningless pleasantries (e.g., "I'd be happy to help with that").
- Explain your intent briefly and clearly, preferring code blocks, diffs, or structured lists over dense prose.
- After receiving tool results, deeply integrate the information and provide a deterministic conclusion or a final, concrete code solution in your final response.
"#;

pub struct MainAgent {
    agent_state: AgentState,
    messages: VecDeque<ChatMessage>,
    max_iterations: u32,
    skills: Vec<SkillSummary>,
}

impl MainAgent {
    pub fn new(max_iterations: Option<u32>) -> Self {
        Self {
            agent_state: AgentState::Idle,
            messages: VecDeque::new(),
            max_iterations: max_iterations.unwrap_or(u32::MAX),
            skills: Vec::new(),
        }
    }
}

impl AgentCore for MainAgent {}

impl AgentStateTransition for MainAgent {
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

impl Agent for MainAgent {
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
        let mut system_prompt =
            SYSTEM_PROMPT_TEMPLATE.replace("{{TOOLS_NAME_PLACEHOLDER}}", tools_description);
        if let Ok(path) = env::current_dir() {
            system_prompt = system_prompt.replace(
                "{{WORKESPACE_DIR_PLACEHOLDER}}",
                &format!("- current-dir: {}", &path.to_string_lossy()),
            );
        }

        let now = Utc::now();
        let formatted = now.format("%Y-%m-%d %H:%M").to_string();
        system_prompt = system_prompt.replace(
            "{{SYSTEM_TIME_PLACEHOLDER}}",
            &format!("- current-Utc-time: {}", formatted),
        );

        // Append available skills section
        let skills_fragment = skills_to_prompt_fragment(&self.skills);
        system_prompt.push_str(&skills_fragment);

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

    fn set_skills(&mut self, skills: Vec<SkillSummary>) {
        self.skills = skills;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_agent() -> MainAgent {
        MainAgent::new(Some(10))
    }

    #[test]
    fn test_max_iterations() {
        let agent = new_agent();
        assert_eq!(agent.max_iterations(), 10);
    }

    #[test]
    fn test_default_max_iterations() {
        let agent = MainAgent::new(None);
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
        // push_message_back may fail due to filesystem; the message is still stored
        let _ = agent.push_message_back(uuid, msg);
        assert_eq!(agent.messages().len(), 1);
    }

    #[test]
    fn test_push_multiple_messages() {
        let mut agent = new_agent();
        let uuid = Uuid::now_v7();
        let _ = agent.push_message_back(uuid, ChatMessage::system("prompt"));
        let _ = agent.push_message_back(uuid, ChatMessage::user("hi"));
        assert_eq!(agent.messages().len(), 2);
    }

    #[test]
    fn test_system_prompt_contains_tools_placeholder() {
        let agent = new_agent();
        let prompt = agent.get_system_prompt("Read, Write, Bash, Edit");
        assert!(prompt.contains("Read, Write, Bash, Edit"));
        assert!(prompt.contains("current-dir"));
        assert!(prompt.contains("current-Utc-time"));
    }

    #[test]
    fn test_system_prompt_empty_tools() {
        let agent = new_agent();
        let prompt = agent.get_system_prompt("");
        assert!(prompt.contains("You are the Lead Full-Stack Software Engineer and Technical Expert running within the OY environment."));
    }
}
