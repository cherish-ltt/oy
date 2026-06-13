#![deny(clippy::cognitive_complexity)]
#![deny(clippy::too_many_arguments)]
#![deny(clippy::too_many_lines)]
pub mod domain;
pub mod infrastructure;

pub use domain::*;
pub use infrastructure::opencode_go_provider::OpenCodeGoProvider;
