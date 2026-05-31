use chrono::Utc;
use oy_ai::ChatMessage;
use std::{collections::VecDeque, env};
use uuid::Uuid;

use crate::{Agent, AgentError, infrastructure::persistence::save_session};

/// 系统提示词模板，使用 `{{TOOLS_NAME_PLACEHOLDER}}` 作为工具列表的占位符
const SYSTEM_PROMPT_TEMPLATE: &str = r#"
你是一个在OY中运行的有用的专业智能编程助手, 可以访问多种外部工具。
你的目标是通过逐步推理并在必要时调用适当工具来帮助用户。

## Guidelines

- 在调用工具之前，务必仔细思考问题。
- 如果工具调用失败，请尝试调用其他工具，如无其他可用工具, 说明原因。
- 当有多个工具可用时，选择与当前子任务最相关的那个工具。
- 不要伪造工具输出结果，应依赖实际执行结果。
- 收到工具结果后，整合相关信息，为用户提供最终答案。

## Available Tools

以下工具已注册，可以通过函数调用使用。

{{TOOLS_NAME_PLACEHOLDER}}

## Additional Instructions

- 保持回复简洁且有帮助, 拒绝捏造事实。
- 如果不需要工具，请直接根据您的知识回答。

## Work directory information

{{WORKESPACE_DIR_PLACEHOLDER}}
{{SYSTEM_TIME_PLACEHOLDER}}
"#;

#[derive(Debug)]
pub struct MainAgent {
    messages: VecDeque<ChatMessage>,
    max_iterations: u32,
}

impl MainAgent {
    pub fn new_with_max_iterations(max_iterations: Option<u32>) -> Self {
        Self {
            messages: VecDeque::new(),
            max_iterations: max_iterations.unwrap_or(u32::MAX),
        }
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

    fn clear_messages(&mut self) {
        self.messages.clear();
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_agent() -> MainAgent {
        MainAgent::new_with_max_iterations(Some(10))
    }

    #[test]
    fn test_max_iterations() {
        let agent = new_agent();
        assert_eq!(agent.max_iterations(), 10);
    }

    #[test]
    fn test_default_max_iterations() {
        let agent = MainAgent::new_with_max_iterations(None);
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
    fn test_clear_messages() {
        let mut agent = new_agent();
        let uuid = Uuid::now_v7();
        let _ = agent.push_message_back(uuid, ChatMessage::user("hello"));
        agent.clear_messages();
        assert!(agent.messages().is_empty());
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
        assert!(prompt.contains("## Available Tools"));
    }
}
