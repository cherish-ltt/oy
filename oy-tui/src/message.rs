use oy_agent::oy_ai::{ChatMessage, Role};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use std::time::Instant;
use unicode_width::UnicodeWidthStr;

use crate::theme::Theme;

/// Maximum lines to show for a read tool result (when collapsed)
const MAX_READ_LINES: usize = 5;
/// Maximum lines to show for a bash tool result (when collapsed) — shows the *last* N lines
const MAX_BASH_LINES: usize = 5;
const MAX_EDIT_LINES: usize = 5;

#[derive(Debug)]
pub struct ToolCallState {
    pub function_name: String,
    pub tool_call_id: String,
    pub result: Option<ChatMessage>,
    pub start_time: Instant,
    pub end_time: Option<Instant>,
    pub expanded: bool,
}

#[derive(Debug)]
pub enum Message {
    UiMessages(String),
    AgentMessages(ChatMessage, bool), // bool = expanded
    ToolCallMessage(ToolCallState),
    AgentStatus(Status),
}

impl Message {
    /// Build the styled lines for display.
    /// For Tool messages with a known function_name, content is truncated per tool type
    /// unless `expanded` is true.
    pub fn to_lines(&self, theme: &Theme) -> Vec<Line<'_>> {
        match self {
            Message::UiMessages(text) => {
                vec![Line::from(Span::styled(
                    format!("> {}", text),
                    Style::default().fg(theme.info_fg).bold(),
                ))]
            }
            Message::AgentMessages(chat_message, expanded) => {
                let role_style = match chat_message.role {
                    Role::User => Style::default().fg(theme.user_fg),
                    Role::Assistant => Style::default().fg(theme.assistant_fg),
                    Role::Tool => Style::default().fg(theme.tool_fg),
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
                                Self::add_read_lines(&mut lines, chat_message, *expanded, &role_style, theme);
                            }
                            "Bash" => {
                                Self::add_bash_lines(&mut lines, chat_message, *expanded, &role_style, theme);
                            }
                            "Edit" => {
                                Self::add_edit_lines(&mut lines, chat_message, *expanded, &role_style, theme);
                            }
                            "Write" => {
                                Self::add_write_lines(&mut lines, chat_message, &role_style, theme);
                            }
                            _ => {
                                Self::add_content_lines(&mut lines, chat_message, &role_style);
                            }
                        }
                    } else {
                        Self::add_content_lines(&mut lines, chat_message, &role_style);
                    }
                } else {
                    if let Some(content) = &chat_message.content {
                        let prefix = format!("[{:#?}] ", chat_message.role);
                        let md_lines = Self::render_markdown(content, role_style, theme);
                        for (i, line) in md_lines.into_iter().enumerate() {
                            if i == 0 {
                                let mut spans = vec![Span::styled(prefix.clone(), role_style)];
                                spans.extend(line.spans);
                                lines.push(Line::from(spans));
                            } else {
                                lines.push(line);
                            }
                        }
                    }
                }

                if let Some(tool_calls) = &chat_message.tool_calls {
                    for tool in tool_calls {
                        lines.push(Line::from(Span::styled(
                            format!("  🔧 调用工具: {}", tool.function_name),
                            Style::default().fg(theme.accent),
                        )));
                        lines.push(Line::from(Span::styled(
                            format!("     参数: {}", tool.arguments),
                            Style::default().fg(theme.subtle),
                        )));
                    }
                }

                lines
            }
            Message::ToolCallMessage(state) => {
                let mut lines = Vec::new();
                let duration = if let Some(end) = state.end_time {
                    end.duration_since(state.start_time).as_secs_f64()
                } else {
                    state.start_time.elapsed().as_secs_f64()
                };

                let icon = if state.result.is_some() { "✓" } else { "·" };

                lines.push(Line::from(vec![
                    Span::styled("🔧 ", Style::default().fg(theme.accent)),
                    Span::styled(
                        format!("工具调用 {} ", icon),
                        Style::default().fg(theme.accent),
                    ),
                    Span::styled(
                        &state.function_name,
                        Style::default().fg(theme.warning).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" ({:.1}s)", duration),
                        Style::default().fg(theme.subtle),
                    ),
                ]));

                if let Some(result) = &state.result {
                    if let Some(content) = &result.content {
                        let all_lines: Vec<&str> = content.lines().collect();
                        let total = all_lines.len();

                        match state.function_name.as_str() {
                            "Read" => {
                                let display = if state.expanded || total <= MAX_READ_LINES {
                                    total
                                } else {
                                    MAX_READ_LINES
                                };
                                for line in &all_lines[..display] {
                                    lines.push(Line::from(Span::styled(
                                        format!("  {}", line),
                                        Style::default().fg(theme.tool_fg),
                                    )));
                                }
                                if !state.expanded && total > MAX_READ_LINES {
                                    lines.push(Line::from(Span::styled(
                                        format!("  ... ({} more lines, ctrl+o to expand) ", total - MAX_READ_LINES),
                                        Style::default().fg(theme.subtle).add_modifier(Modifier::ITALIC),
                                    )));
                                }
                            }
                            "Bash" => {
                                if !state.expanded && total > MAX_BASH_LINES {
                                    let hidden = total - MAX_BASH_LINES;
                                    lines.push(Line::from(Span::styled(
                                        format!("  ... ({} earlier lines, ctrl+o to expand) ", hidden),
                                        Style::default().fg(theme.subtle).add_modifier(Modifier::ITALIC),
                                    )));
                                    for line in &all_lines[total - MAX_BASH_LINES..] {
                                        lines.push(Line::from(Span::styled(
                                            format!("  {}", line),
                                            Style::default().fg(theme.tool_fg),
                                        )));
                                    }
                                } else {
                                    for line in &all_lines {
                                        lines.push(Line::from(Span::styled(
                                            format!("  {}", line),
                                            Style::default().fg(theme.tool_fg),
                                        )));
                                    }
                                }
                            }
                            _ => {
                                for line in &all_lines {
                                    lines.push(Line::from(Span::styled(
                                        format!("  {}", line),
                                        Style::default().fg(theme.tool_fg),
                                    )));
                                }
                            }
                        }
                    }
                }

                lines
            }
            Message::AgentStatus(status) => match status {
                Status::Pause => vec![Line::from(Span::styled(
                    "> pause",
                    Style::default().fg(theme.subtle).bold(),
                ))],
                Status::Running => vec![Line::from(Span::styled(
                    "> running",
                    Style::default().fg(theme.success).bold(),
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
        theme: &Theme,
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
                Style::default().fg(theme.subtle).add_modifier(Modifier::ITALIC),
            )));
        }
    }

    /// Bash: show last MAX_BASH_LINES lines, with a truncation header
    fn add_bash_lines(
        lines: &mut Vec<Line<'static>>,
        msg: &ChatMessage,
        expanded: bool,
        style: &Style,
        theme: &Theme,
    ) {
        let content = msg.content.as_deref().unwrap_or("");
        let all_lines: Vec<&str> = content.lines().collect();
        let total = all_lines.len();

        if !expanded && total > MAX_BASH_LINES {
            let hidden = total - MAX_BASH_LINES;
            lines.push(Line::from(Span::styled(
                format!("... ({} earlier lines, ctrl+o to expand) ", hidden),
                Style::default().fg(theme.subtle).add_modifier(Modifier::ITALIC),
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

    /// Edit: show old text (red) → new text (green), with line truncation
    fn add_edit_lines(
        lines: &mut Vec<Line<'static>>,
        msg: &ChatMessage,
        expanded: bool,
        style: &Style,
        theme: &Theme,
    ) {
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

        if let Some(result) = &msg.content {
            lines.push(Line::from(Span::styled(
                format!("[Tool - Edit] {}", result),
                *style,
            )));
        }

        let old_lines: Vec<&str> = old_text.lines().collect();
        let new_lines: Vec<&str> = new_text.lines().collect();
        let old_total = old_lines.len();
        let new_total = new_lines.len();

        let old_display_count = if expanded { old_total } else { old_total.min(MAX_EDIT_LINES) };
        for line in &old_lines[..old_display_count] {
            lines.push(Line::from(vec![
                Span::styled("      - ", Style::default().fg(theme.subtle)),
                Span::styled(line.to_string(), Style::default().fg(theme.error)),
            ]));
        }
        if !expanded && old_total > MAX_EDIT_LINES {
            lines.push(Line::from(Span::styled(
                format!("      ... ({} more lines, ctrl+o to expand) ", old_total - MAX_EDIT_LINES),
                Style::default().fg(theme.subtle).add_modifier(Modifier::ITALIC),
            )));
        }

        let new_display_count = if expanded { new_total } else { new_total.min(MAX_EDIT_LINES) };
        for line in &new_lines[..new_display_count] {
            lines.push(Line::from(vec![
                Span::styled("      + ", Style::default().fg(theme.subtle)),
                Span::styled(line.to_string(), Style::default().fg(theme.success)),
            ]));
        }
        if !expanded && new_total > MAX_EDIT_LINES {
            lines.push(Line::from(Span::styled(
                format!("      ... ({} more lines, ctrl+o to expand) ", new_total - MAX_EDIT_LINES),
                Style::default().fg(theme.subtle).add_modifier(Modifier::ITALIC),
            )));
        }
    }

    /// Write: show file path and line count only
    fn add_write_lines(
        lines: &mut Vec<Line<'static>>,
        msg: &ChatMessage,
        style: &Style,
        theme: &Theme,
    ) {
        let file_path = msg
            .tool_call_arguments
            .as_ref()
            .and_then(|args| args.get("file_path").and_then(|v| v.as_str()))
            .unwrap_or("?")
            .to_string();

        let line_count = msg
            .tool_call_arguments
            .as_ref()
            .and_then(|args| args.get("content").and_then(|v| v.as_str()))
            .map(|c| c.lines().count())
            .unwrap_or(0);

        if let Some(result) = &msg.content {
            lines.push(Line::from(Span::styled(
                format!("[Tool - Write] {}", result),
                *style,
            )));
        }

        lines.push(Line::from(Span::styled(
            format!("      📄 {} ({} lines)", file_path, line_count),
            Style::default().fg(theme.subtle),
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
    /// Background color for this message type (fills full line width).
    pub fn message_bg(&self, theme: &Theme) -> Color {
        match self {
            Message::UiMessages(_) | Message::AgentStatus(_) => theme.surface_bg,
            Message::AgentMessages(chat, _) => match chat.role {
                Role::User => theme.user_bg,
                Role::Assistant => theme.assistant_bg,
                Role::Tool => theme.tool_bg,
                Role::System => theme.surface_bg,
            },
            Message::ToolCallMessage(_) => theme.tool_bg,
        }
    }

    pub fn visual_line_count(&self, width: usize, _theme: &Theme) -> usize {
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
                            "Edit" => self.visual_edit_count(chat, *expanded, width),
                            "Write" => self.visual_write_count(chat, width),
                            _ => self.visual_default_count(chat, width, _theme),
                        }
                    } else {
                        self.visual_default_count(chat, width, _theme)
                    }
                } else {
                    self.visual_default_count(chat, width, _theme)
                }
            }
            Message::ToolCallMessage(state) => {
                let mut count = 1; // header line
                if let Some(result) = &state.result {
                    if let Some(content) = &result.content {
                        let all_lines: Vec<&str> = content.lines().collect();
                        let total = all_lines.len();
                        match state.function_name.as_str() {
                            "Read" => {
                                if state.expanded || total <= MAX_READ_LINES {
                                    count += total;
                                } else {
                                    count += MAX_READ_LINES + 1;
                                }
                            }
                            "Bash" => {
                                if state.expanded || total <= MAX_BASH_LINES {
                                    count += total;
                                } else {
                                    count += 1 + MAX_BASH_LINES;
                                }
                            }
                            _ => {
                                count += total;
                            }
                        }
                    }
                }
                count.max(1)
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

    fn visual_edit_count(&self, chat: &ChatMessage, expanded: bool, _width: usize) -> usize {
        let mut count = 0usize;

        // result line
        if chat.content.is_some() {
            count += 1;
        }

        // old text lines (with truncation)
        if let Some(args) = &chat.tool_call_arguments {
            if let Some(old) = args.get("old_text").and_then(|v| v.as_str()) {
                let old_lines: Vec<&str> = old.lines().collect();
                let old_total = old_lines.len();
                if expanded || old_total <= MAX_EDIT_LINES {
                    count += old_total;
                } else {
                    count += MAX_EDIT_LINES + 1; // +1 for the hint
                }
            }
            // new text lines (with truncation)
            if let Some(new) = args.get("new_text").and_then(|v| v.as_str()) {
                let new_lines: Vec<&str> = new.lines().collect();
                let new_total = new_lines.len();
                if expanded || new_total <= MAX_EDIT_LINES {
                    count += new_total;
                } else {
                    count += MAX_EDIT_LINES + 1; // +1 for the hint
                }
            }
        }

        count.max(1)
    }

    fn visual_write_count(&self, _chat: &ChatMessage, _width: usize) -> usize {
        // result line + file path line
        2
    }

    fn visual_default_count(&self, chat: &ChatMessage, _width: usize, theme: &Theme) -> usize {
        let mut count = 0usize;
        if let Some(r) = &chat.reasoning_content {
            count += Self::render_markdown(r, Style::default(), theme).len();
        }
        if let Some(c) = &chat.content {
            count += Self::render_markdown(c, Style::default(), theme).len();
        }
        if let Some(tools) = &chat.tool_calls {
            for _tool in tools {
                count += 2;
            }
        }
        count
    }

    /// Render Markdown text into styled ratatui lines.
    fn render_markdown(text: &str, base_style: Style, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines: Vec<Vec<Span<'static>>> = Vec::new();
        let mut current: Vec<Span<'static>> = Vec::new();
        let mut style_stack: Vec<Style> = Vec::new();
        let mut in_code_block = false;

        fn current_style(base: Style, stack: &[Style]) -> Style {
            let mut s = base;
            for st in stack {
                s = s.patch(*st);
            }
            s
        }

        fn flush(out: &mut Vec<Vec<Span<'static>>>, cur: &mut Vec<Span<'static>>) {
            if !cur.is_empty() {
                out.push(std::mem::take(cur));
            }
        }

        for event in Parser::new(text) {
            match event {
                Event::Start(tag) => match tag {
                    Tag::Paragraph => {}
                    Tag::Strong => {
                        style_stack.push(Style::default().add_modifier(Modifier::BOLD));
                    }
                    Tag::Emphasis => {
                        style_stack.push(Style::default().add_modifier(Modifier::ITALIC));
                    }
                    Tag::Strikethrough => {
                        style_stack.push(Style::default().add_modifier(Modifier::CROSSED_OUT));
                    }
                    Tag::CodeBlock(_) => {
                        flush(&mut lines, &mut current);
                        in_code_block = true;
                    }
                    Tag::Heading { level, .. } => {
                        flush(&mut lines, &mut current);
                        let prefix = "#".repeat(level as usize);
                        current.push(Span::styled(
                            format!("{} ", prefix),
                            base_style
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ));
                        style_stack.push(Style::default().add_modifier(Modifier::BOLD));
                    }
                    Tag::List(_) => {}
                    Tag::Item => {
                        flush(&mut lines, &mut current);
                        current.push(Span::styled(
                            "  \u{2022} ",
                            base_style.fg(Color::DarkGray),
                        ));
                    }
                    Tag::Link { .. } => {
                        style_stack
                            .push(Style::default().fg(Color::Blue).add_modifier(Modifier::UNDERLINED));
                    }
                    Tag::BlockQuote(_) => {
                        flush(&mut lines, &mut current);
                        current.push(Span::styled(
                            "> ",
                            base_style.fg(Color::DarkGray),
                        ));
                    }
                    _ => {}
                },
                Event::End(tag) => match tag {
                    TagEnd::Paragraph
                    | TagEnd::Heading(_)
                    | TagEnd::Item
                    | TagEnd::CodeBlock => {
                        flush(&mut lines, &mut current);
                        if matches!(tag, TagEnd::CodeBlock) {
                            in_code_block = false;
                        }
                    }
                    TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough => {
                        style_stack.pop();
                    }
                    TagEnd::Link => {
                        style_stack.pop();
                    }
                    _ => {}
                },
                Event::Text(text) => {
                    if in_code_block {
                        for (i, line) in text.lines().enumerate() {
                            if i > 0 {
                                flush(&mut lines, &mut current);
                            }
                            current.push(Span::styled(
                                line.to_string(),
                                base_style
                                    .fg(Color::Cyan)
                                    .bg(Color::Black),
                            ));
                        }
                    } else {
                        let style = current_style(base_style, &style_stack);
                        current.push(Span::styled(text.to_string(), style));
                    }
                }
                Event::Code(text) => {
                    current.push(Span::styled(
                        format!("`{}`", text),
                        base_style
                            .fg(Color::Cyan)
                            .bg(Color::Black),
                    ));
                }
                Event::SoftBreak | Event::HardBreak => {
                    flush(&mut lines, &mut current);
                }
                Event::Html(html) => {
                    current.push(Span::raw(html.to_string()));
                }
                Event::Rule => {
                    flush(&mut lines, &mut current);
                    current.push(Span::styled(
                        "\u{2500}".repeat(50),
                        base_style.fg(theme.subtle),
                    ));
                    flush(&mut lines, &mut current);
                }
                _ => {}
            }
        }

        flush(&mut lines, &mut current);

        lines.into_iter().map(Line::from).collect()
    }
}

#[derive(Debug)]
pub enum Status {
    Pause,
    Running,
}
