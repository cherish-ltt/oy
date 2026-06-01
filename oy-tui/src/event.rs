// event.rs (unchanged)
use color_eyre::eyre::OptionExt;
use crossterm::event::Event as CrosstermEvent;
use futures::{FutureExt, StreamExt};
use oy_agent::{agent::ResponseAgent, oy_ai::ChatMessage};
use oy_agent::TokenUsage;
use std::time::Duration;
use tokio::sync::mpsc;

const TICK_FPS: f64 = 30.0;

#[derive(Clone, Debug)]
pub enum Event {
    Tick,
    Crossterm(CrosstermEvent),
    App(AppEvent),
}

#[derive(Clone, Debug)]
pub enum AppEvent {
    Quit,
    ChatMessage(ChatMessage),
    TokenUsage(TokenUsage),
    AgentError(String),
    Pause,
    Running,
}

#[derive(Debug)]
pub struct EventHandler {
    sender: mpsc::UnboundedSender<Event>,
    receiver: mpsc::UnboundedReceiver<Event>,
}

impl Default for EventHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl EventHandler {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        let actor = EventTask::new(sender.clone());
        tokio::spawn(async { actor.run().await });
        Self { sender, receiver }
    }

    pub fn new_with_receiver(response_receiver: mpsc::Receiver<ResponseAgent>) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        let actor = EventTask::new_with_response_receiver(sender.clone(), response_receiver);
        tokio::spawn(async { actor.run().await });
        Self { sender, receiver }
    }

    pub async fn next(&mut self) -> color_eyre::Result<Event> {
        self.receiver
            .recv()
            .await
            .ok_or_eyre("Failed to receive event")
    }

    pub fn send(&mut self, app_event: AppEvent) {
        let _ = self.sender.send(Event::App(app_event));
    }
}

struct EventTask {
    sender: mpsc::UnboundedSender<Event>,
    response_receiver: Option<mpsc::Receiver<ResponseAgent>>,
}

impl EventTask {
    fn new(sender: mpsc::UnboundedSender<Event>) -> Self {
        Self {
            sender,
            response_receiver: None,
        }
    }

    fn new_with_response_receiver(
        sender: mpsc::UnboundedSender<Event>,
        response_receiver: mpsc::Receiver<ResponseAgent>,
    ) -> Self {
        Self {
            sender,
            response_receiver: Some(response_receiver),
        }
    }

    async fn run(mut self) -> color_eyre::Result<()> {
        let tick_rate = Duration::from_secs_f64(1.0 / TICK_FPS);
        let mut reader = crossterm::event::EventStream::new();
        let mut tick = tokio::time::interval(tick_rate);
        loop {
            let tick_delay = tick.tick();
            let crossterm_event = reader.next().fuse();
            tokio::select! {
              _ = self.sender.closed() => {
                break;
              }
              _ = tick_delay => {
                self.send(Event::Tick);
              }
              Some(Ok(evt)) = crossterm_event => {
                self.send(Event::Crossterm(evt));
              }
              msg_opt = async {
                    if let Some(rx) = self.response_receiver.as_mut() {
                        rx.recv().await
                    } else {
                        std::future::pending().await
                    }
                } => {
                    match msg_opt {
                        Some(msg) => {
                            match msg {
                                ResponseAgent::Pause => self.send(Event::App(AppEvent::Pause)),
                                ResponseAgent::Running => self.send(Event::App(AppEvent::Running)),
                                ResponseAgent::ChatMessage(chat_message) => self.send(Event::App(AppEvent::ChatMessage(chat_message))),
                                ResponseAgent::TokenUsage(token_usage) => self.send(Event::App(AppEvent::TokenUsage(token_usage))),
                                ResponseAgent::AgentError(agent_error) => self.send(Event::App(AppEvent::AgentError(agent_error.to_string()))),
                            }
                        }
                        None => {
                            self.response_receiver = None;
                            break; // 或继续
                        }
                    }
                }
            };
        }
        Ok(())
    }

    fn send(&self, event: Event) {
        let _ = self.sender.send(event);
    }
}
