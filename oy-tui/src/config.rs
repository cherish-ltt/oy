pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const WELCOME_TIPS_VEC: &[&str] = &[
    "OY",
    "Type a message and press Enter to send.\nThen communicate with the LLM to achieve your goal",
    "Use ↑/↓/←/→ to move cursor, Enter to send, Ctrl+C/Esc/q to quit.",
];
