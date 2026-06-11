// app.rs
use crate::{
    agent::AgentManager,
    command::{CommandId, CommandRegistry, context_items, theme_items, thinking_items},
    config::{VERSION, WELCOME_TIPS_VEC},
    event::{AppEvent, Event, EventHandler},
    load_config::{GlobalTomlConfig, build_provider_config, register_default_tools},
    message::{
        Message::{self, AgentMessages, ToolCallMsg, UiMessages},
        Status, ToolCallState,
    },
    theme::{DARK_THEME, LIGHT_THEME, Theme},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use oy_agent::{
    CommanderAgent, CreateSubAgentTool, Orchestrator, SkillSummary, TokenUsage,
    agent::{PromptKind, RequestAgent},
    format_token_count,
    infrastructure::{agents::main_agent::MainAgent, persistence, tools::ToolRegistry},
    oy_ai::{ChatMessage, OpenCodeGoProvider, Role},
};
use ratatui::DefaultTerminal;
use std::path::PathBuf;
use std::{
    cell::Cell,
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Instant,
};
use uuid::Uuid;

const MAX_POPUP_ROWS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AgentType {
    MainAgent,
    CommanderAgent,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    RevokeSelect,
    CommandSelector {
        selected: usize,
        scroll_offset: usize,
    },
    SubMenu {
        title: String,
        items: Vec<(String, String)>,
        selected: usize,
        scroll_offset: usize,
    },
    ModelForm {
        step: usize,
        values: [String; 4],
    },
}

/// UI state for tracking sub-agent execution progress.
#[derive(Debug, Clone)]
pub struct SubAgentUiState {
    pub agent_type: String,
    pub task: String,
    pub start_time: Instant,
    pub completed: bool,
    pub success: bool,
    pub summary: String,
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
    /// 上一次渲染时聊天区域的宽度，用于检测 resize 后自动滚动到底部
    pub last_chat_width: Cell<u16>,
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
    /// commander-agent (调度代理)
    pub commander_agent: Option<AgentManager>,
    /// 当前活动 agent
    pub active_agent: AgentType,
    /// 命令注册器
    pub command_registry: CommandRegistry,
    /// 当前界面模式
    pub app_mode: AppMode,
    /// Input 框标题（form 模式下用）
    pub input_title: String,
    /// 当前主题
    pub theme: &'static Theme,
    /// Agent 运行状态（running / pause）
    pub agent_status: Cell<Status>,
    /// 帧计数器（用于 spinner 动画）
    pub tick_counter: Cell<u64>,
    /// 累计token使用量
    pub token_usage: TokenUsage,
    /// 已加载的技能列表
    pub skills: Vec<SkillSummary>,
    /// 等待被 Agent 消费的 Prompt IDs
    pub pending_prompts: Vec<Uuid>,
    /// 子代理运行状态 (CommanderAgent 专用)
    pub sub_agent_states: Vec<SubAgentUiState>,
    /// 共享 session UUID (MainAgent + CommanderAgent 共用)
    pub session_uuid: Option<Uuid>,
}

impl App {
    pub async fn new(session_path: Option<PathBuf>) -> Self {
        let mut messages = VecDeque::new();
        let global_toml_config = GlobalTomlConfig::load();

        // Load skills
        let read_claude = global_toml_config
            .as_ref()
            .and_then(|c| c.read_claude_skills)
            .unwrap_or(true);
        let skills = oy_agent::domain::skill::discover_skills(read_claude);

        let mut main_agent: Option<AgentManager> = None;
        let mut commander_agent: Option<AgentManager> = None;
        let mut session_loaded = false;

        // We'll merge both agents' response receivers into one
        let (merged_response_tx, merged_response_rx) = tokio::sync::mpsc::channel(128);

        // ── Generate a session UUID: from loaded session or fresh ──
        let shared_session_uuid = if let Some(path) = &session_path {
            // Try to extract UUID from the loaded session path
            match persistence::load_session_messages(path) {
                Ok((uuid, ref history_msgs)) => {
                    // Populate TUI messages from history (skip system prompt)
                    for msg in history_msgs {
                        if msg.role != Role::System {
                            messages.push_back(Message::AgentMessages(msg.clone(), false));
                        }
                    }

                    // Start main agent with session uuid + history
                    if let Some(global_toml_config) = &global_toml_config
                        && config_is_complete(global_toml_config)
                    {
                        let mut agent = start_agent_with_session(
                            global_toml_config,
                            uuid,
                            history_msgs.clone(),
                        )
                        .await;
                        let rx = agent.response_receiver.take();
                        if let Some(mut rx) = rx {
                            let tx = merged_response_tx.clone();
                            tokio::spawn(async move {
                                while let Some(msg) = rx.recv().await {
                                    if tx.send(msg).await.is_err() {
                                        break;
                                    }
                                }
                            });
                        }
                        main_agent = Some(agent);

                        // Also start commander agent with the same session
                        let mut cmd_agent = start_commander_agent_with_session(
                            global_toml_config,
                            uuid,
                            history_msgs.clone(),
                        )
                        .await;
                        let rx = cmd_agent.response_receiver.take();
                        if let Some(mut rx) = rx {
                            let tx = merged_response_tx.clone();
                            tokio::spawn(async move {
                                while let Some(msg) = rx.recv().await {
                                    if tx.send(msg).await.is_err() {
                                        break;
                                    }
                                }
                            });
                        }
                        commander_agent = Some(cmd_agent);
                    }

                    messages.push_back(Message::UiMessages(
                        "Session restored. Continue the conversation below.".to_string(),
                    ));
                    session_loaded = true;
                    uuid
                }
                Err(e) => {
                    messages.push_back(Message::UiMessages(format!(
                        "Failed to load session: {}. Starting fresh.",
                        e
                    )));
                    Uuid::now_v7()
                }
            }
        } else {
            Uuid::now_v7()
        };

        // ── Start agents (if config is complete) ──
        if let Some(global_toml_config) = &global_toml_config
            && config_is_complete(global_toml_config)
        {
            // Start main agent with shared uuid (no history for fresh start)
            if !session_loaded {
                let mut agent =
                    start_main_agent_background(global_toml_config, shared_session_uuid).await;
                let rx = agent.response_receiver.take();
                if let Some(mut rx) = rx {
                    let tx = merged_response_tx.clone();
                    tokio::spawn(async move {
                        while let Some(msg) = rx.recv().await {
                            if tx.send(msg).await.is_err() {
                                break;
                            }
                        }
                    });
                }
                main_agent = Some(agent);
            }

            // Start commander agent with the SAME shared uuid
            if commander_agent.is_none() {
                let mut cmd_agent =
                    start_commander_agent_background(global_toml_config, shared_session_uuid).await;
                let rx = cmd_agent.response_receiver.take();
                if let Some(mut rx) = rx {
                    let tx = merged_response_tx.clone();
                    tokio::spawn(async move {
                        while let Some(msg) = rx.recv().await {
                            if tx.send(msg).await.is_err() {
                                break;
                            }
                        }
                    });
                }
                commander_agent = Some(cmd_agent);
            }
        }

        // ── Fresh (session not loaded) path: add welcome tips ──
        if !session_loaded {
            WELCOME_TIPS_VEC.iter().for_each(|tip| {
                if tip.eq(&"OY") {
                    messages.push_back(Message::UiMessages(format!("{} {}", tip, VERSION)));
                } else {
                    messages.push_back(Message::UiMessages(tip.to_string()));
                }
            });
        }

        // Show config hint if no agent can start due to missing fields
        if main_agent.is_none() && commander_agent.is_none() && !session_loaded {
            let has_config = global_toml_config.is_some();
            if !has_config || !config_is_complete(global_toml_config.as_ref().unwrap()) {
                messages.push_back(Message::UiMessages(
                    "Welcome to OY! Please configure your API settings using /model command or /settings menu.".to_string()
                ));
            }
        }

        // Skills banner (append after session messages if any)
        if !skills.is_empty() {
            let skill_names: Vec<String> = skills
                .iter()
                .map(|s| format!("{}/{}", s.folder_name, s.name))
                .collect();
            messages.push_back(Message::UiMessages(format!(
                "[Available Skills] \n{}",
                skill_names.join(", ")
            )));
        }

        // Pass skills to both agents
        if let Some(ref agent_manager) = main_agent {
            let _ = agent_manager
                .request_sender
                .send(RequestAgent::SetSkills(skills.clone()))
                .await;
        }

        // Drop the original merged_tx so the receiver closes when all forwarders stop
        drop(merged_response_tx);

        let events = EventHandler::new_with_receiver(merged_response_rx);

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

        // Determine initial scroll offset: session loads should show latest messages
        let initial_scroll_offset = if session_loaded { u16::MAX } else { 0 };

        Self {
            running: true,
            messages,
            input: String::new(),
            cursor_pos: 0,
            cursor_x: Cell::new(0),
            cursor_y: Cell::new(0),
            input_width: Cell::new(0),
            scroll_offset: Cell::new(initial_scroll_offset),
            auto_scroll: Cell::new(true),
            last_chat_width: Cell::new(0),
            paste_snippets: HashMap::new(),
            paste_counter: 0,
            events,
            global_toml_config,
            main_agent,
            commander_agent,
            active_agent: AgentType::MainAgent,
            command_registry,
            app_mode: AppMode::Normal,
            input_title: String::new(),
            theme,
            agent_status: Cell::new(Status::Pause),
            tick_counter: Cell::new(0),
            token_usage: TokenUsage::new(),
            skills,
            pending_prompts: Vec::new(),
            sub_agent_states: Vec::new(),
            session_uuid: Some(shared_session_uuid),
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
                    AppEvent::TokenUsage(token_usage) => {
                        self.token_usage = token_usage;
                    }
                    AppEvent::AgentError(e) => {
                        self.insert_before_queued(UiMessages(format!("errors: {}", e)));
                        if self.auto_scroll.get() {
                            self.scroll_offset.set(u16::MAX);
                        }
                    }
                    AppEvent::Pause => {
                        self.agent_status.set(Status::Pause);
                    }
                    AppEvent::Running => {
                        self.agent_status.set(Status::Running);
                    }
                    AppEvent::PromptConsumed { id } => {
                        self.pending_prompts.retain(|x| *x != id);
                        // Remove the queuing message from the UI
                        self.messages.retain(|msg| match msg {
                            Message::PromptQueued { id: queued_id, .. } => *queued_id != id,
                            _ => true,
                        });
                        if self.auto_scroll.get() {
                            self.scroll_offset.set(u16::MAX);
                        }
                    }
                    AppEvent::PromptQueued { id } => {
                        // The Enter/Alt+Enter handler already added the Message::PromptQueued
                        // and pushed the id to pending_prompts. This event from the reactor
                        // is just a confirmation that the prompt was queued on the agent side.
                        // If somehow the id isn't tracked yet (shouldn't happen), add it.
                        if !self.pending_prompts.contains(&id) {
                            self.pending_prompts.push(id);
                        }
                    }
                },
            }
        }
        Ok(())
    }

    async fn handle_chat_message(&mut self, chat_message: oy_agent::oy_ai::ChatMessage) {
        use oy_agent::oy_ai::Role;

        // Assistant message with tool calls: split into content + ToolCallMsg
        if chat_message.role == Role::Assistant
            && let Some(tool_calls) = &chat_message.tool_calls
            && !tool_calls.is_empty()
        {
            // Push assistant content (thinking/reasoning) without tool calls
            let mut content_msg = chat_message.clone();
            content_msg.tool_calls = None;
            if content_msg.content.is_some() || content_msg.reasoning_content.is_some() {
                self.insert_before_queued(AgentMessages(content_msg, false));
            }
            // Push a ToolCallMsg for each tool call
            for tc in tool_calls {
                // Track sub-agent tool calls for the status panel
                if tc.function_name == "create_sub_agent" {
                    let task = tc
                        .arguments
                        .get("task")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let agent_type = tc
                        .arguments
                        .get("agent_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("sub-agent")
                        .to_string();
                    self.sub_agent_states.push(SubAgentUiState {
                        agent_type,
                        task,
                        start_time: Instant::now(),
                        completed: false,
                        success: false,
                        summary: String::new(),
                    });
                }

                self.insert_before_queued(ToolCallMsg(ToolCallState {
                    function_name: tc.function_name.clone(),
                    arguments: if tc.function_name.eq("Read")
                        || tc.function_name.eq("Edit")
                        || tc.function_name.eq("Write")
                    {
                        tc.arguments
                            .get("file_path")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    } else if tc.function_name.eq("Bash") {
                        tc.arguments
                            .get("command")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    } else {
                        None
                    },
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

        // Tool result: find matching ToolCallMsg by tool_call_id
        if chat_message.role == Role::Tool
            && let Some(call_id) = &chat_message.tool_call_id
        {
            let mut sub_agent_error: Option<String> = None;
            for msg in self.messages.iter_mut().rev() {
                if let ToolCallMsg(state) = msg
                    && state.result.is_none()
                    && state.tool_call_id == *call_id
                {
                    // Mark sub-agent as completed if applicable
                    if state.function_name == "create_sub_agent" {
                        let success = chat_message
                            .content
                            .as_deref()
                            .map(|c| {
                                !c.contains("失败")
                                    && !c.contains("Internal error")
                                    && !c.contains("Error:")
                            })
                            .unwrap_or(false);
                        if let Some(last) = self
                            .sub_agent_states
                            .iter_mut()
                            .rev()
                            .find(|s| !s.completed)
                        {
                            last.completed = true;
                            last.success = success;
                            last.summary = chat_message.content.clone().unwrap_or_default();
                        }
                        if !success {
                            sub_agent_error = chat_message
                                .content
                                .as_deref()
                                .map(|c| c.lines().next().unwrap_or("unknown error").to_string());
                        }
                    }
                    state.result = Some(chat_message);
                    state.end_time = Some(Instant::now());
                    break;
                }
            }
            if let Some(err) = sub_agent_error {
                self.insert_before_queued(UiMessages(format!("⚠ Sub-agent error: {}", err)));
            }
            if self.auto_scroll.get() {
                self.scroll_offset.set(u16::MAX);
            }
            return;
        }

        // Regular message (no tool calls / no tool result): push as-is
        self.insert_before_queued(AgentMessages(chat_message, false));
        if self.auto_scroll.get() {
            self.scroll_offset.set(u16::MAX);
        }
    }

    pub async fn handle_key_events(&mut self, key_event: KeyEvent) -> color_eyre::Result<()> {
        // Ctrl+O: toggle expand/collapse — works in ALL modes
        if key_event.code == KeyCode::Char('o') && key_event.modifiers == KeyModifiers::CONTROL {
            for msg in self.messages.iter_mut().rev() {
                match msg {
                    Message::AgentMessages(_, expanded) => {
                        *expanded = !*expanded;
                        break;
                    }
                    Message::ToolCallMsg(state) => {
                        state.expanded = !state.expanded;
                        break;
                    }
                    _ => {}
                }
            }
            return Ok(());
        }

        match self.app_mode {
            AppMode::Normal => self.handle_key_normal(key_event).await,
            AppMode::RevokeSelect => self.handle_key_revoke_select(key_event).await,
            AppMode::CommandSelector { selected, .. } => {
                self.handle_key_command_selector(key_event, selected).await
            }
            AppMode::ModelForm { .. } => self.handle_key_model_form(key_event).await,
            AppMode::SubMenu { .. } => self.handle_key_submenu(key_event).await,
        }
    }

    async fn handle_key_normal(&mut self, key_event: KeyEvent) -> color_eyre::Result<()> {
        match key_event.code {
            KeyCode::Esc | KeyCode::Char('q') => self.events.send(AppEvent::Quit),
            KeyCode::Char('r' | 'R')
                if key_event.modifiers == KeyModifiers::CONTROL
                    && !self.pending_prompts.is_empty() =>
            {
                self.app_mode = AppMode::RevokeSelect;
            }
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

                // Determine prompt kind: Alt+Enter = AltEnter, Enter = Enter
                let kind = if key_event.modifiers == KeyModifiers::ALT {
                    PromptKind::AltEnter
                } else {
                    PromptKind::Enter
                };

                // Check for slash commands — exact match only, otherwise send as prompt
                if input.starts_with('/') && self.execute_command(&input).await {
                    // Handled as a command
                } else if self.active_agent == AgentType::MainAgent {
                    // ── Route to MainAgent ──
                    if let Some(main_agent) = &self.main_agent {
                        if self.pending_prompts.len() >= 9 {
                            self.insert_before_queued(UiMessages(
                                "Maximum 9 prompts can be queued. Press Ctrl+R then 1..9 to revoke a queued prompt first.".to_string()
                            ));
                            if self.auto_scroll.get() {
                                self.scroll_offset.set(u16::MAX);
                            }
                            return Ok(());
                        }

                        let id = Uuid::now_v7();
                        let _ = main_agent
                            .request_sender
                            .send(RequestAgent::Prompt {
                                text: input.clone(),
                                id,
                                kind,
                            })
                            .await;
                        self.pending_prompts.push(id);
                        self.insert_before_queued(Message::PromptQueued { id, text: input });
                    } else {
                        self.insert_before_queued(UiMessages(
                            "MainAgent not initialized. Please use /model to configure your API key and model first.".to_string()
                        ));
                    }
                } else {
                    // ── Route to CommanderAgent ──
                    if let Some(cmd_agent) = &self.commander_agent {
                        if self.pending_prompts.len() >= 9 {
                            self.insert_before_queued(UiMessages(
                                "Maximum 9 prompts can be queued. Press Ctrl+R then 1..9 to revoke a queued prompt first.".to_string()
                            ));
                            if self.auto_scroll.get() {
                                self.scroll_offset.set(u16::MAX);
                            }
                            return Ok(());
                        }

                        let id = Uuid::now_v7();
                        let _ = cmd_agent
                            .request_sender
                            .send(RequestAgent::Prompt {
                                text: input.clone(),
                                id,
                                kind,
                            })
                            .await;
                        self.pending_prompts.push(id);
                        self.insert_before_queued(Message::PromptQueued { id, text: input });
                    } else {
                        self.insert_before_queued(UiMessages(
                            "CommanderAgent not initialized.".to_string(),
                        ));
                    }
                }

                if self.auto_scroll.get() {
                    self.scroll_offset.set(u16::MAX);
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
            // Shift+Tab (BackTab) — switch between MainAgent and CommanderAgent
            KeyCode::BackTab => {
                self.switch_agent().await;
            }
            KeyCode::Tab if key_event.modifiers == KeyModifiers::SHIFT => {
                self.switch_agent().await;
            }
            // Ctrl+O is handled at top-level handle_key_events; do not re-handle here.
            KeyCode::Char(c) => {
                self.input.insert(self.cursor_pos, c);
                self.cursor_pos += c.len_utf8();
                // Enter command mode when input starts with "/" and matches known commands
                if self.input == "/"
                    || (self.input.starts_with('/')
                        && self.input.len() > 1
                        && !self.command_registry.search(&self.input).is_empty())
                {
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
                let scroll_offset = Self::adjust_scroll(new_sel, matches.len(), MAX_POPUP_ROWS);
                self.app_mode = AppMode::CommandSelector {
                    selected: new_sel,
                    scroll_offset,
                };
            }
            KeyCode::Down => {
                let new_sel = if selected >= max_idx { 0 } else { selected + 1 };
                let scroll_offset = Self::adjust_scroll(new_sel, matches.len(), MAX_POPUP_ROWS);
                self.app_mode = AppMode::CommandSelector {
                    selected: new_sel,
                    scroll_offset,
                };
            }
            KeyCode::Enter if !matches.is_empty() => {
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

                // Determine if this is a single-field form by checking input_title
                let is_single = matches!(
                    self.input_title.as_str(),
                    "API Base URL:" | "API Key:" | "Model:" | "Custom Context Capacity (tokens):"
                ) && step == 0;

                if is_single {
                    // Single-field: save the specific setting
                    let val = values[0].clone();
                    match self.input_title.as_str() {
                        "API Base URL:" => self.switch_single_setting("base_url", &val).await,
                        "API Key:" => self.switch_single_setting("api_key", &val).await,
                        "Model:" => self.switch_single_setting("model", &val).await,
                        "Custom Context Capacity (tokens):" => {
                            if let Ok(n) = val.trim().parse::<u64>() {
                                self.switch_context_capacity(n).await;
                            } else {
                                self.insert_before_queued(UiMessages(format!(
                                    "Invalid context capacity: {}",
                                    val
                                )));
                                if self.auto_scroll.get() {
                                    self.scroll_offset.set(u16::MAX);
                                }
                            }
                        }
                        _ => {}
                    }
                    self.app_mode = AppMode::Normal;
                    self.input_title.clear();
                } else if step == 3 {
                    // Full 4-field form complete
                    let [url, key, model, ctx] = std::mem::take(&mut values);
                    self.execute_model_command(url, key, model, ctx).await;
                    self.app_mode = AppMode::Normal;
                    self.input_title.clear();
                } else {
                    let new_step = step + 1;
                    self.app_mode = AppMode::ModelForm {
                        step: new_step,
                        values,
                    };
                    self.input_title = match new_step {
                        1 => "API Key:".to_string(),
                        2 => "Model:".to_string(),
                        3 => "Context Capacity (tokens, e.g. 200000):".to_string(),
                        _ => unreachable!(),
                    };
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
        let mut pending_ws = 0u16;
        for (i, ch) in self.input.char_indices() {
            if i >= self.cursor_pos {
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
        col += pending_ws;
        if col >= width as u16 {
            row += 1;
            col = 0;
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

        if let Some(rel) = before.rfind("[paste #")
            && rel >= search_from
        {
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
                ..
            } => (title.clone(), items.clone(), *selected),
            _ => return Ok(()),
        };
        let (title, items, selected) = snapshot;
        let max_idx = items.len().saturating_sub(1);

        match key_event.code {
            KeyCode::Up => {
                let new_sel = if selected == 0 { max_idx } else { selected - 1 };
                let new_scroll = Self::adjust_scroll(new_sel, items.len(), MAX_POPUP_ROWS);
                self.app_mode = AppMode::SubMenu {
                    title,
                    items,
                    selected: new_sel,
                    scroll_offset: new_scroll,
                };
            }
            KeyCode::Down => {
                let new_sel = if selected >= max_idx { 0 } else { selected + 1 };
                let new_scroll = Self::adjust_scroll(new_sel, items.len(), MAX_POPUP_ROWS);
                self.app_mode = AppMode::SubMenu {
                    title,
                    items,
                    selected: new_sel,
                    scroll_offset: new_scroll,
                };
            }
            KeyCode::Enter if !items.is_empty() => {
                let item = &items[selected.min(max_idx)];
                self.execute_submenu_item(&title, &item.0).await;
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
                let items: Vec<(String, String)> = theme_items()
                    .iter()
                    .map(|c| (c.name.to_string(), c.description.to_string()))
                    .collect();
                self.app_mode = AppMode::SubMenu {
                    title: format!("{} {}", parent_title, item_name),
                    items,
                    selected: 0,
                    scroll_offset: 0,
                };
            }
            "/settings" if item_name == "/thinking" => {
                // Open thinking effort submenu
                let items: Vec<(String, String)> = thinking_items()
                    .iter()
                    .map(|c| (c.name.to_string(), c.description.to_string()))
                    .collect();
                self.app_mode = AppMode::SubMenu {
                    title: format!("{} {}", parent_title, item_name),
                    items,
                    selected: 0,
                    scroll_offset: 0,
                };
            }
            "/settings" if item_name == "/context" => {
                // Open context capacity submenu
                let items: Vec<(String, String)> = context_items()
                    .iter()
                    .map(|c| (c.name.to_string(), c.description.to_string()))
                    .collect();
                self.app_mode = AppMode::SubMenu {
                    title: format!("{} {}", parent_title, item_name),
                    items,
                    selected: 0,
                    scroll_offset: 0,
                };
            }
            _ => {
                // Collect all known leaf items
                let matched_id = theme_items()
                    .iter()
                    .chain(thinking_items().iter())
                    .chain(context_items().iter())
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
                    Some(CommandId::SetBaseUrl) => {
                        self.input_title = "API Base URL:".to_string();
                        self.app_mode = AppMode::ModelForm {
                            step: 0,
                            values: [String::new(), String::new(), String::new(), String::new()],
                        };
                        return;
                    }
                    Some(CommandId::SetApiKey) => {
                        self.input_title = "API Key:".to_string();
                        self.app_mode = AppMode::ModelForm {
                            step: 0,
                            values: [String::new(), String::new(), String::new(), String::new()],
                        };
                        return;
                    }
                    Some(CommandId::SetModel) => {
                        self.input_title = "Model:".to_string();
                        self.app_mode = AppMode::ModelForm {
                            step: 0,
                            values: [String::new(), String::new(), String::new(), String::new()],
                        };
                        return;
                    }
                    Some(CommandId::ReadClaudeSkills) => {
                        self.switch_claude_skills().await;
                    }
                    Some(id)
                        if matches!(
                            id,
                            CommandId::ThinkingNone
                                | CommandId::ThinkingLow
                                | CommandId::ThinkingMedium
                                | CommandId::ThinkingHigh
                                | CommandId::ThinkingXhigh
                        ) =>
                    {
                        let effort = match id {
                            CommandId::ThinkingNone => "none",
                            CommandId::ThinkingLow => "low",
                            CommandId::ThinkingMedium => "medium",
                            CommandId::ThinkingHigh => "high",
                            CommandId::ThinkingXhigh => "xhigh",
                            _ => unreachable!(),
                        };
                        self.switch_reasoning_effort(effort).await;
                    }
                    Some(id)
                        if matches!(
                            id,
                            CommandId::ContextSize32k
                                | CommandId::ContextSize64k
                                | CommandId::ContextSize128k
                                | CommandId::ContextSize200k
                                | CommandId::ContextSize512k
                                | CommandId::ContextSize1M
                                | CommandId::ContextSizeCustom
                        ) =>
                    {
                        let capacity = match id {
                            CommandId::ContextSize32k => 32_768,
                            CommandId::ContextSize64k => 65_536,
                            CommandId::ContextSize128k => 131_072,
                            CommandId::ContextSize200k => 200_000,
                            CommandId::ContextSize512k => 524_288,
                            CommandId::ContextSize1M => 1_048_576,
                            CommandId::ContextSizeCustom => 0, // will open form
                            _ => unreachable!(),
                        };
                        if id == CommandId::ContextSizeCustom {
                            self.input_title = "Custom Context Capacity (tokens):".to_string();
                            self.app_mode = AppMode::ModelForm {
                                step: 0,
                                values: [
                                    String::new(),
                                    String::new(),
                                    String::new(),
                                    String::new(),
                                ],
                            };
                            return;
                        } else {
                            self.switch_context_capacity(capacity).await;
                        }
                    }
                    _ => {
                        self.insert_before_queued(UiMessages(format!(
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

    /// Execute a slash command. Returns true if input was recognized as a command.
    async fn execute_command(&mut self, input: &str) -> bool {
        // Clear any residual input text (e.g. from CommandSelector auto-complete)
        self.input.clear();
        self.cursor_pos = 0;

        let trimmed = input.trim();

        // Check if any top-level command matches and has children → open submenu
        if let Some(cmd) = self
            .command_registry
            .commands
            .iter()
            .find(|c| c.name == trimmed)
        {
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
                    scroll_offset: 0,
                };
            } else {
                // Leaf command (currently only /model)
                self.input_title = "API Base URL (step 1/4):".to_string();
                self.app_mode = AppMode::ModelForm {
                    step: 0,
                    values: [String::new(), String::new(), String::new(), String::new()],
                };
            }
            return true;
        }

        // Not a recognized command
        false
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
            reasoning_effort: None,
            context_capacity: None,
            read_claude_skills: None,
        };
        let _ = config.save();

        self.insert_before_queued(UiMessages(format!("Switched to {} theme", self.theme.name)));
        if self.auto_scroll.get() {
            self.scroll_offset.set(u16::MAX);
        }
    }

    /// Switch reasoning effort, save config, and restart agent with new provider.
    async fn switch_reasoning_effort(&mut self, effort: &str) {
        // Save config
        let config = GlobalTomlConfig {
            base_url: None,
            api_key: None,
            model: None,
            theme: None,
            reasoning_effort: Some(effort.to_string()),
            context_capacity: None,
            read_claude_skills: None,
        };
        if let Err(e) = config.save() {
            self.insert_before_queued(UiMessages(format!("Failed to save config: {}", e)));
            if self.auto_scroll.get() {
                self.scroll_offset.set(u16::MAX);
            }
            return;
        }

        // Update in-memory config
        if let Some(ref mut global_config) = self.global_toml_config {
            global_config.reasoning_effort = Some(effort.to_string());
        }

        // Only restart agent if all required fields are configured
        if let Some(ref global_config) = self.global_toml_config
            && config_is_complete(global_config)
        {
            let ai_config = build_provider_config(global_config);
            let provider = OpenCodeGoProvider::new(ai_config);
            if let Some(agent_manager) = &self.main_agent {
                let _ = agent_manager
                    .request_sender
                    .send(RequestAgent::SetProvider(Box::new(provider)))
                    .await;
            }
        }

        self.insert_before_queued(UiMessages(format!(
            "Switched reasoning effort to: {}",
            effort
        )));
        if self.auto_scroll.get() {
            self.scroll_offset.set(u16::MAX);
        }
    }

    /// Switch context capacity, save config, and restart agent.
    async fn switch_context_capacity(&mut self, capacity: u64) {
        // Save config
        let config = GlobalTomlConfig {
            base_url: None,
            api_key: None,
            model: None,
            theme: None,
            reasoning_effort: None,
            context_capacity: Some(capacity),
            read_claude_skills: None,
        };
        if let Err(e) = config.save() {
            self.insert_before_queued(UiMessages(format!("Failed to save config: {}", e)));
            if self.auto_scroll.get() {
                self.scroll_offset.set(u16::MAX);
            }
            return;
        }

        // Update in-memory config
        if let Some(ref mut global_config) = self.global_toml_config {
            global_config.context_capacity = Some(capacity);
        }

        // Only restart agent if all required fields are configured
        if let Some(ref global_config) = self.global_toml_config
            && config_is_complete(global_config)
        {
            let ai_config = build_provider_config(global_config);
            let provider = OpenCodeGoProvider::new(ai_config);
            if let Some(agent_manager) = &self.main_agent {
                let _ = agent_manager
                    .request_sender
                    .send(RequestAgent::SetProvider(Box::new(provider)))
                    .await;
            }
        }

        self.insert_before_queued(UiMessages(format!(
            "Switched context capacity to: {}",
            format_token_count(capacity),
        )));
        if self.auto_scroll.get() {
            self.scroll_offset.set(u16::MAX);
        }
    }

    /// Update a single config field (base_url / api_key / model) and restart agent.
    async fn switch_single_setting(&mut self, field: &str, value: &str) {
        let mut config = GlobalTomlConfig {
            base_url: None,
            api_key: None,
            model: None,
            theme: None,
            reasoning_effort: None,
            context_capacity: None,
            read_claude_skills: None,
        };

        // Preserve existing values from in-memory config
        if let Some(ref global) = self.global_toml_config {
            config.base_url = global.base_url.clone();
            config.api_key = global.api_key.clone();
            config.model = global.model.clone();
            config.reasoning_effort = global.reasoning_effort.clone();
            config.context_capacity = global.context_capacity;
            config.theme = global.theme.clone();
        }

        // Override the specific field
        match field {
            "base_url" => config.base_url = Some(value.to_string()),
            "api_key" => config.api_key = Some(value.to_string()),
            "model" => config.model = Some(value.to_string()),
            _ => {}
        }

        if let Err(e) = config.save() {
            self.insert_before_queued(UiMessages(format!("Failed to save config: {}", e)));
            if self.auto_scroll.get() {
                self.scroll_offset.set(u16::MAX);
            }
            return;
        }
        self.global_toml_config = Some(config);

        // Check if config has all required fields before restarting the agent
        let cfg = self.global_toml_config.as_ref().unwrap();
        if config_is_complete(cfg) {
            // All required fields present: restart agent with new provider
            let ai_config = build_provider_config(cfg);
            let provider = OpenCodeGoProvider::new(ai_config);
            if let Some(agent_manager) = &self.main_agent {
                let _ = agent_manager
                    .request_sender
                    .send(RequestAgent::SetProvider(Box::new(provider)))
                    .await;
            }
            self.insert_before_queued(UiMessages(format!("Updated {} to: {}", field, value)));
        } else {
            // Some fields missing: show helpful message instead of crashing
            let mut missing = Vec::new();
            if cfg.api_key.as_deref().is_none_or(str::is_empty) {
                missing.push("api_key");
            }
            if cfg.base_url.as_deref().is_none_or(str::is_empty) {
                missing.push("base_url");
            }
            if cfg.model.as_deref().is_none_or(str::is_empty) {
                missing.push("model");
            }
            self.insert_before_queued(UiMessages(format!(
                "Saved {}. Still need to configure: {}  (use /settings submenu or /model command)",
                field,
                missing.join(", ")
            )));
        }
        if self.auto_scroll.get() {
            self.scroll_offset.set(u16::MAX);
        }
    }

    /// Execute /model with collected values: save config and restart agent.
    async fn execute_model_command(
        &mut self,
        base_url: String,
        api_key: String,
        model: String,
        context_capacity: String,
    ) {
        // Parse context capacity, default to 200000
        let ctx_val: Option<u64> = context_capacity.trim().parse().ok();
        // 1. Save config
        let config = GlobalTomlConfig {
            base_url: Some(base_url.clone()),
            api_key: Some(api_key.clone()),
            model: Some(model.clone()),
            theme: None,
            reasoning_effort: None, // preserve existing
            context_capacity: ctx_val,
            read_claude_skills: None,
        };
        if let Err(e) = config.save() {
            self.insert_before_queued(UiMessages(format!("Failed to save config: {}", e)));
            if self.auto_scroll.get() {
                self.scroll_offset.set(u16::MAX);
            }
            return;
        }
        self.global_toml_config = Some(config);

        // 2. Build new provider + registry
        let Some(ref global_config) = self.global_toml_config else {
            return;
        };
        let ai_config = build_provider_config(global_config);
        let provider = OpenCodeGoProvider::new(ai_config);
        if let Some(agent_manager) = &self.main_agent {
            let _ = agent_manager
                .request_sender
                .send(RequestAgent::SetProvider(Box::new(provider)))
                .await;
        }

        self.insert_before_queued(UiMessages(format!(
            "Switched to model: {} , context: {} , please start the conversation again",
            model,
            ctx_val.map_or("200k".to_string(), format_token_count),
        )));
        if self.auto_scroll.get() {
            self.scroll_offset.set(u16::MAX);
        }
    }

    /// Toggle reading ~/.claude/skills/ on/off.
    async fn switch_claude_skills(&mut self) {
        let current = self
            .global_toml_config
            .as_ref()
            .and_then(|c| c.read_claude_skills)
            .unwrap_or(true);
        let new_val = !current;

        // Save to config
        let config = GlobalTomlConfig {
            base_url: None,
            api_key: None,
            model: None,
            theme: None,
            reasoning_effort: None,
            context_capacity: None,
            read_claude_skills: Some(new_val),
        };
        if let Err(e) = config.save() {
            self.insert_before_queued(UiMessages(format!("Failed to save config: {}", e)));
            if self.auto_scroll.get() {
                self.scroll_offset.set(u16::MAX);
            }
            return;
        }

        // Update in-memory config
        if let Some(ref mut global_config) = self.global_toml_config {
            global_config.read_claude_skills = Some(new_val);
        }

        // Re-discover skills with new setting
        let new_skills = oy_agent::domain::skill::discover_skills(new_val);
        self.skills = new_skills.clone();

        // Send updated skills to agent
        if let Some(ref agent_manager) = self.main_agent {
            let _ = agent_manager
                .request_sender
                .send(RequestAgent::SetSkills(new_skills))
                .await;
        }

        let status = if new_val { "on" } else { "off" };
        self.insert_before_queued(UiMessages(format!(
            "Reading ~/.claude/skills/ is now {}",
            status
        )));
        if self.auto_scroll.get() {
            self.scroll_offset.set(u16::MAX);
        }

        self.app_mode = AppMode::Normal;
        self.input.clear();
        self.cursor_pos = 0;
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

    /// Handle number selection in revoke mode (Alt+R then 1-9).
    async fn handle_key_revoke_select(&mut self, key_event: KeyEvent) -> color_eyre::Result<()> {
        match key_event.code {
            KeyCode::Char(c @ '1'..='9') => {
                let target_number = (c as u8) - b'1' + 1;
                self.revoke_prompt_by_number(target_number).await;
                self.app_mode = AppMode::Normal;
            }
            KeyCode::Esc | KeyCode::Enter => {
                self.app_mode = AppMode::Normal;
            }
            _ => {}
        }
        Ok(())
    }

    /// Insert a message right before the first PromptQueued (so queued prompts stay at bottom).
    fn insert_before_queued(&mut self, msg: Message) {
        let pos = self
            .messages
            .iter()
            .position(|m| matches!(m, Message::PromptQueued { .. }));
        match pos {
            Some(idx) => self.messages.insert(idx, msg),
            None => self.messages.push_back(msg),
        }
    }

    /// Revoke a queued prompt by its display number (1-9).
    /// Numbers are 1-based positions in the queue (not persistent IDs).
    async fn revoke_prompt_by_number(&mut self, number: u8) {
        // Find the Nth PromptQueued message (1-based) by counting
        let mut count = 0u8;
        let idx = self.messages.iter().position(|m| {
            if matches!(m, Message::PromptQueued { .. }) {
                count += 1;
                count == number
            } else {
                false
            }
        });

        let Some(idx) = idx else {
            return;
        };

        // Extract the id and text
        let (id, text) = match &self.messages[idx] {
            Message::PromptQueued { id, text } => (*id, text.clone()),
            _ => return,
        };

        // Remove from messages
        self.messages.remove(idx);

        // Remove from pending_prompts
        self.pending_prompts.retain(|x| *x != id);

        // Cancel on the active agent
        let active_sender = match self.active_agent {
            AgentType::MainAgent => self.main_agent.as_ref(),
            AgentType::CommanderAgent => self.commander_agent.as_ref(),
        };
        if let Some(agent) = active_sender {
            let _ = agent
                .request_sender
                .send(RequestAgent::CancelPrompt { id })
                .await;
        }

        // Append text to input (not overwrite)
        if !self.input.is_empty() {
            self.input.push('\n');
            self.cursor_pos = self.input.len();
        }
        self.input.push_str(&text);
        self.cursor_pos = self.input.len();
    }

    /// Switch between MainAgent and CommanderAgent.
    pub async fn switch_agent(&mut self) {
        // 1. Get messages from the current (FROM) agent via channel
        let history = match self.active_agent {
            AgentType::MainAgent => self.get_agent_messages(&self.main_agent).await,
            AgentType::CommanderAgent => self.get_agent_messages(&self.commander_agent).await,
        };

        // 2. Switch active agent
        self.active_agent = match self.active_agent {
            AgentType::MainAgent => AgentType::CommanderAgent,
            AgentType::CommanderAgent => AgentType::MainAgent,
        };
        let to_name = match self.active_agent {
            AgentType::MainAgent => "MainAgent",
            AgentType::CommanderAgent => "CommanderAgent",
        };

        // 3. Set messages on the TO agent via channel
        if !history.is_empty() {
            match self.active_agent {
                AgentType::MainAgent => self.set_agent_messages(&self.main_agent, &history).await,
                AgentType::CommanderAgent => {
                    self.set_agent_messages(&self.commander_agent, &history)
                        .await
                }
            }
        }

        self.insert_before_queued(UiMessages(format!("Switched to {}.", to_name)));
        self.scroll_offset.set(u16::MAX);
        self.auto_scroll.set(true);
    }

    /// Get agent's messages via RequestAgent::GetMessages channel.
    async fn get_agent_messages(&self, agent: &Option<AgentManager>) -> Vec<ChatMessage> {
        let agent = match agent {
            Some(a) => a,
            None => return vec![],
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        if agent
            .request_sender
            .send(RequestAgent::GetMessages { tx })
            .await
            .is_err()
        {
            return vec![];
        }
        rx.await.unwrap_or_default()
    }

    /// Set agent's messages via RequestAgent::SetMessages channel.
    async fn set_agent_messages(&self, agent: &Option<AgentManager>, msgs: &[ChatMessage]) {
        let agent = match agent {
            Some(a) => a,
            None => return,
        };
        let _ = agent
            .request_sender
            .send(RequestAgent::SetMessages(msgs.to_vec()))
            .await;
    }

    pub fn tick(&self) {
        self.tick_counter.set(self.tick_counter.get() + 1);
    }

    pub fn quit(&mut self) {
        self.running = false;
    }
}

/// Check if the config has all required fields (api_key, base_url, model) to start an agent.
pub fn config_is_complete(config: &GlobalTomlConfig) -> bool {
    config.api_key.as_ref().is_some_and(|s| !s.is_empty())
        && config.base_url.as_ref().is_some_and(|s| !s.is_empty())
        && config.model.as_ref().is_some_and(|s| !s.is_empty())
}

pub async fn start_agent_with_session(
    global_toml_config: &GlobalTomlConfig,
    session_uuid: Uuid,
    session_messages: Vec<ChatMessage>,
) -> AgentManager {
    let ai_config = build_provider_config(global_toml_config);
    let provider = OpenCodeGoProvider::new(ai_config);
    let mut tool_registry = ToolRegistry::new();
    register_default_tools(&mut tool_registry);

    let main_agent = MainAgent::new(None);
    let (request_sender, response_receiver, join_handle) = Orchestrator::start_with_session(
        main_agent,
        provider,
        tool_registry,
        session_uuid,
        session_messages,
    );

    AgentManager::new(
        "MainAgent".to_owned(),
        join_handle,
        request_sender,
        response_receiver,
    )
}

pub async fn start_main_agent_background(
    global_toml_config: &GlobalTomlConfig,
    session_uuid: Uuid,
) -> AgentManager {
    let ai_config = build_provider_config(global_toml_config);

    let provider = OpenCodeGoProvider::new(ai_config);
    let mut tool_registry = ToolRegistry::new();
    register_default_tools(&mut tool_registry);

    let main_agent = MainAgent::new(None);
    let (request_sender, response_receiver, join_handle) =
        Orchestrator::start_with_session(main_agent, provider, tool_registry, session_uuid, vec![]);

    AgentManager::new(
        "MainAgent".to_owned(),
        join_handle,
        request_sender,
        response_receiver,
    )
}

/// Start the CommanderAgent in the background with a shared session UUID.
///
/// CommanderAgent uses a tool registry that includes both regular file tools
/// (for sub-agents to use) and the `create_sub_agent` meta-tool.
/// Uses the same UUID as MainAgent so both agents persist to the same session file.
pub async fn start_commander_agent_background(
    global_toml_config: &GlobalTomlConfig,
    session_uuid: Uuid,
) -> AgentManager {
    let ai_config = build_provider_config(global_toml_config);

    // Create two provider instances: one for CommanderAgent's own LLM calls,
    // one for sub-agents' LLM calls (shared via Arc).
    let provider = OpenCodeGoProvider::new(ai_config.clone());
    let provider_for_sub_agents = Arc::new(OpenCodeGoProvider::new(ai_config));

    // Create file tool registry for sub-agents (shared via Arc)
    let file_tools = Arc::new({
        let mut r = ToolRegistry::new();
        register_default_tools(&mut r);
        r
    });

    // Create CommanderAgent's tool registry (file tools + create_sub_agent for schema)
    let mut commander_registry = ToolRegistry::new();
    register_default_tools(&mut commander_registry);
    // Register a minimal CreateSubAgentTool so LLM sees the schema.
    // Its execute() is NOT called — acting() handles create_sub_agent directly.
    commander_registry.register(CreateSubAgentTool::new(
        provider_for_sub_agents.clone(),
        file_tools.clone(),
    ));

    let commander_agent = CommanderAgent::new(None);
    let (request_sender, response_receiver, join_handle) = Orchestrator::start_commander(
        commander_agent,
        provider,
        commander_registry,
        provider_for_sub_agents,
        file_tools,
    );

    AgentManager::new(
        "CommanderAgent".to_owned(),
        join_handle,
        request_sender,
        response_receiver,
    )
}

/// Start CommanderAgent with a pre-loaded session (same UUID + history as MainAgent).
pub async fn start_commander_agent_with_session(
    global_toml_config: &GlobalTomlConfig,
    session_uuid: Uuid,
    session_messages: Vec<ChatMessage>,
) -> AgentManager {
    let ai_config = build_provider_config(global_toml_config);

    let provider = OpenCodeGoProvider::new(ai_config.clone());
    let provider_for_sub_agents = Arc::new(OpenCodeGoProvider::new(ai_config));

    let file_tools = Arc::new({
        let mut r = ToolRegistry::new();
        register_default_tools(&mut r);
        r
    });

    // Register a minimal CreateSubAgentTool so LLM sees the schema
    let mut commander_registry = ToolRegistry::new();
    register_default_tools(&mut commander_registry);
    commander_registry.register(CreateSubAgentTool::new(
        provider_for_sub_agents.clone(),
        file_tools.clone(),
    ));

    let commander_agent = CommanderAgent::new(None);
    let (request_sender, response_receiver, join_handle) = Orchestrator::start_commander_with_session(
        commander_agent,
        provider,
        commander_registry,
        session_uuid,
        session_messages,
        provider_for_sub_agents,
        file_tools,
    );

    AgentManager::new(
        "CommanderAgent".to_owned(),
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
    // Commit any trailing pending whitespace (cursor after spaces at end)
    col += pending_ws;
    // Clamp to prevent cursor going beyond terminal width (causes visual glitch)
    if col >= width as u16 {
        row += 1;
        col = 0;
    }
    (row, col)
}
