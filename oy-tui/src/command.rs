/// Command registry for slash commands (e.g. /model, /settings)

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CommandId {
    None,
    ThemeLight,
    ThemeDark,
    ThinkingNone,
    ThinkingLow,
    ThinkingMedium,
    ThinkingHigh,
    ThinkingXhigh,
    ContextSize32k,
    ContextSize64k,
    ContextSize128k,
    ContextSize200k,
    ContextSize512k,
    ContextSize1M,
    ContextSizeCustom,
    SetBaseUrl,
    SetApiKey,
    SetModel,
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
                description: "Set full API configuration (base-url, api-key, model, context)",
                children: vec![],
            },
            CommandInfo {
                name: "/settings",
                description: "Open settings menu",
                children: vec![
                    CommandItem {
                        name: "/theme",
                        description: "Switch color theme",
                        id: CommandId::None,
                    },
                    CommandItem {
                        name: "/thinking",
                        description: "Set reasoning effort (none/low/medium/high/xhigh)",
                        id: CommandId::None,
                    },
                    CommandItem {
                        name: "/context",
                        description: "Set context capacity (32k/64k/128k/200k/512k/1M/custom)",
                        id: CommandId::None,
                    },
                    CommandItem {
                        name: "/base-url",
                        description: "Set API base URL",
                        id: CommandId::SetBaseUrl,
                    },
                    CommandItem {
                        name: "/api-key",
                        description: "Set API key",
                        id: CommandId::SetApiKey,
                    },
                    CommandItem {
                        name: "/model-name",
                        description: "Set model name",
                        id: CommandId::SetModel,
                    },
                ],
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

/// Thinking effort items shown when /settings /thinking is selected
pub fn thinking_items() -> Vec<CommandItem> {
    vec![
        CommandItem {
            name: "none",
            description: "No reasoning effort",
            id: CommandId::ThinkingNone,
        },
        CommandItem {
            name: "low",
            description: "Low reasoning effort",
            id: CommandId::ThinkingLow,
        },
        CommandItem {
            name: "medium",
            description: "Medium reasoning effort",
            id: CommandId::ThinkingMedium,
        },
        CommandItem {
            name: "high",
            description: "High reasoning effort (default)",
            id: CommandId::ThinkingHigh,
        },
        CommandItem {
            name: "xhigh",
            description: "Extra high reasoning effort",
            id: CommandId::ThinkingXhigh,
        },
    ]
}

/// Context capacity items shown when /settings /context is selected
pub fn context_items() -> Vec<CommandItem> {
    vec![
        CommandItem {
            name: "32k",
            description: "32,768 tokens",
            id: CommandId::ContextSize32k,
        },
        CommandItem {
            name: "64k",
            description: "65,536 tokens",
            id: CommandId::ContextSize64k,
        },
        CommandItem {
            name: "128k",
            description: "131,072 tokens",
            id: CommandId::ContextSize128k,
        },
        CommandItem {
            name: "200k",
            description: "200,000 tokens (default)",
            id: CommandId::ContextSize200k,
        },
        CommandItem {
            name: "512k",
            description: "524,288 tokens",
            id: CommandId::ContextSize512k,
        },
        CommandItem {
            name: "1M",
            description: "1,048,576 tokens",
            id: CommandId::ContextSize1M,
        },
        CommandItem {
            name: "custom",
            description: "Enter custom context size",
            id: CommandId::ContextSizeCustom,
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
        assert_eq!(settings.children.len(), 6);
        assert_eq!(settings.children[0].name, "/theme");
        assert_eq!(settings.children[1].name, "/thinking");
        assert_eq!(settings.children[2].name, "/context");
        assert_eq!(settings.children[3].name, "/base-url");
        assert_eq!(settings.children[4].name, "/api-key");
        assert_eq!(settings.children[5].name, "/model-name");
    }

    #[test]
    fn test_theme_items() {
        let items = theme_items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, CommandId::ThemeLight);
        assert_eq!(items[1].id, CommandId::ThemeDark);
    }

    #[test]
    fn test_thinking_items() {
        let items = thinking_items();
        assert_eq!(items.len(), 5);
        assert_eq!(items[0].name, "none");
        assert_eq!(items[1].name, "low");
        assert_eq!(items[2].name, "medium");
        assert_eq!(items[3].name, "high");
        assert_eq!(items[4].name, "xhigh");
    }

    #[test]
    fn test_thinking_items_ids() {
        let items = thinking_items();
        assert_eq!(items[0].id, CommandId::ThinkingNone);
        assert_eq!(items[3].id, CommandId::ThinkingHigh);
        assert_eq!(items[4].id, CommandId::ThinkingXhigh);
    }

    #[test]
    fn test_context_items() {
        let items = context_items();
        assert_eq!(items.len(), 7);
        assert_eq!(items[0].name, "32k");
        assert_eq!(items[3].name, "200k");
        assert_eq!(items[5].name, "1M");
        assert_eq!(items[6].name, "custom");
    }

    #[test]
    fn test_context_items_ids() {
        let items = context_items();
        assert_eq!(items[0].id, CommandId::ContextSize32k);
        assert_eq!(items[3].id, CommandId::ContextSize200k);
        assert_eq!(items[5].id, CommandId::ContextSize1M);
        assert_eq!(items[6].id, CommandId::ContextSizeCustom);
    }

    #[test]
    fn test_context_item_descriptions() {
        let items = context_items();
        assert_eq!(items[3].description, "200,000 tokens (default)");
        assert_eq!(items[6].description, "Enter custom context size");
    }
}
