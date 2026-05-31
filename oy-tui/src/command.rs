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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_new_has_two_commands() {
        let reg = CommandRegistry::new();
        assert_eq!(reg.commands.len(), 2);
    }

    #[test]
    fn test_search_empty_input() {
        let reg = CommandRegistry::new();
        let result = reg.search("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_search_slash_returns_all() {
        let reg = CommandRegistry::new();
        let result = reg.search("/");
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_search_model_prefix() {
        let reg = CommandRegistry::new();
        let result = reg.search("/mo");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "/model");
    }

    #[test]
    fn test_search_settings_prefix() {
        let reg = CommandRegistry::new();
        let result = reg.search("/se");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "/settings");
    }

    #[test]
    fn test_search_no_match() {
        let reg = CommandRegistry::new();
        let result = reg.search("/xyz");
        assert!(result.is_empty());
    }

    #[test]
    fn test_commands_sorted_alphabetically() {
        let reg = CommandRegistry::new();
        assert_eq!(reg.commands[0].name, "/model");
        assert_eq!(reg.commands[1].name, "/settings");
    }

    #[test]
    fn test_model_has_no_children() {
        let reg = CommandRegistry::new();
        let model = &reg.commands[0];
        assert!(model.children.is_empty());
    }

    #[test]
    fn test_settings_has_children() {
        let reg = CommandRegistry::new();
        let settings = &reg.commands[1];
        assert_eq!(settings.children.len(), 1);
        assert_eq!(settings.children[0].name, "/theme");
    }

    #[test]
    fn test_theme_items() {
        let items = theme_items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, CommandId::ThemeLight);
        assert_eq!(items[1].id, CommandId::ThemeDark);
    }
}
