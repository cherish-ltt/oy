use std::env;

// ui.rs
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Paragraph, Widget, Wrap},
};

use crate::{
    app::{App, AppMode, visual_cursor_pos},
};

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // 根据输入内容行数动态调整 input 区域高度（2～7 行文本，加边框）
        let input_text_width = area.width.saturating_sub(2) as usize;
        let visual_lines = self.total_visual_lines(input_text_width.max(1));
        let input_text_height = visual_lines.clamp(2, 7);
        let input_height = input_text_height + 2; // +2 for borders

        let chunks = Layout::vertical([
            Constraint::Min(5),               // Message area - flexible
            Constraint::Length(input_height), // Input area (dynamic)
            Constraint::Length(3),            // Status line
        ])
        .split(area);

        // --- Message Area (scrollable) ---
        // Convert messages to a Text object for proper wrapping
        let mut text = Text::default();
        for msg in &self.messages {
            let lines = msg.to_lines();
            text.extend(lines);
        }

        // Create paragraph with wrapping
        let message_paragraph = Paragraph::new(text)
            .block(
                Block::bordered()
                    .title("Chat History")
                    .title_alignment(Alignment::Center)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .scroll((self.scroll_offset.get(), 0))
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::Black).bg(Color::White));

        // Render message area
        message_paragraph.render(chunks[0], buf);

        // 根据实际内容高度和换行后的视觉行数计算并限制滚动偏移量
        let content_width = chunks[0].width.saturating_sub(2) as usize;
        let total_visual_lines: usize = self
            .messages
            .iter()
            .map(|msg| msg.visual_line_count(content_width))
            .sum();
        let visible_height = chunks[0].height.saturating_sub(2) as usize;
        if total_visual_lines > visible_height {
            let max_offset = (total_visual_lines - visible_height) as u16;
            let current = self.scroll_offset.get();
            if current > max_offset {
                self.scroll_offset.set(max_offset);
            }
            // 如果当前已经滚动到底部，重新启用自动滚动
            if self.scroll_offset.get() >= max_offset {
                self.auto_scroll.set(true);
            }
        } else {
            if self.scroll_offset.get() > 0 {
                self.scroll_offset.set(0);
            }
            // 内容不足一屏时，始终自动滚动到底部
            self.auto_scroll.set(true);
        }

        // --- Input Area (with dynamic title) ---
        let input_display = self.input.to_string();
        let input_text_width = chunks[1].width.saturating_sub(2) as usize;

        // Calculate cursor visual position for input
        let (cursor_visual_row, cursor_visual_col) =
            visual_cursor_pos(&self.input, self.cursor_pos, input_text_width);

        // Calculate scroll to keep cursor in view
        let input_visible_height = chunks[1].height.saturating_sub(2);
        let input_scroll = if cursor_visual_row >= input_visible_height {
            cursor_visual_row - input_visible_height + 1
        } else {
            0
        };

        // Store cursor screen position and input width for key handling
        self.cursor_x.set(chunks[1].x + 1 + cursor_visual_col);
        self.cursor_y
            .set(chunks[1].y + 1 + cursor_visual_row - input_scroll);
        self.input_width.set(chunks[1].width.saturating_sub(2));

        let input_title = if matches!(self.app_mode, AppMode::ModelForm { .. }) && !self.input_title.is_empty() {
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
                    .border_style(Style::default().fg(Color::Blue)),
            )
            .wrap(Wrap { trim: false })
            .scroll((input_scroll, 0))
            .style(Style::default().fg(Color::Black).bg(Color::White));

        input_paragraph.render(chunks[1], buf);

        // --- Status Area (information) ---
        let mut status_text = format!(
            " <Current Agent> (Cycle with shift+tab)\n Messages: {} | ↑/↓/←/→ move cursor | Enter send | Ctrl+O expand | Ctrl+C/Esc/q quit",
            self.messages.len()
        );

        if let Some(main_agent) = &self.main_agent {
            status_text =
                status_text.replace("<Current Agent>", &format!("<🖥 {}>", &main_agent.name));
        }
        let status_paragraph = Paragraph::new(status_text)
            .alignment(Alignment::Left)
            .style(Style::default().fg(Color::DarkGray).bg(Color::White));

        status_paragraph.render(chunks[2], buf);

        let mut status_text = "Use the /model command to set up one model 
            Unknown directory "
            .to_string();
        if let Some(config) = &self.global_toml_config {
            if let Some(model_name) = &config.model {
                status_text = status_text.replace(
                    "Use the /model command to set up one model ",
                    &format!("{} ", model_name),
                );
            }
            if let Ok(path) = env::current_dir() {
                status_text = status_text.replace(
                    "Unknown directory ",
                    &format!("{} ", &path.to_string_lossy()),
                );
            }
        }
        let status_paragraph = Paragraph::new(status_text)
            .alignment(Alignment::Right)
            .style(Style::default().fg(Color::DarkGray).bg(Color::White));

        status_paragraph.render(chunks[2], buf);

        // --- Command Selector Popup (rendered last, on top) ---
        if let AppMode::CommandSelector { selected } = &self.app_mode {
            let matches = self.command_registry.search(&self.input);
            if !matches.is_empty() {
                let sel = *selected;
                let popup_height = matches.len() as u16 + 2;
                let popup_area = Rect {
                    x: chunks[1].x + 1,
                    y: chunks[1].y + chunks[1].height,
                    width: chunks[1].width.saturating_sub(2),
                    height: popup_height.min(10),
                };

                let mut popup_text = Text::default();
                for (i, cmd) in matches.iter().enumerate() {
                    let style = if i == sel {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    popup_text.push_line(Line::from(vec![
                        Span::styled(
                            if i == sel { "▸ " } else { "  " },
                            style,
                        ),
                        Span::styled(
                            format!("{}  - {}", cmd.name, cmd.description),
                            style,
                        ),
                    ]));
                }

                let popup = Paragraph::new(popup_text)
                    .block(
                        Block::bordered()
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(Color::Cyan)),
                    )
                    .style(Style::default().bg(Color::Black));
                popup.render(popup_area, buf);
            }
        }
    }
}
