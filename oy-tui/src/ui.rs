// ui.rs
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Text},
    widgets::{Block, BorderType, Paragraph, Widget, Wrap},
};

use crate::app::{App, visual_cursor_pos};

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // 根据输入内容行数动态调整 input 区域高度（2～6 行文本，加边框）
        let input_text_width = area.width.saturating_sub(4) as usize;
        let visual_lines = self.total_visual_lines(input_text_width.max(1));
        let input_text_height = visual_lines.clamp(2, 7);
        let input_height = input_text_height + 2; // +2 for borders

        let chunks = Layout::vertical([
            Constraint::Min(5),        // Message area - flexible
            Constraint::Length(input_height), // Input area (dynamic)
            Constraint::Length(3),     // Status line
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
                    .border_type(BorderType::Rounded),
            )
            .scroll((self.scroll_offset.get(), 0))
            .style(Style::default().fg(Color::White).bg(Color::Black));

        // Render message area
        message_paragraph.render(chunks[0], buf);

        // 根据实际内容高度计算并限制滚动偏移量
        // 在渲染之后才能知道包裹线的数量，但我们会进行近似计算。
        // 为了实现完美的夹紧效果，我们需要测量所显示的高度。更简单的办法是：保持原状不变。
        // 用户自然会达到上限。或者，我们可以计算理论上的最大行数：
        let total_lines = self.messages.len();
        let visible_height = chunks[0].height.saturating_sub(2); // subtract borders
        if total_lines > visible_height as usize {
            let max_offset = (total_lines - visible_height as usize) as u16;
            let current = self.scroll_offset.get();
            if current > max_offset {
                self.scroll_offset.set(max_offset);
            }
        } else if self.scroll_offset.get() > 0 {
            self.scroll_offset.set(0);
        }

        // --- Input Area ---
        let input_display = format!("{}", self.input);
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
        self.cursor_y.set(chunks[1].y + 1 + cursor_visual_row - input_scroll);
        self.input_width.set(chunks[1].width.saturating_sub(2));

        let input_paragraph = Paragraph::new(input_display)
            .block(
                Block::bordered()
                    .title("Input")
                    .title_alignment(Alignment::Left)
                    .border_type(BorderType::Double),
            )
            .wrap(Wrap { trim: false })
            .scroll((input_scroll, 0))
            .style(Style::default().fg(Color::Cyan).bg(Color::Black));

        input_paragraph.render(chunks[1], buf);

        // --- Status Area (information) ---
        let status_text = format!(
            " Messages: {} | ↑/↓/←/→ move cursor | Enter send | Ctrl+C/Esc/q quit",
            self.messages.len()
        );
        let status_paragraph = Paragraph::new(status_text)
            .alignment(Alignment::Left)
            .style(Style::default().fg(Color::Gray).bg(Color::Black));

        status_paragraph.render(chunks[2], buf);

        let status_text = format!(
            "opencode-go/deepseek-v4-flash · (xhigh) 
            0.0%/1.0M (auto) 
            github/project (mian) "
        );
        let status_paragraph = Paragraph::new(status_text)
            .alignment(Alignment::Right)
            .style(Style::default().fg(Color::Gray).bg(Color::Black));

        status_paragraph.render(chunks[2], buf);
    }
}
