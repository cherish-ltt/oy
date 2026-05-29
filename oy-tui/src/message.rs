use oy_agent::oy_ai::{ChatMessage, Role};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

#[derive(Debug)]
pub enum Message {
    UiMessages(String),
    AgentMessages(ChatMessage),
}

impl Message {
    pub fn to_lines(&self) -> Vec<Line<'_>> {
        match self {
            Message::UiMessages(text) => {
                vec![Line::from(Span::styled(
                    format!("> {}", text),
                    Style::default().fg(Color::Black).bold(),
                ))]
            }
            Message::AgentMessages(chat_message) => {
                let role_style = match chat_message.role {
                    Role::User => Style::default().fg(Color::Cyan).bg(Color::DarkGray),
                    Role::Assistant => Style::default().fg(Color::DarkGray),
                    Role::Tool => Style::default().fg(Color::Yellow).bg(Color::DarkGray),
                    Role::System => return Vec::new(),
                };
                let mut lines = Vec::new();

                // think内容行
                if let Some(reasoning_content) = &chat_message.reasoning_content {
                    lines.push(Line::from(Span::styled(
                        format!(
                            "[{:#?} - thinking] {}",
                            chat_message.role, reasoning_content
                        ),
                        role_style.add_modifier(Modifier::ITALIC),
                    )));
                }
                // 普通内容行
                if let Some(content) = &chat_message.content {
                    lines.push(Line::from(Span::styled(
                        format!("[{:#?} - content] {}", chat_message.role, content),
                        role_style,
                    )));
                }

                // 如果是工具调用，可以额外显示详细信息（带深色背景）
                if let Some(tool_calls) = &chat_message.tool_calls {
                    for tool in tool_calls {
                        lines.push(Line::from(Span::styled(
                            format!("  🔧 调用工具: {}", tool.function_name),
                            Style::default().fg(Color::White).bg(Color::DarkGray),
                        )));
                        lines.push(Line::from(Span::styled(
                            format!("     参数: {}", tool.arguments),
                            Style::default().fg(Color::Gray).bg(Color::DarkGray),
                        )));
                    }
                }

                lines
            }
        }
    }

    // 返回此消息占用的总行数（用于滚动计算）
    // fn line_count(&self) -> usize {
    //     // 简单实现：对于 UiMessages 固定 1 行
    //     // 对于 AgentMessages，需要根据实际产生的行数计算
    //     match self {
    //         Message::UiMessages(_) => 1,
    //         Message::AgentMessages(chat) => {
    //             let mut count = 1;
    //             if let Some(tool_calls) = &chat.tool_calls {
    //                 count += tool_calls.len() * 2; // 每个工具占两行（名称+参数）
    //             }
    //             count
    //         }
    //     }
    // }
}
