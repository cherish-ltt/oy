#![deny(clippy::cognitive_complexity)]
#![deny(clippy::too_many_arguments)]
#![deny(clippy::too_many_lines)]
pub mod application;
pub mod domain;
pub mod infrastructure;

pub use application::orchestrator::Orchestrator;
pub use domain::*;
// Re-export CommanderAgent for use in TUI layer
pub use infrastructure::agents::commander_agent::CommanderAgent;
// Re-export SubAgentRunner and related types
pub use infrastructure::agents::sub_agent_runner::{SubAgentConfig, SubAgentEvent, run_sub_agent};
// Re-export the meta-tool (registered only for its JSON Schema — execute() is
// handled by Worker's special-cased async path, not via Tool::execute).
pub use infrastructure::tools::sub_agent_tool::CreateSubAgentTool;
pub extern crate oy_ai;
