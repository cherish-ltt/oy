use oy_agent::oy_ai::{ChatMessage, Role};
use pulldown_cmark::{Alignment, Event, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
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
    pub arguments: Option<String>,
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
    ToolCallMsg(ToolCallState),
    AgentStatus(Status),
    PromptQueued {
        id: uuid::Uuid,
        text: String,
    }, // 排队中的 prompt
}

/// Accumulates table cell data during markdown parsing.
/// Used by `render_markdown` to buffer table content and then render it as a grid.
struct TableAccum {
    alignments: Vec<Alignment>,
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    current_row: Vec<String>,
    current_cell: String,
    in_head: bool,
}

impl Message {
    /// Build the styled lines for display.
    /// For Tool messages with a known function_name, content is truncated per tool type
    /// unless `expanded` is true.
    /// `queue_number` is the 1-based display number for PromptQueued messages (None for others).
    pub fn to_lines(&self, theme: &Theme, queue_number: Option<u8>) -> Vec<Line<'_>> {
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
                            chat_message.role,
                            reasoning_content.trim_end()
                        ),
                        role_style.add_modifier(Modifier::ITALIC),
                    )));
                }

                // ----- Tool result: per-tool-type formatting -----
                if chat_message.role == Role::Tool {
                    if let Some(fn_name) = &chat_message.function_name {
                        match fn_name.as_str() {
                            "Read" => {
                                Self::add_read_lines(
                                    &mut lines,
                                    chat_message,
                                    *expanded,
                                    &role_style,
                                    theme,
                                );
                            }
                            "Bash" => {
                                Self::add_bash_lines(
                                    &mut lines,
                                    chat_message,
                                    *expanded,
                                    &role_style,
                                    theme,
                                );
                            }
                            "Edit" => {
                                Self::add_edit_lines(
                                    &mut lines,
                                    chat_message,
                                    *expanded,
                                    &role_style,
                                    theme,
                                );
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
                        let content = content.trim_end();
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
            Message::ToolCallMsg(state) => {
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
                        format!("ToolCall {} ", icon),
                        Style::default().fg(theme.accent),
                    ),
                    Span::styled(
                        &state.function_name,
                        Style::default()
                            .fg(theme.warning)
                            .add_modifier(Modifier::BOLD),
                    ),
                    if let Some(arguments) = &state.arguments {
                        Span::styled(
                            format!(": {}", arguments),
                            Style::default()
                                .fg(theme.subtle)
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        Span::styled(
                            ": unknown arguments",
                            Style::default()
                                .fg(theme.subtle)
                                .add_modifier(Modifier::BOLD),
                        )
                    },
                    Span::styled(
                        format!(" ({:.1}s)", duration),
                        Style::default().fg(theme.subtle),
                    ),
                ]));

                if let Some(result) = &state.result
                    && let Some(content) = &result.content
                {
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
                                    format!(
                                        "  ... ({} more lines, ctrl+o to expand) ",
                                        total - MAX_READ_LINES
                                    ),
                                    Style::default()
                                        .fg(theme.subtle)
                                        .add_modifier(Modifier::ITALIC),
                                )));
                            }
                        }
                        "Bash" if !state.expanded && total > MAX_BASH_LINES => {
                            let hidden = total - MAX_BASH_LINES;
                            lines.push(Line::from(Span::styled(
                                format!("  ... ({} earlier lines, ctrl+o to expand) ", hidden),
                                Style::default()
                                    .fg(theme.subtle)
                                    .add_modifier(Modifier::ITALIC),
                            )));
                            for line in &all_lines[total - MAX_BASH_LINES..] {
                                lines.push(Line::from(Span::styled(
                                    format!("  {}", line),
                                    Style::default().fg(theme.tool_fg),
                                )));
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
            Message::PromptQueued { id: _id, text } => {
                let number = queue_number.unwrap_or(0);
                let number_style = Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD);
                let label_style = Style::default().fg(theme.warning);

                // Truncate prompt text: if longer than 80 chars, cut with "..."
                let display_text = if text.len() > 80 {
                    let cut = text.char_indices()
                        .take(80)
                        .last()
                        .map(|(i, c)| i + c.len_utf8())
                        .unwrap_or(80);
                    format!("{}...", &text[..cut])
                } else {
                    text.clone()
                };

                let mut lines = Vec::new();
                // Line 1: [N] ⏳ Prompt queuing...
                lines.push(Line::from(vec![
                    Span::styled(format!("[{}] ", number), number_style),
                    Span::styled("⏳ Prompt queuing...", label_style),
                ]));
                // Line 2: indented italic prompt content
                lines.push(Line::from(Span::styled(
                    format!("    {}", display_text),
                    Style::default()
                        .fg(theme.subtle)
                        .add_modifier(Modifier::ITALIC),
                )));
                lines
            }
        }
    }

    /// ── Tool-type-specific formatting helpers ──────────────────────
    ///
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
                Style::default()
                    .fg(theme.subtle)
                    .add_modifier(Modifier::ITALIC),
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
                Style::default()
                    .fg(theme.subtle)
                    .add_modifier(Modifier::ITALIC),
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

        let old_display_count = if expanded {
            old_total
        } else {
            old_total.min(MAX_EDIT_LINES)
        };
        for line in &old_lines[..old_display_count] {
            lines.push(Line::from(vec![
                Span::styled("      - ", Style::default().fg(theme.subtle)),
                Span::styled(line.to_string(), Style::default().fg(theme.error)),
            ]));
        }
        if !expanded && old_total > MAX_EDIT_LINES {
            lines.push(Line::from(Span::styled(
                format!(
                    "      ... ({} more lines, ctrl+o to expand) ",
                    old_total - MAX_EDIT_LINES
                ),
                Style::default()
                    .fg(theme.subtle)
                    .add_modifier(Modifier::ITALIC),
            )));
        }

        let new_display_count = if expanded {
            new_total
        } else {
            new_total.min(MAX_EDIT_LINES)
        };
        for line in &new_lines[..new_display_count] {
            lines.push(Line::from(vec![
                Span::styled("      + ", Style::default().fg(theme.subtle)),
                Span::styled(line.to_string(), Style::default().fg(theme.success)),
            ]));
        }
        if !expanded && new_total > MAX_EDIT_LINES {
            lines.push(Line::from(Span::styled(
                format!(
                    "      ... ({} more lines, ctrl+o to expand) ",
                    new_total - MAX_EDIT_LINES
                ),
                Style::default()
                    .fg(theme.subtle)
                    .add_modifier(Modifier::ITALIC),
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
    fn add_content_lines(lines: &mut Vec<Line<'static>>, msg: &ChatMessage, style: &Style) {
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
            Message::ToolCallMsg(_) => theme.tool_bg,
            Message::PromptQueued { .. } => theme.surface_bg,
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
                1.max(w.div_ceil(width))
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
            Message::ToolCallMsg(state) => {
                let mut count = 1; // header line
                if let Some(result) = &state.result
                    && let Some(content) = &result.content
                {
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
                count.max(1)
            }
            Message::AgentStatus(_) => 1,
            Message::PromptQueued { .. } => 2,
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

        // Table accumulation state
        let mut table_accum: Option<TableAccum> = None;

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

        for event in Parser::new_ext(text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH) {
            // ── If inside a table, accumulate cell content only ──
            if let Some(ref mut accum) = table_accum {
                match event {
                    Event::Start(Tag::TableHead) => {
                        accum.in_head = true;
                    }
                    Event::Start(Tag::TableRow) => {
                        accum.current_row.clear();
                    }
                    Event::Start(Tag::TableCell) => {
                        accum.current_cell.clear();
                    }
                    Event::End(TagEnd::TableCell) => {
                        accum
                            .current_row
                            .push(std::mem::take(&mut accum.current_cell));
                    }
                    Event::End(TagEnd::TableRow) => {
                        let row = std::mem::take(&mut accum.current_row);
                        if accum.in_head {
                            accum.headers = row;
                            accum.in_head = false;
                        } else {
                            accum.rows.push(row);
                        }
                    }
                    Event::End(TagEnd::Table) => {
                        let rendered = render_table(accum, theme);
                        lines.extend(rendered);
                        table_accum = None;
                    }
                    Event::Text(t) => {
                        accum.current_cell.push_str(&t);
                    }
                    Event::Code(t) => {
                        accum.current_cell.push_str(&t);
                    }
                    Event::SoftBreak | Event::HardBreak => {
                        accum.current_cell.push(' ');
                    }
                    _ => {}
                }
                continue;
            }

            // ── Normal (non-table) event handling ──
            match event {
                Event::Start(tag) => match tag {
                    Tag::Table(alignments) => {
                        flush(&mut lines, &mut current);
                        table_accum = Some(TableAccum {
                            alignments,
                            headers: Vec::new(),
                            rows: Vec::new(),
                            current_row: Vec::new(),
                            current_cell: String::new(),
                            in_head: false,
                        });
                    }
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
                            base_style.fg(Color::Cyan).add_modifier(Modifier::BOLD),
                        ));
                        style_stack.push(Style::default().add_modifier(Modifier::BOLD));
                    }
                    Tag::List(_) => {}
                    Tag::Item => {
                        flush(&mut lines, &mut current);
                        current.push(Span::styled("  \u{2022} ", base_style.fg(Color::DarkGray)));
                    }
                    Tag::Link { .. } => {
                        style_stack.push(
                            Style::default()
                                .fg(Color::Blue)
                                .add_modifier(Modifier::UNDERLINED),
                        );
                    }
                    Tag::BlockQuote(_) => {
                        flush(&mut lines, &mut current);
                        current.push(Span::styled("> ", base_style.fg(Color::DarkGray)));
                    }
                    _ => {}
                },
                Event::End(tag) => match tag {
                    TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item | TagEnd::CodeBlock => {
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
                                base_style.fg(theme.code_fg).bg(theme.code_bg),
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
                        base_style.fg(Color::Cyan).bg(Color::Black),
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

        // Trim trailing whitespace/blank lines at the end of output
        trim_trailing_lines(&mut lines);

        lines.into_iter().map(Line::from).collect()
    }
}

/// Render a collected markdown table into styled lines with box-drawing characters.
fn render_table(accum: &TableAccum, theme: &Theme) -> Vec<Vec<Span<'static>>> {
    let mut out: Vec<Vec<Span<'static>>> = Vec::new();
    let col_count = accum.alignments.len();
    if col_count == 0 {
        return out;
    }

    let border_style = Style::default().fg(theme.subtle);
    let header_style = Style::default()
        .fg(theme.surface_fg)
        .add_modifier(Modifier::BOLD);
    let cell_style = Style::default().fg(theme.surface_fg);

    // Determine max content width per column (visual width of trimmed content)
    let mut col_widths = vec![1usize; col_count];
    for (i, h) in accum.headers.iter().enumerate() {
        if i < col_count {
            col_widths[i] = col_widths[i].max(UnicodeWidthStr::width(h.trim()));
        }
    }
    for row in &accum.rows {
        for (i, cell) in row.iter().enumerate() {
            if i < col_count {
                col_widths[i] = col_widths[i].max(UnicodeWidthStr::width(cell.trim()));
            }
        }
    }

    // Format cell content with alignment: returns a string of visual width `col_width + 2`
    let fmt_cell = |content: &str, align: Alignment, width: usize| -> String {
        let text = content.trim();
        let text_w = UnicodeWidthStr::width(text);
        let pad = width.saturating_sub(text_w);
        let padded = match align {
            Alignment::Left | Alignment::None => {
                format!("{}{}", text, " ".repeat(pad))
            }
            Alignment::Right => {
                format!("{}{}", " ".repeat(pad), text)
            }
            Alignment::Center => {
                let l = pad / 2;
                let r = pad - l;
                format!("{}{}{}", " ".repeat(l), text, " ".repeat(r))
            }
        };
        format!(" {} ", padded)
    };

    // Top border:  ┌───┬───┐
    {
        let top = format!(
            "┌{}┐",
            col_widths
                .iter()
                .map(|w| "─".repeat(w + 2))
                .collect::<Vec<_>>()
                .join("┬")
        );
        out.push(vec![Span::styled(top, border_style)]);
    }

    // Header row
    if !accum.headers.is_empty() {
        let mut spans = vec![Span::styled("│", border_style)];
        for (i, h) in accum.headers.iter().enumerate() {
            if i < col_count {
                let align = accum.alignments[i];
                let cell = fmt_cell(h, align, col_widths[i]);
                spans.push(Span::styled(cell, header_style));
                spans.push(Span::styled("│", border_style));
            }
        }
        out.push(spans);
    }

    // Header-body separator:  ├───┼───┤
    if !accum.headers.is_empty() {
        let sep = format!(
            "├{}┤",
            col_widths
                .iter()
                .map(|w| "─".repeat(w + 2))
                .collect::<Vec<_>>()
                .join("┼")
        );
        out.push(vec![Span::styled(sep, border_style)]);
    }

    // Body rows
    for row in &accum.rows {
        let mut spans = vec![Span::styled("│", border_style)];
        for (i, _) in col_widths.iter().enumerate().take(col_count) {
            let align = accum.alignments[i];
            let text = row.get(i).map(|s| s.as_str()).unwrap_or("");
            let cell = fmt_cell(text, align, col_widths[i]);
            spans.push(Span::styled(cell, cell_style));
            spans.push(Span::styled("│", border_style));
        }
        out.push(spans);
    }

    // Bottom border:  └───┴───┘
    {
        let bottom = format!(
            "└{}┘",
            col_widths
                .iter()
                .map(|w| "─".repeat(w + 2))
                .collect::<Vec<_>>()
                .join("┴")
        );
        out.push(vec![Span::styled(bottom, border_style)]);
    }

    out
}

/// Remove trailing empty/whitespace-only lines and trailing whitespace from the last line.
fn trim_trailing_lines(lines: &mut Vec<Vec<Span<'static>>>) {
    // Remove trailing lines that are empty or contain only whitespace spans
    while let Some(last) = lines.last() {
        let all_blank = last.iter().all(|s| s.content.trim().is_empty());
        if all_blank {
            lines.pop();
        } else {
            break;
        }
    }

    // Trim trailing whitespace from the last non-blank span
    if let Some(last_line) = lines.last_mut() {
        // Remove trailing spans that are pure whitespace
        while let Some(last_span) = last_line.last() {
            if last_span.content.trim().is_empty() {
                last_line.pop();
            } else {
                break;
            }
        }
        // Trim trailing whitespace from the last span's content
        if let Some(last_span) = last_line.last_mut() {
            let trimmed = last_span.content.trim_end().to_string();
            last_span.content = std::borrow::Cow::Owned(trimmed);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Status {
    Pause,
    Running,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::LIGHT_THEME;

    #[test]
    fn test_message_bg_ui_messages() {
        let msg = Message::UiMessages("test".into());
        assert_eq!(msg.message_bg(&LIGHT_THEME), LIGHT_THEME.surface_bg);
    }

    #[test]
    fn test_message_bg_agent_status() {
        let msg = Message::AgentStatus(Status::Pause);
        assert_eq!(msg.message_bg(&LIGHT_THEME), LIGHT_THEME.surface_bg);
    }

    #[test]
    fn test_message_bg_user() {
        let chat = ChatMessage::user("hello");
        let msg = Message::AgentMessages(chat, false);
        assert_eq!(msg.message_bg(&LIGHT_THEME), LIGHT_THEME.user_bg);
    }

    #[test]
    fn test_message_bg_assistant() {
        let chat = ChatMessage::assistant(Some("hi".into()), None, None);
        let msg = Message::AgentMessages(chat, false);
        assert_eq!(msg.message_bg(&LIGHT_THEME), LIGHT_THEME.assistant_bg);
    }

    #[test]
    fn test_message_bg_tool() {
        let chat = ChatMessage::tool("result", "id".into(), Some("Read".into()), None);
        let msg = Message::AgentMessages(chat, false);
        assert_eq!(msg.message_bg(&LIGHT_THEME), LIGHT_THEME.tool_bg);
    }

    #[test]
    fn test_message_bg_system() {
        let chat = ChatMessage::system("prompt");
        let msg = Message::AgentMessages(chat, false);
        assert_eq!(msg.message_bg(&LIGHT_THEME), LIGHT_THEME.surface_bg);
    }

    #[test]
    fn test_message_bg_tool_call_message() {
        let state = ToolCallState {
            function_name: "Read".into(),
            arguments: None,
            tool_call_id: "id".into(),
            result: None,
            start_time: Instant::now(),
            end_time: None,
            expanded: false,
        };
        let msg = Message::ToolCallMsg(state);
        assert_eq!(msg.message_bg(&LIGHT_THEME), LIGHT_THEME.tool_bg);
    }

    #[test]
    fn test_visual_line_count_ui_message() {
        let msg = Message::UiMessages("hello".into());
        assert_eq!(msg.visual_line_count(80, &LIGHT_THEME), 1);
    }

    #[test]
    fn test_visual_line_count_agent_status() {
        let msg = Message::AgentStatus(Status::Running);
        assert_eq!(msg.visual_line_count(80, &LIGHT_THEME), 1);
    }

    #[test]
    fn test_visual_line_count_user_message() {
        let chat = ChatMessage::user("hello");
        let msg = Message::AgentMessages(chat, false);
        let count = msg.visual_line_count(80, &LIGHT_THEME);
        assert!(count >= 1);
    }

    #[test]
    fn test_tool_call_state_debug() {
        let state = ToolCallState {
            function_name: "Bash".into(),
            arguments: None,
            tool_call_id: "c1".into(),
            result: None,
            start_time: Instant::now(),
            end_time: None,
            expanded: true,
        };
        let debug = format!("{:?}", state);
        assert!(debug.contains("Bash"));
        assert!(debug.contains("c1"));
    }
}
