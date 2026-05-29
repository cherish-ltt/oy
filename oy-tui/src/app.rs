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
            "Use ↑/↓ to scroll, Ctrl+C/Esc/q to quit.".to_string(),
        ));
        Self {
            running: true,
            messages,
            input: String::new(),
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
            terminal.draw(|frame| frame.render_widget(&self, frame.area()))?;
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
                    // Add the current input to messages
                    // self.messages.push_back(Message::UiMessages(self.input.clone()));
                    self.input.clear();
                    // Scroll to bottom by setting offset to maximum
                    self.scroll_offset.set(u16::MAX);
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Up => {
                // Scroll up: increase offset (but will be clamped in rendering)
                let new_offset = self.scroll_offset.get().saturating_add(1);
                self.scroll_offset.set(new_offset);
            }
            KeyCode::Down => {
                // Scroll down: decrease offset
                let new_offset = self.scroll_offset.get().saturating_sub(1);
                self.scroll_offset.set(new_offset);
            }
            KeyCode::Char(c) => {
                self.input.push(c);
            }
            _ => {}
        }
        Ok(())
    }

    pub fn tick(&self) {
        // Can be used for periodic updates (e.g., polling)
    }

    pub fn quit(&mut self) {
        self.running = false;
    }
}
