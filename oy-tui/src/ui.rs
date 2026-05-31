use std::env;

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Paragraph, Widget, Wrap},
};

use crate::{
    app::{App, AppMode, visual_cursor_pos},
    command::CommandInfo,
};

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.theme;

        let input_text_width = area.width.saturating_sub(2) as usize;
        let visual_lines = self.total_visual_lines(input_text_width.max(1));
        let input_text_height = visual_lines.clamp(2, 7);
        let input_height = input_text_height + 2;

        // Reserve space for popups when in selector/menu mode
        let has_popup = matches!(self.app_mode, AppMode::CommandSelector{..} | AppMode::SubMenu{..});
        let popup_rows: u16 = if has_popup { 7 } else { 0 };

        let chunks = Layout::vertical([
            Constraint::Min(5),
            Constraint::Length(input_height),
            Constraint::Length(popup_rows),
            Constraint::Length(3),
        ])
        .split(area);

        // ── Message Area (per-message paragraphs with full-line backgrounds) ──
        let content_width = chunks[0].width.saturating_sub(2) as usize;
        let visible_height = (chunks[0].height.saturating_sub(2)) as usize;

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
        // (spacers are counted as 1 line each in the scroll space)
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
        let inner_x = chunks[0].x + 1;
        let inner_w = content_width as u16;
        let mut used_lines = 0usize;

        for i in msg_idx..self.messages.len() {
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
                y: chunks[0].y + 1 + used_lines as u16,
                width: inner_w,
                height: render_lines as u16,
            };

            let lines = self.messages[i].to_lines(t);
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
                let spacer_y = chunks[0].y + 1 + used_lines as u16;
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
        border_block.render(chunks[0], buf);

        // ── Input Area ──
        let input_display = self.input.to_string();
        let input_text_width = chunks[1].width.saturating_sub(2) as usize;

        let (cursor_visual_row, cursor_visual_col) =
            visual_cursor_pos(&self.input, self.cursor_pos, input_text_width);

        let input_visible_height = chunks[1].height.saturating_sub(2);
        let input_scroll = if cursor_visual_row >= input_visible_height {
            cursor_visual_row - input_visible_height + 1
        } else {
            0
        };

        self.cursor_x.set(chunks[1].x + 1 + cursor_visual_col);
        self.cursor_y
            .set(chunks[1].y + 1 + cursor_visual_row - input_scroll);
        self.input_width.set(chunks[1].width.saturating_sub(2));

        let input_title = if matches!(self.app_mode, AppMode::ModelForm { .. }) && !self.input_title.is_empty()
        {
            self.input_title.clone()
        } else {
            "Input".to_string()
        };

        let input_paragraph = Paragraph::new(input_display)
            .block(
                Block::bordered()
                    .title(input_title)
                    .title_alignment(Alignment::Left)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(t.input_border)),
            )
            .wrap(Wrap { trim: false })
            .scroll((input_scroll, 0))
            .style(Style::default().fg(t.surface_fg).bg(t.surface_bg));

        input_paragraph.render(chunks[1], buf);

        // ── Status Area ──
        let mut status_text = format!(
            " <Current Agent> (Cycle with shift+tab)\n Messages: {} | ↑/↓/←/→ move cursor | Enter send | Ctrl+O expand | Ctrl+C/Esc/q quit",
            self.messages.len()
        );

        if let Some(main_agent) = &self.main_agent {
            status_text =
                status_text.replace("<Current Agent>", &format!("<{}>", &main_agent.name));
        }
        let status_paragraph = Paragraph::new(status_text)
            .alignment(Alignment::Left)
            .style(Style::default().fg(t.status_fg).bg(t.status_bg));

        status_paragraph.render(chunks[3], buf);

        let mut status_right = "Use the /model command to set up one model 
            Unknown directory "
            .to_string();
        if let Some(config) = &self.global_toml_config {
            if let Some(model_name) = &config.model {
                status_right = status_right.replace(
                    "Use the /model command to set up one model ",
                    &format!("{} ", model_name),
                );
            }
            if let Ok(path) = env::current_dir() {
                status_right = status_right.replace(
                    "Unknown directory ",
                    &format!("{} ", &path.to_string_lossy()),
                );
            }
        }
        let status_right_para = Paragraph::new(status_right)
            .alignment(Alignment::Right)
            .style(Style::default().fg(t.status_fg).bg(t.status_bg));

        status_right_para.render(chunks[3], buf);

        // ── SubMenu / Command Selector Popup ──
        // Render into chunks[2] (reserved popup area)
        if let AppMode::SubMenu {
            title: _,
            items,
            selected,
        } = &self.app_mode
        {
            if !items.is_empty() {
                let sel = *selected;
                let mut popup_text = Text::default();
                for (i, (name, desc)) in items.iter().enumerate() {
                    let style = if i == sel {
                        Style::default().fg(t.surface_bg).bg(t.accent)
                    } else {
                        Style::default().fg(t.surface_fg)
                    };
                    popup_text.push_line(Line::from(vec![
                        Span::styled(if i == sel { "\u{25b8} " } else { "  " }, style),
                        Span::styled(format!("{}  - {}", name, desc), style),
                    ]));
                }
                let popup = Paragraph::new(popup_text)
                    .block(
                        Block::bordered()
                            .title("Settings")
                            .title_alignment(Alignment::Left)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(t.accent)),
                    )
                    .style(Style::default().bg(t.surface_bg));
                popup.render(chunks[2], buf);
            }
        }

        // ── Command Selector Popup ──
        if let AppMode::CommandSelector {
            selected,
            scroll_offset,
        } = &self.app_mode
        {
            let matches = self.command_registry.search(&self.input);
            if !matches.is_empty() {
                let sel = *selected;
                let scroll = *scroll_offset;
                let total = matches.len();
                let max_rows = 5usize;
                let visible: Vec<&&CommandInfo> = matches
                    .iter()
                    .skip(scroll)
                    .take(max_rows)
                    .collect();
                let has_more_down = scroll + max_rows < total;
                let has_more_up = scroll > 0;

                let popup_area = Rect {
                    x: chunks[2].x + 1,
                    y: chunks[2].y + 1,
                    width: chunks[2].width.saturating_sub(2),
                    height: chunks[2].height.saturating_sub(2),
                };

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
                        Span::styled(
                            if abs_idx == sel { "\u{25b8} " } else { "  " },
                            style,
                        ),
                        Span::styled(
                            format!("{}  - {}", cmd.name, cmd.description),
                            style,
                        ),
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
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(t.accent)),
                    )
                    .style(Style::default().bg(t.surface_bg));
                popup.render(popup_area, buf);
            }
        }
    }
}
