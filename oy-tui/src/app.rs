// app.rs
use crate::{
    agent::AgentManager,
    command::{CommandId, CommandRegistry, theme_items},
    event::{AppEvent, Event, EventHandler},
    load_config::{GlobalTomlConfig, build_provider_config, register_default_tools},
    message::{
        Message::{self, AgentMessages, AgentStatus, ToolCallMessage, UiMessages},
        Status, ToolCallState,
    },
    theme::{Theme, DARK_THEME, LIGHT_THEME},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use oy_agent::{
    agent::InputAgentSignal,
    application::orchestrator::{
        start_agent_background, start_agent_background_with_context,
    },
    infrastructure::{agents::main_agent::MainAgent, tools::ToolRegistry},
    oy_ai::OpenCodeGoProvider,
};
const MAX_POPUP_ROWS: usize = 5;

use ratatui::DefaultTerminal;
use std::{
    cell::Cell,
    collections::{HashMap, VecDeque},
    time::Instant,
};

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    CommandSelector {
        selected: usize,
        scroll_offset: usize,
    },
    SubMenu {
        title: String,
        items: Vec<(String, String)>,
        selected: usize,
    },
    ModelForm { step: usize, values: [String; 3] },
}

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
    /// 命令注册器
    pub command_registry: CommandRegistry,
    /// 当前界面模式
    pub app_mode: AppMode,
    /// Input 框标题（form 模式下用）
    pub input_title: String,
    /// 当前主题
    pub theme: &'static Theme,
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

        let command_registry = CommandRegistry::new();

        // Read theme from config, default to light
        let theme: &'static Theme = global_toml_config
            .as_ref()
            .and_then(|c| c.theme.as_deref())
            .map(|t| match t {
                "dark" => &DARK_THEME,
                _ => &LIGHT_THEME,
            })
            .unwrap_or(&LIGHT_THEME);

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
            command_registry,
            app_mode: AppMode::Normal,
            input_title: String::new(),
            theme,
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
                        self.handle_chat_message(chat_message).await;
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

    async fn handle_chat_message(&mut self, chat_message: oy_agent::oy_ai::ChatMessage) {
        use oy_agent::oy_ai::Role;

        // Assistant message with tool calls: split into content + ToolCallMessage
        if chat_message.role == Role::Assistant {
            if let Some(tool_calls) = &chat_message.tool_calls {
                if !tool_calls.is_empty() {
                    // Push assistant content (thinking/reasoning) without tool calls
                    let mut content_msg = chat_message.clone();
                    content_msg.tool_calls = None;
                    if content_msg.content.is_some() || content_msg.reasoning_content.is_some() {
                        self.messages.push_back(AgentMessages(content_msg, false));
                    }
                    // Push a ToolCallMessage for each tool call
                    for tc in tool_calls {
                        self.messages.push_back(ToolCallMessage(ToolCallState {
                            function_name: tc.function_name.clone(),
                            tool_call_id: tc.id.clone(),
                            result: None,
                            start_time: Instant::now(),
                            end_time: None,
                            expanded: false,
                        }));
                    }
                    if self.auto_scroll.get() {
                        self.scroll_offset.set(u16::MAX);
                    }
                    return;
                }
            }
        }

        // Tool result: find matching ToolCallMessage by tool_call_id
        if chat_message.role == Role::Tool {
            if let Some(call_id) = &chat_message.tool_call_id {
                for msg in self.messages.iter_mut().rev() {
                    if let ToolCallMessage(state) = msg {
                        if state.result.is_none() && state.tool_call_id == *call_id {
                            state.result = Some(chat_message);
                            state.end_time = Some(Instant::now());
                            break;
                        }
                    }
                }
                if self.auto_scroll.get() {
                    self.scroll_offset.set(u16::MAX);
                }
                return;
            }
        }

        // Regular message (no tool calls / no tool result): push as-is
        self.messages.push_back(AgentMessages(chat_message, false));
        if self.auto_scroll.get() {
            self.scroll_offset.set(u16::MAX);
        }
    }

    pub async fn handle_key_events(&mut self, key_event: KeyEvent) -> color_eyre::Result<()> {
        match self.app_mode {
            AppMode::Normal => self.handle_key_normal(key_event).await,
            AppMode::CommandSelector {
                selected, ..
            } => self.handle_key_command_selector(key_event, selected).await,
            AppMode::ModelForm { .. } => {
                self.handle_key_model_form(key_event).await
            }
            AppMode::SubMenu { .. } => {
                self.handle_key_submenu(key_event).await
            }
        }
    }

    async fn handle_key_normal(&mut self, key_event: KeyEvent) -> color_eyre::Result<()> {
        match key_event.code {
            KeyCode::Esc | KeyCode::Char('q') => self.events.send(AppEvent::Quit),
            KeyCode::Char('c' | 'C') if key_event.modifiers == KeyModifiers::CONTROL => {
                if self.input.is_empty() {
                    self.events.send(AppEvent::Quit)
                } else {
                    self.input.clear();
                    self.cursor_pos = 0;
                    // If input started with "/", exit command mode
                    self.app_mode = AppMode::Normal;
                }
            }
            KeyCode::Enter if !self.input.is_empty() => {
                self.expand_paste_snippets();
                let input = std::mem::take(&mut self.input);
                self.cursor_pos = 0;
                self.paste_counter = 0;
                self.scroll_offset.set(u16::MAX);

                // Check for slash commands
                if input.starts_with('/') {
                    self.execute_command(&input).await;
                } else if let Some(main_agent) = &self.main_agent {
                    let _ = main_agent
                        .request_sender
                        .send(InputAgentSignal::UserPrompt(input))
                        .await;
                }
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
                // If input becomes empty after "/", go back to Normal
                if self.input.is_empty() {
                    self.app_mode = AppMode::Normal;
                }
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
            KeyCode::Char('o') if key_event.modifiers == KeyModifiers::CONTROL => {
                for msg in self.messages.iter_mut().rev() {
                    match msg {
                        Message::AgentMessages(_, expanded) => {
                            *expanded = !*expanded;
                            break;
                        }
                        Message::ToolCallMessage(state) => {
                            state.expanded = !state.expanded;
                            break;
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Char(c) => {
                self.input.insert(self.cursor_pos, c);
                self.cursor_pos += c.len_utf8();
                // Enter command mode when input starts with "/"
                if self.input == "/" || (self.input.starts_with('/') && self.input.len() > 1) {
                    self.app_mode = AppMode::CommandSelector {
                        selected: 0,
                        scroll_offset: 0,
                    };
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_key_command_selector(
        &mut self,
        key_event: KeyEvent,
        selected: usize,
    ) -> color_eyre::Result<()> {
        let matches = self.command_registry.search(&self.input);
        let max_idx = matches.len().saturating_sub(1);

        match key_event.code {
            KeyCode::Up => {
                let new_sel = if selected == 0 { max_idx } else { selected - 1 };
                let scroll_offset = Self::adjust_scroll(
                    new_sel,
                    matches.len(),
                    MAX_POPUP_ROWS,
                );
                self.app_mode = AppMode::CommandSelector {
                    selected: new_sel,
                    scroll_offset,
                };
            }
            KeyCode::Down => {
                let new_sel = if selected >= max_idx { 0 } else { selected + 1 };
                let scroll_offset = Self::adjust_scroll(
                    new_sel,
                    matches.len(),
                    MAX_POPUP_ROWS,
                );
                self.app_mode = AppMode::CommandSelector {
                    selected: new_sel,
                    scroll_offset,
                };
            }
            KeyCode::Enter => {
                if !matches.is_empty() {
                    let cmd = matches[selected.min(max_idx)].name;
                    let input = std::mem::take(&mut self.input);
                    self.cursor_pos = 0;
                    // If user typed the full command or selected from menu, execute it
                    if input == cmd || input.starts_with(cmd) {
                        self.execute_command(cmd).await;
                    } else {
                        // Replace input with full command name
                        self.input = cmd.to_string();
                        self.cursor_pos = self.input.len();
                        self.execute_command(cmd).await;
                    }
                }
            }
            KeyCode::Esc => {
                self.app_mode = AppMode::Normal;
                self.input.clear();
                self.cursor_pos = 0;
            }
            KeyCode::Char(c) => {
                self.input.insert(self.cursor_pos, c);
                self.cursor_pos += c.len_utf8();
                // Re-filter; if no matches, fall back to normal
                let new_matches = self.command_registry.search(&self.input);
                if new_matches.is_empty() {
                    self.app_mode = AppMode::Normal;
                } else {
                    self.app_mode = AppMode::CommandSelector {
                        selected: 0,
                        scroll_offset: 0,
                    };
                }
            }
            KeyCode::Backspace if self.cursor_pos > 0 => {
                let len = self.input[..self.cursor_pos]
                    .chars()
                    .last()
                    .map(|c| c.len_utf8())
                    .unwrap_or(0);
                self.input
                    .replace_range(self.cursor_pos - len..self.cursor_pos, "");
                self.cursor_pos -= len;
                if self.input.is_empty() {
                    self.app_mode = AppMode::Normal;
                } else {
                    let new_matches = self.command_registry.search(&self.input);
                    if new_matches.is_empty() {
                        self.app_mode = AppMode::Normal;
                    } else {
                        self.app_mode = AppMode::CommandSelector {
                            selected: 0,
                            scroll_offset: 0,
                        };
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_key_model_form(&mut self, key_event: KeyEvent) -> color_eyre::Result<()> {
        // Clone the current mode to extract values, then mutate
        let snapshot = match &self.app_mode {
            AppMode::ModelForm { step, values } => (*step, values.clone()),
            _ => return Ok(()),
        };
        let (step, mut values) = snapshot;

        match key_event.code {
            KeyCode::Esc => {
                self.app_mode = AppMode::Normal;
                self.input.clear();
                self.cursor_pos = 0;
                self.input_title.clear();
            }
            KeyCode::Enter if !self.input.is_empty() => {
                values[step] = std::mem::take(&mut self.input);
                self.cursor_pos = 0;
                if step == 2 {
                    let [url, key, model] =
                        std::mem::take(&mut values);
                    self.execute_model_command(url, key, model).await;
                    self.app_mode = AppMode::Normal;
                    self.input_title.clear();
                } else {
                    let new_step = step + 1;
                    self.app_mode = AppMode::ModelForm {
                        step: new_step,
                        values,
                    };
                    self.input_title = match new_step {
                        1 => "API Key:",
                        2 => "Model:",
                        _ => unreachable!(),
                    }
                    .to_string();
                }
            }
            KeyCode::Char(c) => {
                self.input.insert(self.cursor_pos, c);
                self.cursor_pos += c.len_utf8();
            }
            KeyCode::Backspace if self.cursor_pos > 0 => {
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
        let mut pending_ws = 0u16; // trailing whitespace not yet committed
        for (i, ch) in self.input.char_indices() {
            if i >= self.cursor_pos {
                // commit pending whitespace before breaking
                col += pending_ws;
                break;
            }
            if ch == '\n' {
                pending_ws = 0;
                row += 1;
                col = 0;
            } else {
                let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
                if ch.is_ascii_whitespace() && ch != '\n' {
                    // Check if this whitespace would cause overflow on its own
                    if col + pending_ws + w > width as u16 {
                        // Whitespace at line end: drop it entirely, start new line empty
                        row += 1;
                        pending_ws = 0;
                        col = 0;
                    } else {
                        pending_ws += w;
                    }
                } else {
                    // Non-whitespace: check if word fits with preceding whitespace
                    if col + pending_ws + w > width as u16 {
                        // Doesn't fit: wrap, drop trailing whitespace
                        row += 1;
                        col = w;
                    } else {
                        col += pending_ws + w;
                    }
                    pending_ws = 0;
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
        let mut pending_ws = 0u16;
        let mut best = 0usize;

        for (i, ch) in self.input.char_indices() {
            if row > target_row {
                break;
            }

            if ch == '\n' {
                if row == target_row {
                    break;
                }
                pending_ws = 0;
                row += 1;
                col = 0;
                continue;
            }

            let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1) as u16;

            if ch.is_ascii_whitespace() && ch != '\n' {
                if col + pending_ws + w > width as u16 {
                    // whitespace at line end: drop it entirely, start new line empty
                    pending_ws = 0;
                    row += 1;
                    col = 0;
                } else {
                    // Update best for target_row tracking (whitespace accumulates, not committed yet)
                    // best tracks the last committed position
                    pending_ws += w;
                }
                continue;
            }

            // Non-whitespace
            if col + pending_ws + w > width as u16 {
                // Doesn't fit: wrap, drop trailing whitespace
                row += 1;
                if row > target_row {
                    break;
                }
                col = w;
                pending_ws = 0;
                if row == target_row {
                    if target_col == 0 || target_col < w {
                        best = i;
                    } else {
                        best = i + ch.len_utf8();
                    }
                }
            } else {
                col += pending_ws + w;
                pending_ws = 0;
                if row == target_row {
                    if col == target_col || (target_col > col - w && target_col < col) {
                        return i;
                    }
                    best = i + ch.len_utf8();
                }
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
        let mut pending_ws = 0u16;
        for ch in self.input.chars() {
            if ch == '\n' {
                pending_ws = 0;
                lines += 1;
                col = 0;
            } else {
                let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
                if ch.is_ascii_whitespace() && ch != '\n' {
                    if col + pending_ws + w > width as u16 {
                        // Whitespace at line end: drop entirely, new line empty
                        pending_ws = 0;
                        lines += 1;
                        col = 0;
                    } else {
                        pending_ws += w;
                    }
                } else {
                    if col + pending_ws + w > width as u16 {
                        // Word wraps: drop trailing whitespace
                        lines += 1;
                        col = w;
                    } else {
                        col += pending_ws + w;
                    }
                    pending_ws = 0;
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

    /// Execute a slash command (called after input is already taken from self.input).
    async fn handle_key_submenu(&mut self, key_event: KeyEvent) -> color_eyre::Result<()> {
        // Extract current submenu state
        let snapshot = match &self.app_mode {
            AppMode::SubMenu {
                title,
                items,
                selected,
            } => (title.clone(), items.clone(), *selected),
            _ => return Ok(()),
        };
        let (title, items, selected) = snapshot;
        let max_idx = items.len().saturating_sub(1);

        match key_event.code {
            KeyCode::Up => {
                let new_sel = if selected == 0 { max_idx } else { selected - 1 };
                self.app_mode = AppMode::SubMenu {
                    title,
                    items,
                    selected: new_sel,
                };
            }
            KeyCode::Down => {
                let new_sel = if selected >= max_idx {
                    0
                } else {
                    selected + 1
                };
                self.app_mode = AppMode::SubMenu {
                    title,
                    items,
                    selected: new_sel,
                };
            }
            KeyCode::Enter => {
                if !items.is_empty() {
                    let item = &items[selected.min(max_idx)];
                    self.execute_submenu_item(&title, &item.0).await;
                }
            }
            KeyCode::Esc => {
                self.app_mode = AppMode::Normal;
                self.input.clear();
                self.cursor_pos = 0;
            }
            _ => {}
        }
        Ok(())
    }

    async fn execute_submenu_item(&mut self, parent_title: &str, item_name: &str) {
        match parent_title {
            "/settings" if item_name == "/theme" => {
                // Open theme submenu
                let theme_items = theme_items();
                let items: Vec<(String, String)> = theme_items
                    .iter()
                    .map(|c| (c.name.to_string(), c.description.to_string()))
                    .collect();
                self.app_mode = AppMode::SubMenu {
                    title: format!("{} {}", parent_title, item_name),
                    items,
                    selected: 0,
                };
            }
            _ => {
                // Check if item has a CommandId
                let matched_id = theme_items()
                    .iter()
                    .chain(
                        self.command_registry
                            .commands
                            .iter()
                            .flat_map(|c| c.children.iter()),
                    )
                    .find(|ci| ci.name == item_name)
                    .map(|ci| ci.id);

                match matched_id {
                    Some(CommandId::ThemeLight) => self.switch_theme("light"),
                    Some(CommandId::ThemeDark) => self.switch_theme("dark"),
                    _ => {
                        self.messages.push_back(UiMessages(format!(
                            "Unknown submenu item: {}",
                            item_name
                        )));
                        if self.auto_scroll.get() {
                            self.scroll_offset.set(u16::MAX);
                        }
                    }
                }
                self.app_mode = AppMode::Normal;
                self.input.clear();
                self.cursor_pos = 0;
            }
        }
    }

    async fn execute_command(&mut self, input: &str) {
        let trimmed = input.trim();

        // Check if any top-level command matches and has children → open submenu
        if let Some(cmd) = self.command_registry.search(trimmed).first() {
            if !cmd.children.is_empty() {
                let items: Vec<(String, String)> = cmd
                    .children
                    .iter()
                    .map(|c| (c.name.to_string(), c.description.to_string()))
                    .collect();
                let title = cmd.name.to_string();
                self.app_mode = AppMode::SubMenu {
                    title,
                    items,
                    selected: 0,
                };
                return;
            }
        }

        if trimmed == "/model" || trimmed.starts_with("/model ") {
            self.input_title = "API Base URL:".to_string();
            self.app_mode = AppMode::ModelForm {
                step: 0,
                values: [String::new(), String::new(), String::new()],
            };
        } else {
            self.messages
                .push_back(UiMessages(format!("Unknown command: {}", trimmed)));
            if self.auto_scroll.get() {
                self.scroll_offset.set(u16::MAX);
            }
        }
    }

    fn switch_theme(&mut self, name: &str) {
        self.theme = match name {
            "light" => &LIGHT_THEME,
            _ => &DARK_THEME,
        };

        // Persist to config
        let config = GlobalTomlConfig {
            base_url: None,
            api_key: None,
            model: None,
            theme: Some(name.to_string()),
        };
        let _ = config.save();

        self.messages
            .push_back(UiMessages(format!("Switched to {} theme", self.theme.name)));
        if self.auto_scroll.get() {
            self.scroll_offset.set(u16::MAX);
        }
    }

    /// Execute /model with collected values: save config and restart agent.
    async fn execute_model_command(&mut self, base_url: String, api_key: String, model: String) {
        // 1. Save config
        let config = GlobalTomlConfig {
            base_url: Some(base_url.clone()),
            api_key: Some(api_key.clone()),
            model: Some(model.clone()),
            theme: None,
        };
        if let Err(e) = config.save() {
            self.messages
                .push_back(UiMessages(format!("Failed to save config: {}", e)));
            if self.auto_scroll.get() {
                self.scroll_offset.set(u16::MAX);
            }
            return;
        }
        self.global_toml_config = Some(config);

        // 2. Extract messages from old agent
        let old_messages = if let Some(ref agent) = self.main_agent {
            agent.extract_messages().await
        } else {
            vec![]
        };

        // 3. Build new provider + registry
        let global_config = self.global_toml_config.as_ref().unwrap();
        let ai_config = build_provider_config(global_config);
        let provider = OpenCodeGoProvider::new(ai_config);
        let mut registry = ToolRegistry::new();
        register_default_tools(&mut registry);
        let main_agent = MainAgent::new_with_max_iterations(None);

        // 4. Start new agent with old context
        let (request_sender, response_receiver, join_handle) =
            start_agent_background_with_context(main_agent, provider, registry, old_messages).await;

        let mut new_agent = AgentManager::new(
            "MainAgent".to_owned(),
            join_handle,
            request_sender,
            response_receiver,
        );

        // 5. Replace event handler
        let new_events = if let Some(response_receiver) = new_agent.response_receiver.take() {
            EventHandler::new_with_receiver(response_receiver)
        } else {
            EventHandler::new()
        };

        self.main_agent = Some(new_agent);
        self.events = new_events;

        self.messages
            .push_back(UiMessages(format!("Switched to model: {}", model)));
        if self.auto_scroll.get() {
            self.scroll_offset.set(u16::MAX);
        }
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

    /// Adjust scroll offset so `selected` stays visible within `max_visible` rows.
    fn adjust_scroll(selected: usize, total: usize, max_visible: usize) -> usize {
        if total <= max_visible {
            return 0;
        }
        if selected < max_visible {
            // First page
            0
        } else if selected + max_visible >= total {
            // Last page
            total - max_visible
        } else {
            // Middle — keep selected near middle of visible window
            selected - max_visible / 2
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
    let mut pending_ws = 0u16;
    for (i, ch) in input.char_indices() {
        if i >= cursor_pos {
            col += pending_ws;
            break;
        }
        if ch == '\n' {
            pending_ws = 0;
            row += 1;
            col = 0;
        } else {
            let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
            if ch.is_ascii_whitespace() && ch != '\n' {
                if col + pending_ws + w > width as u16 {
                    row += 1;
                    pending_ws = 0;
                    col = 0;
                } else {
                    pending_ws += w;
                }
            } else {
                if col + pending_ws + w > width as u16 {
                    row += 1;
                    col = w;
                } else {
                    col += pending_ws + w;
                }
                pending_ws = 0;
            }
        }
    }
    (row, col)
}
