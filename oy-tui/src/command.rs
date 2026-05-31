/// Command registry for slash commands (e.g. /model)

#[derive(Debug, Clone)]
pub struct CommandInfo {
    pub name: &'static str,
    pub description: &'static str,
}

#[derive(Debug)]
pub struct CommandRegistry {
    pub commands: Vec<CommandInfo>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: vec![CommandInfo {
                name: "/model",
                description: "Set API configuration (base-url, api-key, model)",
            }],
        }
    }

    /// Return all commands whose name starts with `input`.
    pub fn search(&self, input: &str) -> Vec<&CommandInfo> {
        if input.is_empty() {
            return vec![];
        }
        self.commands
            .iter()
            .filter(|c| c.name.starts_with(input))
            .collect()
    }
}
