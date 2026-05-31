/// Command registry for slash commands (e.g. /model, /settings)

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CommandId {
    None,
    ThemeLight,
    ThemeDark,
}

#[derive(Debug, Clone)]
pub struct CommandItem {
    pub name: &'static str,
    pub description: &'static str,
    pub id: CommandId,
}

#[derive(Debug, Clone)]
pub struct CommandInfo {
    pub name: &'static str,
    pub description: &'static str,
    pub children: Vec<CommandItem>,
}

#[derive(Debug)]
pub struct CommandRegistry {
    pub commands: Vec<CommandInfo>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        let mut cmds = vec![
            CommandInfo {
                name: "/model",
                description: "Set API configuration (base-url, api-key, model)",
                children: vec![],
            },
            CommandInfo {
                name: "/settings",
                description: "Open settings menu",
                children: vec![CommandItem {
                    name: "/theme",
                    description: "Switch color theme",
                    id: CommandId::None,
                }],
            },
        ];
        cmds.sort_by(|a, b| a.name.cmp(b.name));
        Self { commands: cmds }
    }

    /// Return all top-level commands whose name starts with `input`.
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

/// Theme items shown when /settings /theme is selected
pub fn theme_items() -> Vec<CommandItem> {
    vec![
        CommandItem {
            name: "light",
            description: "Light theme",
            id: CommandId::ThemeLight,
        },
        CommandItem {
            name: "dark",
            description: "Dark theme",
            id: CommandId::ThemeDark,
        },
    ]
}
