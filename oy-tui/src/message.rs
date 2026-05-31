use oy_agent::oy_ai::{ChatMessage, Role};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

/// Maximum lines to show for a read tool result (when collapsed)
const MAX_READ_LINES: usize = 200;
/// Maximum lines to show for a bash tool result (when collapsed) — shows the *last* N lines
const MAX_BASH_LINES: usize = 5;

#[derive(Debug)]
pub enum Message {
    UiMessages(String),
    AgentMessages(ChatMessage, bool), // bool = expanded
    AgentStatus(Status),
}

impl Message {
    /// Build the styled lines for display.
    /// For Tool messages with a known function_name, content is truncated per tool type
    /// unless `expanded` is true.
    pub fn to_lines(&self) -> Vec<Line<'_>> {
        match self {
            Message::UiMessages(text) => {
                vec![Line::from(Span::styled(
                    format!("> {}", text),
                    Style::default().fg(Color::Blue).bold(),
                ))]
            }
            Message::AgentMessages(chat_message, expanded) => {
                let role_style = match chat_message.role {
                    Role::User => Style::default().fg(Color::Blue),
                    Role::Assistant => Style::default().fg(Color::Green),
                    Role::Tool => Style::default().fg(Color::Magenta),
                    Role::System => return Vec::new(),
                };
                let mut lines = Vec::new();

                // reasoning content (e.g. thinking)
                if let Some(reasoning_content) = &chat_message.reasoning_content {
                    lines.push(Line::from(Span::styled(
                        format!(
                            "[{:#?} - thinking] {}",
                            chat_message.role, reasoning_content
                        ),
                        role_style.add_modifier(Modifier::ITALIC),
                    )));
                }

                // ----- Tool result: per-tool-type formatting -----
                if chat_message.role == Role::Tool {
                    if let Some(fn_name) = &chat_message.function_name {
                        match fn_name.as_str() {
                            "Read" => {
                                Self::add_read_lines(&mut lines, chat_message, *expanded, &role_style);
                            }
                            "Bash" => {
                                Self::add_bash_lines(&mut lines, chat_message, *expanded, &role_style);
                            }
                            "Edit" => {
                                Self::add_edit_lines(&mut lines, chat_message, &role_style);
                            }
                            "Write" => {
                                Self::add_write_lines(&mut lines, chat_message, &role_style);
                            }
                            _ => {
                                // Unknown tool: show content normally
                                Self::add_content_lines(&mut lines, chat_message, &role_style);
                            }
                        }
                    } else {
                        // No function_name — show as plain content
                        Self::add_content_lines(&mut lines, chat_message, &role_style);
                    }
                } else {
                    // Non-tool messages: show content normally
                    if let Some(content) = &chat_message.content {
                        lines.push(Line::from(Span::styled(
                            format!("[{:#?} - content] {}", chat_message.role, content),
                            role_style,
                        )));
                    }
                }

                // Tool calls (for Assistant messages)
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
                    "> pause",
                    Style::default().fg(Color::Gray).bold(),
                ))],
                Status::Running => vec![Line::from(Span::styled(
                    "> running",
                    Style::default().fg(Color::Green).bold(),
                ))],
            },
        }
    }

    /// ── Tool-type-specific formatting helpers ──────────────────────

    /// Read: show first MAX_READ_LINES lines, then a truncation hint
    fn add_read_lines(
        lines: &mut Vec<Line<'static>>,
        msg: &ChatMessage,
        expanded: bool,
        style: &Style,
    ) {
        let content = msg.content.as_deref().unwrap_or("");
        let all_lines: Vec<&str> = content.lines().collect();
        let total = all_lines.len();

        let display_lines: Vec<&str> = if expanded || total <= MAX_READ_LINES {
            all_lines
        } else {
            all_lines[..MAX_READ_LINES].to_vec()
        };

        for line in &display_lines {
            lines.push(Line::from(Span::styled(
                format!("[Tool - Read] {}", line),
                *style,
            )));
        }

        if !expanded && total > MAX_READ_LINES {
            let hidden = total - MAX_READ_LINES;
            lines.push(Line::from(Span::styled(
                format!("... ({} more lines, ctrl+o to expand) ", hidden),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
            )));
        }
    }

    /// Bash: show last MAX_BASH_LINES lines, with a truncation header
    fn add_bash_lines(
        lines: &mut Vec<Line<'static>>,
        msg: &ChatMessage,
        expanded: bool,
        style: &Style,
    ) {
        let content = msg.content.as_deref().unwrap_or("");
        let all_lines: Vec<&str> = content.lines().collect();
        let total = all_lines.len();

        if !expanded && total > MAX_BASH_LINES {
            let hidden = total - MAX_BASH_LINES;
            lines.push(Line::from(Span::styled(
                format!("... ({} earlier lines, ctrl+o to expand) ", hidden),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
            )));
            for line in &all_lines[total - MAX_BASH_LINES..] {
                lines.push(Line::from(Span::styled(
                    format!("[Tool - Bash] {}", line),
                    *style,
                )));
            }
        } else {
            for line in &all_lines {
                lines.push(Line::from(Span::styled(
                    format!("[Tool - Bash] {}", line),
                    *style,
                )));
            }
        }
    }

    /// Edit: show old text (red) → new text (green), plus file path
    fn add_edit_lines(
        lines: &mut Vec<Line<'static>>,
        msg: &ChatMessage,
        style: &Style,
    ) {
        // Try to extract old_text & new_text from arguments
        let (old_text, new_text) = msg
            .tool_call_arguments
            .as_ref()
            .map(|args| {
                let old = args
                    .get("old_text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();
                let new = args
                    .get("new_text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();
                (old, new)
            })
            .unwrap_or(("?".to_string(), "?".to_string()));

        // Use the result content (success/error message) as a header
        if let Some(result) = &msg.content {
            // Try to produce a compact one-liner if the result is a success message
            if result.starts_with("Successfully replaced") || result.starts_with("No changes made") {
                // Show file path on its own row
                lines.push(Line::from(Span::styled(
                    format!("[Tool - Edit] {}", result),
                    *style,
                )));
            } else {
                lines.push(Line::from(Span::styled(
                    format!("[Tool - Edit] {}", result),
                    *style,
                )));
            }
        }

        // Show old → new diff
        lines.push(Line::from(vec![
            Span::styled("      - ", Style::default().fg(Color::DarkGray)),
            Span::styled(old_text, Style::default().fg(Color::Red)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("      + ", Style::default().fg(Color::DarkGray)),
            Span::styled(new_text, Style::default().fg(Color::Green)),
        ]));
    }

    /// Write: show file path and line count only
    fn add_write_lines(
        lines: &mut Vec<Line<'static>>,
        msg: &ChatMessage,
        style: &Style,
    ) {
        // Extract file_path from arguments
        let file_path = msg
            .tool_call_arguments
            .as_ref()
            .and_then(|args| args.get("file_path").and_then(|v| v.as_str()))
            .unwrap_or("?")
            .to_string();

        // Count lines from the written content (in arguments)
        let line_count = msg
            .tool_call_arguments
            .as_ref()
            .and_then(|args| args.get("content").and_then(|v| v.as_str()))
            .map(|c| c.lines().count())
            .unwrap_or(0);

        // Show the result message
        if let Some(result) = &msg.content {
            lines.push(Line::from(Span::styled(
                format!("[Tool - Write] {}", result),
                *style,
            )));
        }

        // Compact path + line count
        lines.push(Line::from(Span::styled(
            format!("      📄 {} ({} lines)", file_path, line_count),
            Style::default().fg(Color::DarkGray),
        )));
    }

    /// Default: show full content as-is
    fn add_content_lines(
        lines: &mut Vec<Line<'static>>,
        msg: &ChatMessage,
        style: &Style,
    ) {
        if let Some(content) = &msg.content {
            lines.push(Line::from(Span::styled(
                format!("[Tool] {}", content),
                *style,
            )));
        }
    }

    // ── Visual line counting ──────────────────────────────────────

    /// Estimate the number of visual lines this message occupies
    /// at the given content width (for scroll computation).
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
            Message::AgentMessages(chat, expanded) => {
                if chat.role == Role::Tool {
                    if let Some(fn_name) = &chat.function_name {
                        match fn_name.as_str() {
                            "Read" => self.visual_read_count(chat, *expanded, width),
                            "Bash" => self.visual_bash_count(chat, *expanded, width),
                            "Edit" => self.visual_edit_count(chat, width),
                            "Write" => self.visual_write_count(chat, width),
                            _ => self.visual_default_count(chat, width),
                        }
                    } else {
                        self.visual_default_count(chat, width)
                    }
                } else {
                    self.visual_default_count(chat, width)
                }
            }
            Message::AgentStatus(_) => 1,
        }
    }

    fn visual_read_count(&self, chat: &ChatMessage, expanded: bool, _width: usize) -> usize {
        let content = chat.content.as_deref().unwrap_or("");
        let all_lines: Vec<&str> = content.lines().collect();
        let total = all_lines.len();

        let display_count = if expanded || total <= MAX_READ_LINES {
            total
        } else {
            MAX_READ_LINES + 1 // +1 for the "... N more lines" hint
        };

        display_count.max(1)
    }

    fn visual_bash_count(&self, chat: &ChatMessage, expanded: bool, _width: usize) -> usize {
        let content = chat.content.as_deref().unwrap_or("");
        let all_lines: Vec<&str> = content.lines().collect();
        let total = all_lines.len();

        if expanded || total <= MAX_BASH_LINES {
            total.max(1)
        } else {
            1 + MAX_BASH_LINES // 1 for the "... N earlier lines" header + last 5
        }
    }

    fn visual_edit_count(&self, chat: &ChatMessage, width: usize) -> usize {
        // result line + 2 diff lines
        let mut count = 3usize;
        if let Some(content) = &chat.content {
            let w = UnicodeWidthStr::width(content.as_str());
            count += 1.max((w + width - 1) / width) - 1; // the first line already counted
        }
        count
    }

    fn visual_write_count(&self, _chat: &ChatMessage, _width: usize) -> usize {
        // result line + file path line
        2
    }

    fn visual_default_count(&self, chat: &ChatMessage, width: usize) -> usize {
        let mut count = 0usize;
        if let Some(r) = &chat.reasoning_content {
            let line = format!("[{:#?} - thinking] {}", chat.role, r);
            let w = UnicodeWidthStr::width(line.as_str());
            count += 1.max((w + width - 1) / width);
        }
        if let Some(c) = &chat.content {
            let line = format!("[{:#?} - content] {}", chat.role, c);
            let w = UnicodeWidthStr::width(line.as_str());
            count += 1.max((w + width - 1) / width);
        }
        if let Some(tools) = &chat.tool_calls {
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
}

#[derive(Debug)]
pub enum Status {
    Pause,
    Running,
}
