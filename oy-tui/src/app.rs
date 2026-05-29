// app.rs
use crate::{
    event::{AppEvent, Event, EventHandler},
    message::Message,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::DefaultTerminal;
use std::{cell::Cell, collections::VecDeque};

/// Application state
#[derive(Debug)]
pub struct App {
    /// 应用程序正在运行吗？
    pub running: bool,
    /// 消息记录（每条条目为一行）
    pub messages: VecDeque<Message>,
    /// 当前输入缓冲区
    pub input: String,
    /// 光标在 input 中的 byte 位置
    pub cursor_pos: usize,
    /// 光标屏幕 x 坐标（计算后设置）
    pub cursor_x: Cell<u16>,
    /// 光标屏幕 y 坐标（计算后设置）
    pub cursor_y: Cell<u16>,
    /// input 区域可用宽度（用于计算换行后的光标位置）
    pub input_width: Cell<u16>,
    /// 消息区域的垂直滚动偏移量（从顶部数行的数量）
    pub scroll_offset: Cell<u16>,
    /// 事件处理器
    pub events: EventHandler,
}

impl Default for App {
    fn default() -> Self {
        let mut messages = VecDeque::new();
        messages.push_back(Message::UiMessages(
            "Welcome to Claude Code Chat!".to_string(),
        ));
        messages.push_back(Message::UiMessages(
            "Type a message and press Enter to send.".to_string(),
        ));
        messages.push_back(Message::UiMessages(
            "Use ↑/↓/←/→ to move cursor, Enter to send, Ctrl+C/Esc/q to quit.".to_string(),
        ));
        Self {
            running: true,
            messages,
            input: String::new(),
            cursor_pos: 0,
            cursor_x: Cell::new(0),
            cursor_y: Cell::new(0),
            input_width: Cell::new(0),
            scroll_offset: Cell::new(0),
            events: EventHandler::new(),
        }
    }
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<()> {
        while self.running {
            terminal.draw(|frame| {
                frame.render_widget(&self, frame.area());
                frame.set_cursor_position((self.cursor_x.get(), self.cursor_y.get()));
            })?;
            match self.events.next().await? {
                Event::Tick => self.tick(),
                Event::Crossterm(event) => match event {
                    crossterm::event::Event::Key(key_event)
                        if key_event.kind == crossterm::event::KeyEventKind::Press =>
                    {
                        self.handle_key_events(key_event)?
                    }
                    _ => {}
                },
                Event::App(app_event) => match app_event {
                    AppEvent::Quit => self.quit(),
                    // Add other custom events if needed
                    _ => {}
                },
            }
        }
        Ok(())
    }

    pub fn handle_key_events(&mut self, key_event: KeyEvent) -> color_eyre::Result<()> {
        match key_event.code {
            KeyCode::Esc | KeyCode::Char('q') => self.events.send(AppEvent::Quit),
            KeyCode::Char('c' | 'C') if key_event.modifiers == KeyModifiers::CONTROL => {
                self.events.send(AppEvent::Quit)
            }
            KeyCode::Enter => {
                if !self.input.is_empty() {
                    self.input.clear();
                    self.cursor_pos = 0;
                    self.scroll_offset.set(u16::MAX);
                }
            }
            KeyCode::Backspace => {
                if self.cursor_pos > 0 {
                    let len = self.input[..self.cursor_pos]
                        .chars()
                        .last()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    self.input.replace_range(self.cursor_pos - len..self.cursor_pos, "");
                    self.cursor_pos -= len;
                }
            }
            KeyCode::Left => {
                if self.cursor_pos > 0 {
                    let len = self.input[..self.cursor_pos]
                        .chars()
                        .last()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    self.cursor_pos -= len;
                }
            }
            KeyCode::Right => {
                if self.cursor_pos < self.input.len() {
                    let len = self.input[self.cursor_pos..]
                        .chars()
                        .next()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    self.cursor_pos += len;
                }
            }
            KeyCode::Up => {
                let width = self.input_width.get() as usize;
                if width > 0 {
                    self.move_cursor_up(width);
                }
            }
            KeyCode::Down => {
                let width = self.input_width.get() as usize;
                if width > 0 {
                    self.move_cursor_down(width);
                }
            }
            KeyCode::Char(c) => {
                self.input.insert(self.cursor_pos, c);
                self.cursor_pos += c.len_utf8();
            }
            _ => {}
        }
        Ok(())
    }

    fn move_cursor_up(&mut self, width: usize) {
        let (row, col) = self.cursor_visual_pos(width);
        if row == 0 {
            return;
        }
        self.cursor_pos = self.byte_at_visual_pos(row - 1, col, width);
    }

    fn move_cursor_down(&mut self, width: usize) {
        let (row, col) = self.cursor_visual_pos(width);
        let total = self.total_visual_lines(width);
        if row + 1 >= total {
            return;
        }
        self.cursor_pos = self.byte_at_visual_pos(row + 1, col, width);
    }

    fn cursor_visual_pos(&self, width: usize) -> (u16, u16) {
        if self.input.is_empty() || width == 0 {
            return (0, 0);
        }
        let mut row = 0u16;
        let mut col = 0u16;
        for (i, _) in self.input.char_indices() {
            if i >= self.cursor_pos {
                break;
            }
            col += 1;
            if col as usize >= width {
                col = 0;
                row += 1;
            }
        }
        (row, col)
    }

    fn byte_at_visual_pos(&self, target_row: u16, target_col: u16, width: usize) -> usize {
        if self.input.is_empty() || width == 0 {
            return 0;
        }
        let mut row = 0u16;
        let mut col = 0u16;
        let mut best = 0usize;

        for (i, c) in self.input.char_indices() {
            if row > target_row {
                break;
            }
            if row == target_row {
                if col == target_col {
                    return i;
                }
                best = i + c.len_utf8();
            }
            col += 1;
            if col as usize >= width {
                col = 0;
                row += 1;
                if row == target_row {
                    best = i + c.len_utf8();
                }
            }
        }

        if row < target_row {
            self.input.len()
        } else {
            best
        }
    }

    fn total_visual_lines(&self, width: usize) -> u16 {
        if self.input.is_empty() || width == 0 {
            return 1;
        }
        let mut lines = 1u16;
        let mut col = 0u16;
        for _ in self.input.chars() {
            col += 1;
            if col as usize >= width {
                col = 0;
                lines += 1;
            }
        }
        lines
    }

    pub fn tick(&self) {
        // Can be used for periodic updates (e.g., polling)
    }

    pub fn quit(&mut self) {
        self.running = false;
    }
}

pub(crate) fn visual_cursor_pos(input: &str, cursor_pos: usize, width: usize) -> (u16, u16) {
    if input.is_empty() || width == 0 {
        return (0, 0);
    }
    let mut row = 0u16;
    let mut col = 0u16;
    for (i, _) in input.char_indices() {
        if i >= cursor_pos {
            break;
        }
        col += 1;
        if col as usize >= width {
            col = 0;
            row += 1;
        }
    }
    (row, col)
}
