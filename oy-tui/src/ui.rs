use oy_agent::format_token_count;
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Paragraph, Widget, Wrap},
};
use unicode_width::UnicodeWidthChar;

use crate::{
    app::{App, AppMode, SubAgentUiState, visual_cursor_pos},
    command::CommandInfo,
    message::{Message, Status},
    theme::Theme,
};

/// 使用与 `visual_cursor_pos` 一致的 word-wrap 算法将输入文本按宽度切割成行。
/// 这样 ratatui 的 Paragraph 就不再需要 Wrap，光标位置与实际渲染完全对齐。
fn wrap_input_text(input: &str, width: usize) -> Vec<String> {
    if input.is_empty() || width == 0 {
        return vec![input.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current_line = String::new();
    let mut col = 0u16;
    let mut pending_ws: Vec<char> = Vec::new(); // 暂存的空白字符

    for ch in input.chars() {
        if ch == '\n' {
            // 显式换行：先刷入暂存空白
            for wc in pending_ws.drain(..) {
                current_line.push(wc);
            }
            lines.push(std::mem::take(&mut current_line));
            col = 0;
            continue;
        }

        let w = UnicodeWidthChar::width(ch).unwrap_or(1) as u16;

        if ch.is_ascii_whitespace() && ch != '\n' {
            // 空白字符：暂存起来，等待后面的非空白字符决定是否换行
            if col + w > width as u16 {
                // 空白在行末：丢弃空白并换行
                lines.push(std::mem::take(&mut current_line));
                pending_ws.clear();
                col = 0;
            } else {
                col += w;
                pending_ws.push(ch);
            }
        } else {
            // 非空白字符
            if col + w > width as u16 {
                // 当前行放不下：先刷入暂存空白，再换到新行
                for wc in pending_ws.drain(..) {
                    current_line.push(wc);
                }
                lines.push(std::mem::take(&mut current_line));
                col = w;
                current_line.push(ch);
            } else {
                // 先刷入暂存空白
                for wc in pending_ws.drain(..) {
                    current_line.push(wc);
                }
                col += w;
                current_line.push(ch);
            }
        }
    }
    // 刷入末尾暂存的空白
    for wc in pending_ws.drain(..) {
        current_line.push(wc);
    }
    if !current_line.is_empty() || input.ends_with('\n') {
        lines.push(current_line);
    }
    lines
}

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.theme;

        let input_text_width = area.width.saturating_sub(2) as usize;
        let visual_lines = self.total_visual_lines(input_text_width.max(1));
        let input_text_height = visual_lines.clamp(2, 7);
        let input_height = input_text_height + 2;

        // Reserve space for popups when in selector/menu mode
        let has_popup = matches!(
            self.app_mode,
            AppMode::CommandSelector { .. } | AppMode::SubMenu { .. }
        );
        let popup_rows: u16 = if has_popup { 7 } else { 0 };

        // Calculate sub-agent panel height
        let has_sub_agents = !self.sub_agent_states.is_empty();
        let sub_agent_rows: u16 = if has_sub_agents {
            (self.sub_agent_states.len() as u16).min(5) + 2 // header + items + border
        } else {
            0
        };

        let chunks = Layout::vertical([
            Constraint::Min(5),
            Constraint::Length(input_height),
            Constraint::Length(popup_rows),
            Constraint::Length(3),              // status bar
            Constraint::Length(sub_agent_rows), // sub-agent panel
        ])
        .split(area);

        self.render_chat_area(chunks[0], buf, t);
        self.render_input_area(chunks[1], buf, t);
        self.render_status_area(chunks[3], buf, t);

        // ── Sub-Agent Status Panel ──
        if has_sub_agents && chunks[4].height >= 3 {
            self.render_sub_agent_panel(chunks[4], buf, t);
        } else {
            // No sub-agents — reset panel Y to avoid stale detection
            self.sub_agent_panel_y.set(u16::MAX);
        }

        // ── Popups ──
        if has_popup {
            self.render_popups(chunks[2], buf, t);
        }
    }
}

impl App {
    /// Render the chat history message area with scrolling support.
    #[allow(clippy::cognitive_complexity)]
    fn render_chat_area(&self, area: Rect, buf: &mut Buffer, t: &Theme) {
        let content_width = area.width.saturating_sub(2) as usize;
        let visible_height = (area.height.saturating_sub(2)) as usize;

        // Detect terminal resize: if chat area width changed, auto-scroll to bottom
        if self.last_chat_width.get() != content_width as u16 && self.last_chat_width.get() != 0 {
            self.auto_scroll.set(true);
            self.scroll_offset.set(u16::MAX);
        }
        self.last_chat_width.set(content_width as u16);

        // Compute per-message heights (content only, no trailing blanks)
        let msg_heights: Vec<usize> = self
            .messages
            .iter()
            .map(|msg| msg.visual_line_count(content_width, t))
            .collect();
        // Add 1 spacer line between messages
        let spacer_count = self.messages.len().saturating_sub(1);
        let total_visual: usize = msg_heights.iter().sum::<usize>() + spacer_count;

        // Clamp scroll offset
        if total_visual > visible_height {
            let max_offset = (total_visual - visible_height) as u16;
            if self.scroll_offset.get() > max_offset {
                self.scroll_offset.set(max_offset);
            }
            if self.scroll_offset.get() >= max_offset {
                self.auto_scroll.set(true);
            }
        } else {
            if self.scroll_offset.get() > 0 {
                self.scroll_offset.set(0);
            }
            self.auto_scroll.set(true);
        }

        // Determine starting message and line offset from scroll
        let mut scroll_rem = self.scroll_offset.get() as usize;
        let mut msg_idx = 0usize;
        let mut line_offset = 0usize;
        for (i, &h) in msg_heights.iter().enumerate() {
            let entry_height = h + if i < msg_heights.len() - 1 { 1 } else { 0 };
            if scroll_rem < entry_height {
                msg_idx = i;
                if scroll_rem < h {
                    line_offset = scroll_rem;
                } else {
                    line_offset = 0;
                }
                break;
            }
            scroll_rem -= entry_height;
        }

        // Render visible messages as individual paragraphs
        let inner_x = area.x + 1;
        let inner_w = content_width as u16;
        let mut used_lines = 0usize;
        // Track the 1-based display number for PromptQueued messages
        let mut prompt_queue_idx = 0u8;

        for (i, _) in msg_heights
            .iter()
            .enumerate()
            .take(self.messages.len())
            .skip(msg_idx)
        {
            if used_lines >= visible_height {
                break;
            }
            let h = msg_heights[i];
            let available = visible_height - used_lines;
            let remaining = h.saturating_sub(line_offset);
            let render_lines = remaining.min(available);
            if render_lines == 0 {
                break;
            }

            let msg_area = Rect {
                x: inner_x,
                y: area.y + 1 + used_lines as u16,
                width: inner_w,
                height: render_lines as u16,
            };

            // Compute the queue display number for PromptQueued messages
            let queue_number = if matches!(self.messages[i], Message::PromptQueued { .. }) {
                prompt_queue_idx += 1;
                Some(prompt_queue_idx)
            } else {
                None
            };
            let lines = self.messages[i].to_lines(t, queue_number);
            let mut text = Text::default();
            text.extend(lines);

            let bg = self.messages[i].message_bg(t);
            Paragraph::new(text)
                .scroll((line_offset as u16, 0))
                .wrap(Wrap { trim: false })
                .style(Style::default().bg(bg))
                .render(msg_area, buf);

            used_lines += render_lines;
            line_offset = 0;

            // Add spacer line (with surface_bg) between messages
            if i + 1 < self.messages.len() && used_lines < visible_height {
                let spacer_y = area.y + 1 + used_lines as u16;
                let spacer_area = Rect {
                    x: inner_x,
                    y: spacer_y,
                    width: inner_w,
                    height: 1,
                };
                // Spacer row with surface_bg (text must be non-empty to fill the row)
                let spacer_text = Text::from(Line::from(Span::raw(" ")));
                let spacer = Paragraph::new(spacer_text)
                    .style(Style::default().fg(t.surface_bg).bg(t.surface_bg));
                spacer.render(spacer_area, buf);
                used_lines += 1;
            }
        }

        // Draw the outer border on top
        let border_block = Block::bordered()
            .title("Chat History")
            .title_alignment(Alignment::Center)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(t.border));
        border_block.render(area, buf);
    }

    /// Render the text input area with wrapping and cursor positioning.
    fn render_input_area(&self, area: Rect, buf: &mut Buffer, t: &Theme) {
        let input_text = self.input.to_string();
        let input_text_width = area.width.saturating_sub(2) as usize;

        // 手动换行，算法与 visual_cursor_pos 一致
        let wrapped_lines = wrap_input_text(&input_text, input_text_width);
        let wrapped_text = wrapped_lines.join("\n");

        let (cursor_visual_row, cursor_visual_col) =
            visual_cursor_pos(&self.input, self.cursor_pos, input_text_width);

        let input_visible_height = area.height.saturating_sub(2);
        let input_scroll = if cursor_visual_row >= input_visible_height {
            cursor_visual_row - input_visible_height + 1
        } else {
            0
        };

        self.cursor_x.set(area.x + 1 + cursor_visual_col);
        self.cursor_y
            .set(area.y + 1 + cursor_visual_row - input_scroll);
        self.input_width.set(area.width.saturating_sub(2));

        let input_title = if matches!(self.app_mode, AppMode::RevokeSelect) {
            "Revoke [1-9]: select, Esc: cancel".to_string()
        } else if matches!(self.app_mode, AppMode::ModelForm { .. }) && !self.input_title.is_empty()
        {
            self.input_title.clone()
        } else {
            "Input".to_string()
        };

        let input_paragraph = Paragraph::new(wrapped_text)
            .block(
                Block::bordered()
                    .title(input_title)
                    .title_alignment(Alignment::Left)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(t.input_border)),
            )
            .scroll((input_scroll, 0))
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(t.surface_fg).bg(t.surface_bg));

        input_paragraph.render(area, buf);
    }

    /// Render the status bar with spinner, token usage, and agent info.
    fn render_status_area(&self, area: Rect, buf: &mut Buffer, t: &Theme) {
        const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let spinner_char = match self.agent_status.get() {
            Status::Running => {
                let idx = (self.tick_counter.get() as usize / 2) % SPINNER.len();
                SPINNER[idx]
            },
            Status::Pause => "•",
        };

        // Build token stats string: [↑input ↓output]
        let token_stats = {
            let input = format_token_count(self.token_usage.input_tokens);
            let output = format_token_count(self.token_usage.output_tokens);
            format!(" [↑{} ↓{}]", input, output)
        };

        // Build context usage string: current_context/max_context (percentage)
        let context_display = {
            let total_used = self.token_usage.context_tokens;
            let capacity = self
                .global_toml_config
                .as_ref()
                .and_then(|c| c.context_capacity)
                .unwrap_or(200_000);
            let used_str = format_token_count(total_used);
            let cap_str = if capacity >= 1_000_000 {
                format!("{}M", capacity / 1_000_000)
            } else {
                format_token_count(capacity)
            };
            let pct = if capacity > 0 {
                (total_used as f64 / capacity as f64 * 100.0).round() as u64
            } else {
                0
            };
            format!(" {}/{} ({}%)", used_str, cap_str, pct)
        };

        let agent_label = match self.active_agent {
            crate::app::AgentType::MainAgent => "<MainAgent>",
            crate::app::AgentType::CommanderAgent => "<CommanderAgent>",
        };

        let revoke_hint = if !self.pending_prompts.is_empty() {
            " | Ctrl+R revoke"
        } else {
            ""
        };
        let status_text = format!(
            " {}{}{} {} (Cycle with shift+tab)\n ↑/↓/←/→ move cursor | Enter send/Alt+Enter send to queue  | Ctrl+O expand | Ctrl+C/Esc/q quit{}",
            agent_label, token_stats, context_display, spinner_char, revoke_hint,
        );

        let status_paragraph = Paragraph::new(status_text)
            .alignment(Alignment::Left)
            .style(Style::default().fg(t.status_fg).bg(t.status_bg));

        status_paragraph.render(area, buf);

        let mut status_right = "Use the /model command to set up one model 
            Unknown directory "
            .to_string();
        if let Some(config) = &self.global_toml_config {
            if let Some(model_name) = &config.model {
                let effort = config.reasoning_effort.as_deref().unwrap_or("high");
                status_right = status_right.replace(
                    "Use the /model command to set up one model ",
                    &format!("{} · {} ", model_name, effort),
                );
            }
            if let Ok(path) = std::env::current_dir() {
                status_right = status_right.replace(
                    "Unknown directory ",
                    &format!("{} ", &path.to_string_lossy()),
                );
            }
        }
        let status_right_para = Paragraph::new(status_right)
            .alignment(Alignment::Right)
            .style(Style::default().fg(t.status_fg).bg(t.status_bg));

        status_right_para.render(area, buf);
    }

    /// Render the sub-agent execution status panel.
    fn render_sub_agent_panel(&self, area: Rect, buf: &mut Buffer, t: &Theme) {
        let max_items = area.height.saturating_sub(2) as usize; // content rows
        let total = self.sub_agent_states.len();
        let scroll = self.sub_agent_scroll.get().min(total.saturating_sub(1));

        // Store panel Y position for mouse scroll detection
        self.sub_agent_panel_y.set(area.y);

        let has_more_up = scroll > 0;
        let has_more_down = scroll + max_items < total;

        let indicator_lines = has_more_up as usize + has_more_down as usize;
        let visible_max = max_items.saturating_sub(indicator_lines);
        let visible: Vec<&SubAgentUiState> = self
            .sub_agent_states
            .iter()
            .skip(scroll)
            .take(visible_max)
            .collect();

        let mut panel_lines = Vec::new();
        if has_more_up {
            panel_lines.push(Line::from(Span::styled(
                "  \u{2191} more...",
                Style::default().fg(t.subtle),
            )));
        }
        for state in &visible {
            let icon = if state.completed {
                if state.success { "✓" } else { "✗" }
            } else {
                "▶"
            };
            let task_preview: String = state.task.chars().take(35).collect();
            let elapsed = state
                .elapsed_secs
                .unwrap_or_else(|| state.start_time.elapsed().as_secs_f64());
            let line_str = format!(
                " {} {}  {}  {:.1}s",
                icon, state.agent_type, task_preview, elapsed
            );
            panel_lines.push(Line::from(Span::styled(
                line_str,
                Style::default().fg(t.info_fg),
            )));
        }
        if has_more_down {
            panel_lines.push(Line::from(Span::styled(
                "  \u{2193} more...",
                Style::default().fg(t.subtle),
            )));
        }

        let panel = Paragraph::new(Text::from(panel_lines))
            .block(
                Block::bordered()
                    .title("Sub-Agents")
                    .title_alignment(Alignment::Left)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(t.accent)),
            )
            .style(Style::default().bg(t.surface_bg));
        panel.render(area, buf);
    }

    /// Render popups: SubMenu and CommandSelector.
    fn render_popups(&self, area: Rect, buf: &mut Buffer, t: &Theme) {
        // ── SubMenu / Command Selector Popup ──
        if let AppMode::SubMenu {
            title,
            items,
            selected,
            scroll_offset,
        } = &self.app_mode
            && !items.is_empty()
        {
            self.render_submenu_popup(area, buf, t, title, items, *selected, *scroll_offset);
            return;
        }

        // ── Command Selector Popup ──
        if let AppMode::CommandSelector {
            selected,
            scroll_offset,
        } = &self.app_mode
        {
            self.render_command_selector_popup(area, buf, t, *selected, *scroll_offset);
        }
    }

    /// Render the SubMenu popup.
    #[allow(clippy::too_many_arguments)]
    fn render_submenu_popup(
        &self,
        area: Rect,
        buf: &mut Buffer,
        t: &Theme,
        title: &str,
        items: &[(String, String)],
        selected: usize,
        scroll_offset: usize,
    ) {
        let sel = selected;
        let scroll = scroll_offset;
        let total = items.len();
        let max_content_rows = (area.height.saturating_sub(2)) as usize;
        let has_more_up = scroll > 0;
        let has_more_down = scroll + max_content_rows < total;

        // Account for "↑/↓ more..." indicator lines
        let indicator_lines = (has_more_up as usize) + (has_more_down as usize);
        let max_items = max_content_rows.saturating_sub(indicator_lines);
        let visible: Vec<&(String, String)> = items.iter().skip(scroll).take(max_items).collect();

        // Recalculate has_more_down based on actual items taken
        let has_more_down = scroll + visible.len() < total;

        let mut popup_text = Text::default();
        if has_more_up {
            popup_text.push_line(Line::from(Span::styled(
                "  \u{2191} more...",
                Style::default().fg(t.subtle),
            )));
        }
        for (i, (name, desc)) in visible.iter().enumerate() {
            let abs_idx = scroll + i;
            let style = if abs_idx == sel {
                Style::default().fg(t.surface_bg).bg(t.accent)
            } else {
                Style::default().fg(t.surface_fg)
            };
            popup_text.push_line(Line::from(vec![
                Span::styled(if abs_idx == sel { "\u{25b8} " } else { "  " }, style),
                Span::styled(format!("{}  - {}", name, desc), style),
            ]));
        }
        if has_more_down {
            popup_text.push_line(Line::from(Span::styled(
                "  \u{2193} more...",
                Style::default().fg(t.subtle),
            )));
        }

        let popup = Paragraph::new(popup_text)
            .block(
                Block::bordered()
                    .title(title)
                    .title_alignment(Alignment::Left)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(t.accent)),
            )
            .style(Style::default().bg(t.surface_bg));
        popup.render(area, buf);
    }

    /// Render the command selector popup.
    #[allow(clippy::too_many_arguments)]
    fn render_command_selector_popup(
        &self,
        area: Rect,
        buf: &mut Buffer,
        t: &Theme,
        selected: usize,
        scroll_offset: usize,
    ) {
        let matches = self.command_registry.search(&self.input);
        if matches.is_empty() {
            return;
        }

        let sel = selected;
        let scroll = scroll_offset;
        let total = matches.len();
        let max_content_rows = (area.height.saturating_sub(2)) as usize;
        let has_more_up = scroll > 0;
        let has_more_down = scroll + max_content_rows < total;

        // Account for "↑/↓ more..." indicator lines
        let indicator_lines = (has_more_up as usize) + (has_more_down as usize);
        let max_items = max_content_rows.saturating_sub(indicator_lines);
        let visible: Vec<&&CommandInfo> = matches.iter().skip(scroll).take(max_items).collect();

        // Recalculate has_more_down based on actual items taken
        let has_more_down = scroll + visible.len() < total;

        let mut popup_text = Text::default();
        if has_more_up {
            popup_text.push_line(Line::from(Span::styled(
                "  \u{2191} more...",
                Style::default().fg(t.subtle),
            )));
        }
        for (i, cmd) in visible.iter().enumerate() {
            let abs_idx = scroll + i;
            let style = if abs_idx == sel {
                Style::default().fg(t.surface_bg).bg(t.accent)
            } else {
                Style::default().fg(t.surface_fg)
            };
            popup_text.push_line(Line::from(vec![
                Span::styled(if abs_idx == sel { "\u{25b8} " } else { "  " }, style),
                Span::styled(format!("{}  - {}", cmd.name, cmd.description), style),
            ]));
        }
        if has_more_down {
            popup_text.push_line(Line::from(Span::styled(
                "  \u{2193} more...",
                Style::default().fg(t.subtle),
            )));
        }

        let popup = Paragraph::new(popup_text)
            .block(
                Block::bordered()
                    .title("Commands")
                    .title_alignment(Alignment::Left)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(t.accent)),
            )
            .style(Style::default().bg(t.surface_bg));
        popup.render(area, buf);
    }
}
