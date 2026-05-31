use oy_agent::oy_ai::{ChatMessage, Role};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

#[derive(Debug)]
pub enum Message {
    UiMessages(String),
    AgentMessages(ChatMessage),
    AgentStatus(Status),
}

impl Message {
    pub fn to_lines(&self) -> Vec<Line<'_>> {
        match self {
            Message::UiMessages(text) => {
                vec![Line::from(Span::styled(
                    format!("> {}", text),
                    Style::default().fg(Color::Blue).bold(),
                ))]
            }
            Message::AgentMessages(chat_message) => {
                let role_style = match chat_message.role {
                    Role::User => Style::default().fg(Color::Blue),
                    Role::Assistant => Style::default().fg(Color::Green),
                    Role::Tool => Style::default().fg(Color::Magenta),
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

                // 如果是工具调用，可以额外显示详细信息
                if let Some(tool_calls) = &chat_message.tool_calls {
                    for tool in tool_calls {
                        lines.push(Line::from(Span::styled(
                            format!("  🔧 调用工具: {}", tool.function_name),
                            Style::default().fg(Color::Cyan),
                        )));
                        lines.push(Line::from(Span::styled(
                            format!("     参数: {}", tool.arguments),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                }

                lines
            }
            Message::AgentStatus(status) => match status {
                Status::Pause => vec![Line::from(Span::styled(
                    format!("> pause"),
                    Style::default().fg(Color::Gray).bold(),
                ))],
                Status::Running => vec![Line::from(Span::styled(
                    format!("> running"),
                    Style::default().fg(Color::Green).bold(),
                ))],
            },
        }
    }

    /// 估算此消息在给定内容宽度下渲染后所占的视觉行数（含自动换行）
    pub fn visual_line_count(&self, width: usize) -> usize {
        if width == 0 {
            return 1;
        }
        match self {
            Message::UiMessages(text) => {
                let line = format!("> {}", text);
                let w = UnicodeWidthStr::width(line.as_str());
                1.max((w + width - 1) / width)
            }
            Message::AgentMessages(chat) => {
                let mut count = 0usize;
                if let Some(ref r) = chat.reasoning_content {
                    let line = format!("[{:#?} - thinking] {}", chat.role, r);
                    let w = UnicodeWidthStr::width(line.as_str());
                    count += 1.max((w + width - 1) / width);
                }
                if let Some(ref c) = chat.content {
                    let line = format!("[{:#?} - content] {}", chat.role, c);
                    let w = UnicodeWidthStr::width(line.as_str());
                    count += 1.max((w + width - 1) / width);
                }
                if let Some(ref tools) = chat.tool_calls {
                    for tool in tools {
                        let name_line = format!("  🔧 调用工具: {}", tool.function_name);
                        let w = UnicodeWidthStr::width(name_line.as_str());
                        count += 1.max((w + width - 1) / width);
                        let args_line = format!("     参数: {}", tool.arguments);
                        let w = UnicodeWidthStr::width(args_line.as_str());
                        count += 1.max((w + width - 1) / width);
                    }
                }
                count
            }
            Message::AgentStatus(_) => 1,
        }
    }
}

#[derive(Debug)]
pub enum Status {
    Pause,
    Running,
}
