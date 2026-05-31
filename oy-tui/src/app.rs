// app.rs
use crate::{
    agent::AgentManager,
    event::{AppEvent, Event, EventHandler},
    load_config::{GlobalTomlConfig, build_provider_config, register_default_tools},
    message::{
        Message::{self, AgentMessages, AgentStatus, UiMessages},
        Status,
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use oy_agent::{
    agent::InputAgentSignal,
    application::orchestrator::start_agent_background,
    infrastructure::{agents::main_agent::MainAgent, tools::ToolRegistry},
    oy_ai::OpenCodeGoProvider,
};
use ratatui::DefaultTerminal;
use std::{
    cell::Cell,
    collections::{HashMap, VecDeque},
};

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
    /// 是否自动滚动到底部（当用户手动向上翻页时为 false）
    pub auto_scroll: Cell<bool>,
    /// 粘贴片段存储（占位符 -> 原始内容）
    pub paste_snippets: HashMap<String, String>,
    /// 粘贴计数器
    pub paste_counter: usize,
    /// 事件处理器
    pub events: EventHandler,
    /// ai配置
    pub global_toml_config: Option<GlobalTomlConfig>,
    /// main-agent
    pub main_agent: Option<AgentManager>,
}

impl App {
    pub async fn new() -> Self {
        let mut messages = VecDeque::new();
        messages.push_back(Message::UiMessages("OY v0.0.1".to_string()));
        messages.push_back(Message::UiMessages(
            "Type a message and press Enter to send.\nThen communicate with the LLM to achieve your goal".to_string(),
        ));
        messages.push_back(Message::UiMessages(
            "Use ↑/↓/←/→ to move cursor, Enter to send, Ctrl+C/Esc/q to quit.".to_string(),
        ));

        let global_toml_config = GlobalTomlConfig::load();

        let mut main_agent: Option<AgentManager> = None;
        if let Some(global_toml_config) = &global_toml_config {
            main_agent = Some(start_main_agent_background(global_toml_config).await);
        }

        let events = if let Some(agent_manager) = &mut main_agent {
            if let Some(response_receiver) = agent_manager.response_receiver.take() {
                EventHandler::new_with_receiver(response_receiver)
            } else {
                EventHandler::new()
            }
        } else {
            EventHandler::new()
        };

        Self {
            running: true,
            messages,
            input: String::new(),
            cursor_pos: 0,
            cursor_x: Cell::new(0),
            cursor_y: Cell::new(0),
            input_width: Cell::new(0),
            scroll_offset: Cell::new(0),
            auto_scroll: Cell::new(true),
            paste_snippets: HashMap::new(),
            paste_counter: 0,
            events,
            global_toml_config,
            main_agent,
        }
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
                        self.handle_key_events(key_event).await?
                    }
                    crossterm::event::Event::Paste(text) => {
                        self.handle_paste(&text);
                    }
                    crossterm::event::Event::Mouse(mouse_event) => match mouse_event.kind {
                        MouseEventKind::ScrollDown => {
                            self.scroll_offset
                                .set(self.scroll_offset.get().saturating_add(3));
                        }
                        MouseEventKind::ScrollUp => {
                            self.scroll_offset
                                .set(self.scroll_offset.get().saturating_sub(3));
                            // 用户手动向上翻页（看旧消息），禁用自动滚动到底部
                            self.auto_scroll.set(false);
                        }
                        _ => {}
                    },
                    _ => {}
                },
                Event::App(app_event) => match app_event {
                    AppEvent::Quit => self.quit(),
                    AppEvent::ChatMessage(chat_message) => {
                        self.messages.push_back(AgentMessages(chat_message));
                        if self.auto_scroll.get() {
                            self.scroll_offset.set(u16::MAX);
                        }
                    }
                    AppEvent::AgentError(e) => {
                        self.messages
                            .push_back(UiMessages(format!("errors: {}", e)));
                        if self.auto_scroll.get() {
                            self.scroll_offset.set(u16::MAX);
                        }
                    }
                    AppEvent::Pause => {
                        self.messages.push_back(AgentStatus(Status::Pause));
                        if self.auto_scroll.get() {
                            self.scroll_offset.set(u16::MAX);
                        }
                    }
                    AppEvent::Running => {
                        self.messages.push_back(AgentStatus(Status::Running));
                        if self.auto_scroll.get() {
                            self.scroll_offset.set(u16::MAX);
                        }
                    }
                },
            }
        }
        Ok(())
    }

    pub async fn handle_key_events(&mut self, key_event: KeyEvent) -> color_eyre::Result<()> {
        match key_event.code {
            KeyCode::Esc | KeyCode::Char('q') => self.events.send(AppEvent::Quit),
            KeyCode::Char('c' | 'C') if key_event.modifiers == KeyModifiers::CONTROL => {
                if self.input.is_empty() {
                    self.events.send(AppEvent::Quit)
                } else {
                    self.input.clear();
                    self.cursor_pos = 0;
                }
            }
            KeyCode::Enter if !self.input.is_empty() => {
                self.expand_paste_snippets();
                if let Some(main_agent) = &self.main_agent {
                    let _ = main_agent
                        .request_sender
                        .send(InputAgentSignal::UserPrompt(self.input.clone()))
                        .await;
                }
                self.input.clear();
                self.cursor_pos = 0;
                self.paste_counter = 0;
                self.scroll_offset.set(u16::MAX);
            }
            KeyCode::Backspace if self.cursor_pos > 0 && !self.delete_paste_placeholder() => {
                let len = self.input[..self.cursor_pos]
                    .chars()
                    .last()
                    .map(|c| c.len_utf8())
                    .unwrap_or(0);
                self.input
                    .replace_range(self.cursor_pos - len..self.cursor_pos, "");
                self.cursor_pos -= len;
            }
            KeyCode::Left if self.cursor_pos > 0 => {
                let len = self.input[..self.cursor_pos]
                    .chars()
                    .last()
                    .map(|c| c.len_utf8())
                    .unwrap_or(0);
                self.cursor_pos -= len;
            }
            KeyCode::Right if self.cursor_pos < self.input.len() => {
                let len = self.input[self.cursor_pos..]
                    .chars()
                    .next()
                    .map(|c| c.len_utf8())
                    .unwrap_or(0);
                self.cursor_pos += len;
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
            KeyCode::Char('v') if key_event.modifiers == KeyModifiers::CONTROL => {
                self.paste_from_clipboard();
            }
            KeyCode::Char('V')
                if key_event.modifiers == KeyModifiers::CONTROL | KeyModifiers::SHIFT =>
            {
                self.paste_from_clipboard();
            }
            KeyCode::Char('v') if key_event.modifiers == KeyModifiers::ALT => {
                self.paste_from_clipboard();
            }
            KeyCode::Insert if key_event.modifiers == KeyModifiers::SHIFT => {
                self.paste_from_clipboard();
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
        for (i, ch) in self.input.char_indices() {
            if i >= self.cursor_pos {
                break;
            }
            if ch == '\n' {
                row += 1;
                col = 0;
            } else {
                let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
                if col + w > width as u16 {
                    col = w;
                    row += 1;
                } else {
                    col += w;
                }
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

        for (i, ch) in self.input.char_indices() {
            if row > target_row {
                break;
            }

            if ch == '\n' {
                if row == target_row {
                    break;
                }
                row += 1;
                col = 0;
                continue;
            }

            let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1) as u16;

            if row == target_row {
                if col == target_col || (target_col > col && target_col < col + w) {
                    return i;
                }
                best = i + ch.len_utf8();
            }

            if col + w > width as u16 {
                col = w;
                row += 1;
                if row == target_row {
                    // Character wraps to target_row, starts at visual column 0
                    if target_col < w {
                        best = i; // Snap to start of character (beginning of line or inside char)
                    } else {
                        best = i + ch.len_utf8(); // Past the character
                    }
                }
            } else {
                col += w;
            }
        }

        if row < target_row {
            self.input.len()
        } else {
            best
        }
    }

    pub(crate) fn total_visual_lines(&self, width: usize) -> u16 {
        if self.input.is_empty() || width == 0 {
            return 1;
        }
        let mut lines = 1u16;
        let mut col = 0u16;
        for ch in self.input.chars() {
            if ch == '\n' {
                lines += 1;
                col = 0;
            } else {
                let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
                if col + w > width as u16 {
                    col = w;
                    lines += 1;
                } else {
                    col += w;
                }
            }
        }
        lines
    }

    fn handle_paste(&mut self, raw: &str) {
        let mut text = raw.to_string();
        // Strip UTF-8 BOM if present
        if text.starts_with('\u{FEFF}') {
            text = text[3..].to_string();
        }
        // Normalize line endings: CRLF → LF, then stray CR → LF
        text = text.replace("\r\n", "\n").replace('\r', "\n");
        // Trim trailing newline(s)
        let text = text.trim_end_matches('\n').to_string();

        if text.is_empty() {
            return;
        }

        let line_count = text.lines().count();

        if line_count >= 2 {
            self.paste_counter += 1;
            let snippet_id = format!("paste #{}", self.paste_counter);
            let placeholder = format!("[{} +{} lines]", snippet_id, line_count);
            self.input.insert_str(self.cursor_pos, &placeholder);
            self.cursor_pos += placeholder.len();
            self.paste_snippets.insert(snippet_id, text);
        } else {
            self.input.insert_str(self.cursor_pos, &text);
            self.cursor_pos += text.len();
        }
    }

    fn paste_from_clipboard(&mut self) {
        let output = match std::process::Command::new("pbpaste").output() {
            Ok(o) => o,
            Err(_) => return,
        };
        if !output.status.success() {
            return;
        }
        let text = match String::from_utf8(output.stdout) {
            Ok(t) => t,
            Err(_) => return,
        };
        self.handle_paste(&text);
    }

    fn delete_paste_placeholder(&mut self) -> bool {
        if self.cursor_pos == 0 {
            return false;
        }

        // Only check the last ~256 bytes before cursor to avoid scanning entire input
        let search_from = self.cursor_pos.saturating_sub(256);
        let before = &self.input[..self.cursor_pos];

        if !before.ends_with(']') {
            return false;
        }

        if let Some(rel) = before.rfind("[paste #") {
            if rel >= search_from {
                let placeholder = &self.input[rel..self.cursor_pos];

                if placeholder.len() > 14 && placeholder.ends_with(" lines]") {
                    let inner = &placeholder[1..placeholder.len() - 1];
                    let parts: Vec<&str> = inner.splitn(3, ' ').collect();
                    if parts.len() == 3 && parts[0] == "paste" {
                        let id = parts[1].to_string();
                        self.input.replace_range(rel..self.cursor_pos, "");
                        self.cursor_pos = rel;
                        self.paste_snippets.remove(&id);
                        return true;
                    }
                }
            }
        }
        false
    }

    fn expand_paste_snippets(&mut self) {
        let snippets = std::mem::take(&mut self.paste_snippets);
        for (id, content) in snippets {
            let placeholder = format!("[{} +{} lines]", id, content.lines().count());
            while let Some(pos) = self.input.find(&placeholder) {
                self.input
                    .replace_range(pos..pos + placeholder.len(), &content);
                if pos + placeholder.len() <= self.cursor_pos {
                    self.cursor_pos = self.cursor_pos - placeholder.len() + content.len();
                } else if pos < self.cursor_pos {
                    self.cursor_pos = pos + content.len();
                }
            }
        }
    }

    pub fn tick(&self) {
        // Can be used for periodic updates (e.g., polling)
    }

    pub fn quit(&mut self) {
        self.running = false;
    }
}

pub async fn start_main_agent_background(global_toml_config: &GlobalTomlConfig) -> AgentManager {
    let ai_config = build_provider_config(global_toml_config);

    let provider = OpenCodeGoProvider::new(ai_config);
    let mut registry = ToolRegistry::new();
    register_default_tools(&mut registry);

    let main_agent = MainAgent::new_with_max_iterations(None);
    let (request_sender, response_receiver, join_handle) =
        start_agent_background(main_agent, provider, registry).await;

    AgentManager::new(
        "MainAgent".to_owned(),
        join_handle,
        request_sender,
        response_receiver,
    )
}

pub(crate) fn visual_cursor_pos(input: &str, cursor_pos: usize, width: usize) -> (u16, u16) {
    if input.is_empty() || width == 0 {
        return (0, 0);
    }
    let mut row = 0u16;
    let mut col = 0u16;
    for (i, ch) in input.char_indices() {
        if i >= cursor_pos {
            break;
        }
        if ch == '\n' {
            row += 1;
            col = 0;
        } else {
            let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
            if col + w > width as u16 {
                col = w;
                row += 1;
            } else {
                col += w;
            }
        }
    }
    (row, col)
}
