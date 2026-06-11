use std::collections::VecDeque;

use oy_ai::ChatMessage;
use tokio::sync::{
    mpsc::{Receiver, Sender},
    oneshot,
};

use crate::agent::{AgentState, PromptKind, PromptRequest, RequestAgent, ResponseAgent};

/// Command sent from Reactor to Worker.
// id/kind fields are used by Reactor for queue routing; Worker matches
// with `{ text, .. }` and doesn't need them — suppress dead_code lint.
#[allow(dead_code)]
pub(crate) enum WorkerCommand {
    Prompt {
        text: String,
        id: uuid::Uuid,
        kind: PromptKind,
    },
    FlushEnterQueue(Vec<PromptRequest>),
    SetProvider(Box<dyn oy_ai::AiProvider + Send + Sync>),
    SetSkills(Vec<crate::domain::skill::SkillSummary>),
    /// Worker replies with its agent's messages via the oneshot.
    GetMessages {
        tx: oneshot::Sender<Vec<ChatMessage>>,
    },
    /// Replace the worker's agent message list.
    SetMessages(Vec<ChatMessage>),
}

/// Event sent from Worker to Reactor.
pub(crate) enum WorkerEvent {
    Response(ResponseAgent),
    StateChanged(AgentState),
}

/// Reactor sits between TUI and Worker.
///
/// It manages prompt queuing based on the scheduling policy:
/// - `Enter` prompts: forwarded immediately if idle, queued for next Thinking if busy
/// - `Alt+Enter` prompts: forwarded immediately if idle, queued for next Idle if busy
pub(crate) struct Reactor {
    /// From TUI
    request_rx: Receiver<RequestAgent>,
    /// To TUI
    response_tx: Sender<ResponseAgent>,
    /// To Worker
    worker_cmd_tx: Sender<WorkerCommand>,
    /// From Worker
    worker_event_rx: Receiver<WorkerEvent>,
    /// Queued Enter prompts (injected before next Thinking)
    enter_queue: VecDeque<PromptRequest>,
    /// Queued Alt+Enter prompts (injected when back to Idle)
    alt_queue: VecDeque<PromptRequest>,
    /// Current worker state
    worker_state: AgentState,
}

impl Reactor {
    pub(crate) fn new(
        request_rx: Receiver<RequestAgent>,
        response_tx: Sender<ResponseAgent>,
        worker_cmd_tx: Sender<WorkerCommand>,
        worker_event_rx: Receiver<WorkerEvent>,
    ) -> Self {
        Self {
            request_rx,
            response_tx,
            worker_cmd_tx,
            worker_event_rx,
            enter_queue: VecDeque::new(),
            alt_queue: VecDeque::new(),
            worker_state: AgentState::Idle,
        }
    }

    pub(crate) async fn run(mut self) {
        loop {
            tokio::select! {
                // ── Request from TUI ──
                res = self.request_rx.recv() => {
                    match res {
                        Some(request) => self.handle_request(request).await,
                        None => break,
                    }
                }
                // ── Event from Worker ──
                res = self.worker_event_rx.recv() => {
                    match res {
                        Some(event) => self.handle_worker_event(event).await,
                        None => break,
                    }
                }
            }
        }
    }

    async fn handle_request(&mut self, request: RequestAgent) {
        match request {
            RequestAgent::Prompt { text, id, kind } => {
                match kind {
                    PromptKind::Enter => {
                        if self.worker_state == AgentState::Idle {
                            // Optimistically mark as Thinking to prevent stale-state
                            // race: Worker will transition Idle→Thinking after processing.
                            self.worker_state = AgentState::Thinking;
                            // Idle → forward immediately
                            let _ = self
                                .worker_cmd_tx
                                .send(WorkerCommand::Prompt {
                                    text,
                                    id,
                                    kind: PromptKind::Enter,
                                })
                                .await;
                            let _ = self
                                .response_tx
                                .send(ResponseAgent::PromptConsumed { id })
                                .await;
                        } else {
                            // Busy → queue for next Thinking
                            self.enter_queue.push_back(PromptRequest {
                                text,
                                id,
                                kind: PromptKind::Enter,
                            });
                            let _ = self
                                .response_tx
                                .send(ResponseAgent::PromptQueued { id })
                                .await;
                        }
                    }
                    PromptKind::AltEnter => {
                        if self.worker_state == AgentState::Idle {
                            // Same optimistic update for Alt+Enter
                            self.worker_state = AgentState::Thinking;
                            // Idle → forward immediately
                            let _ = self
                                .worker_cmd_tx
                                .send(WorkerCommand::Prompt {
                                    text,
                                    id,
                                    kind: PromptKind::AltEnter,
                                })
                                .await;
                            let _ = self
                                .response_tx
                                .send(ResponseAgent::PromptConsumed { id })
                                .await;
                        } else {
                            // Busy → queue for next Idle
                            self.alt_queue.push_back(PromptRequest {
                                text,
                                id,
                                kind: PromptKind::AltEnter,
                            });
                            let _ = self
                                .response_tx
                                .send(ResponseAgent::PromptQueued { id })
                                .await;
                        }
                    }
                }
            }
            RequestAgent::CancelPrompt { id } => {
                // Remove from both queues if still pending
                let removed_enter = self.enter_queue.iter().any(|pr| pr.id == id);
                self.enter_queue.retain(|pr| pr.id != id);
                let removed_alt = self.alt_queue.iter().any(|pr| pr.id == id);
                self.alt_queue.retain(|pr| pr.id != id);
                // If found in either queue, notify TUI that it's been cancelled
                if removed_enter || removed_alt {
                    let _ = self
                        .response_tx
                        .send(ResponseAgent::PromptConsumed { id })
                        .await;
                }
            }
            RequestAgent::SetProvider(provider) => {
                let _ = self
                    .worker_cmd_tx
                    .send(WorkerCommand::SetProvider(provider))
                    .await;
            }
            RequestAgent::SetSkills(skills) => {
                let _ = self
                    .worker_cmd_tx
                    .send(WorkerCommand::SetSkills(skills))
                    .await;
            }
            RequestAgent::GetMessages { tx } => {
                let _ = self
                    .worker_cmd_tx
                    .send(WorkerCommand::GetMessages { tx })
                    .await;
            }
            RequestAgent::SetMessages(msgs) => {
                let _ = self
                    .worker_cmd_tx
                    .send(WorkerCommand::SetMessages(msgs))
                    .await;
            }
        }
    }

    async fn handle_worker_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::StateChanged(new_state) => {
                let old_state = std::mem::replace(&mut self.worker_state, new_state);

                // When worker enters ToolCall: flush Enter queue so prompts
                // arrive at cmd_rx before worker transitions back to Thinking.
                // Worker will drain them in tool_call() and inject as user messages
                // alongside tool results, before the next LLM call.
                if self.worker_state == AgentState::ToolCall {
                    self.flush_enter_queue().await;
                }

                // When worker enters Thinking: flush Enter queue (fallback for
                // the no-tool-call path: Thinking→Acting→TaskCompleted→Observing→Idle)
                if self.worker_state == AgentState::Thinking {
                    self.flush_enter_queue().await;
                }

                // When worker transitions to Idle (was busy before): flush all queues
                if self.worker_state == AgentState::Idle && old_state != AgentState::Idle {
                    if !self.enter_queue.is_empty() {
                        self.flush_enter_queue().await;
                    }
                    self.flush_alt_queue().await;
                }
            }
            WorkerEvent::Response(response) => {
                let _ = self.response_tx.send(response).await;
            }
        }
    }

    /// Drain all queued Enter prompts to the Worker.
    async fn flush_enter_queue(&mut self) {
        if self.enter_queue.is_empty() {
            return;
        }
        // Optimistically mark as Thinking before sending commands to Worker
        self.worker_state = AgentState::Thinking;
        let batch: Vec<PromptRequest> = self.enter_queue.drain(..).collect();
        // Notify Worker to inject these prompts
        let _ = self
            .worker_cmd_tx
            .send(WorkerCommand::FlushEnterQueue(batch.clone()))
            .await;
        // Notify TUI that all are consumed
        for pr in batch {
            let _ = self
                .response_tx
                .send(ResponseAgent::PromptConsumed { id: pr.id })
                .await;
        }
    }

    /// Drain all queued Alt+Enter prompts to the Worker.
    async fn flush_alt_queue(&mut self) {
        if self.alt_queue.is_empty() {
            return;
        }
        // Optimistically mark as Thinking before sending commands to Worker
        self.worker_state = AgentState::Thinking;
        let batch: Vec<PromptRequest> = self.alt_queue.drain(..).collect();
        // Send each as a prompt command to the worker
        for pr in &batch {
            let _ = self
                .worker_cmd_tx
                .send(WorkerCommand::Prompt {
                    text: pr.text.clone(),
                    id: pr.id,
                    kind: PromptKind::AltEnter,
                })
                .await;
            let _ = self
                .response_tx
                .send(ResponseAgent::PromptConsumed { id: pr.id })
                .await;
        }
    }
}
