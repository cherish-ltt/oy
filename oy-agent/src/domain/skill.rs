use std::path::{Path, PathBuf};

/// Summary of a loaded skill — only metadata, no content.
/// The AI decides whether to `Read` the full file based on this info.
#[derive(Debug, Clone)]
pub struct SkillSummary {
    /// Directory name (unique identifier, e.g. "karpathy-guidelines")
    pub folder_name: String,
    /// `name` from YAML frontmatter inside SKILL.md (e.g. "karpathy-guidelines")
    pub name: String,
    /// `description` from YAML frontmatter
    pub description: String,
    /// Full path to the SKILL.md file
    pub path: PathBuf,
}

/// Parse YAML frontmatter from a SKILL.md file.
/// Returns (name, description) if valid frontmatter is found.
fn parse_skill_frontmatter(content: &str) -> Option<(String, String)> {
    let content = content.trim();
    if !content.starts_with("---") {
        return None;
    }

    // Find the closing ---
    let end = content[3..].find("---")?;
    let frontmatter = &content[3..3 + end];

    let mut name = None;
    let mut description = None;

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(stripped) = line.strip_prefix("name:") {
            name = Some(stripped.trim().trim_matches(['"', '\'']).to_string());
        } else if let Some(stripped) = line.strip_prefix("description:") {
            description = Some(stripped.trim().trim_matches(['"', '\'']).to_string());
        }
    }

    match (name, description) {
        (Some(n), Some(d)) => Some((n, d)),
        _ => None,
    }
}

/// Load all skill summaries from a single directory.
/// Looks for `<dir>/<subdir>/SKILL.md` in the given base path.
#[allow(clippy::too_many_lines)]
pub fn load_skills_from_dir(base_dir: &Path) -> Vec<SkillSummary> {
    let mut skills = Vec::new();

    let entries = match std::fs::read_dir(base_dir) {
        Ok(entries) => entries,
        Err(_) => return skills,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let folder_name = match path.file_name() {
            Some(name) => name.to_string_lossy().to_string(),
            None => continue,
        };

        let skill_path = path.join("SKILL.md");
        if !skill_path.exists() {
            continue;
        }

        let content = match std::fs::read_to_string(&skill_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if let Some((name, description)) = parse_skill_frontmatter(&content) {
            skills.push(SkillSummary {
                folder_name,
                name,
                description,
                path: skill_path,
            });
        }
    }

    // Sort by folder_name for deterministic ordering
    skills.sort_by(|a, b| a.folder_name.cmp(&b.folder_name));
    skills
}

/// Discover skills from both OY and (optionally) Claude skill directories.
/// - OY skills: `~/.oy-ai-agent/skills/<folder>/SKILL.md`
/// - Claude skills: `~/.claude/skills/<folder>/SKILL.md` (only when `read_claude` is true)
pub fn discover_skills(read_claude: bool) -> Vec<SkillSummary> {
    let mut skills = Vec::new();

    // Always load OY skills
    if let Some(home) = dirs::home_dir() {
        let oy_dir = home.join(".oy-ai-agent").join("skills");
        skills.extend(load_skills_from_dir(&oy_dir));

        // Optionally load Claude skills
        if read_claude {
            let claude_dir = home.join(".claude").join("skills");
            skills.extend(load_skills_from_dir(&claude_dir));
        }
    }

    // Unify and remove duplicates, ensuring a definite order and avoiding repetitions (retain OY skills first).
    skills.sort_by(|a, b| a.folder_name.cmp(&b.folder_name));
    skills.dedup_by(|a, b| a.folder_name == b.folder_name);

    skills
}

/// Build a system prompt fragment listing available skills.
/// Format:
/// ```ignore
/// ## Available Skills
/// - folder_name: <folder>
///   name: <name>
///   description: <description>
///   path: <path>
/// ```
pub fn skills_to_prompt_fragment(skills: &[SkillSummary]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let mut result = String::from("\n## Available Skills\n");
    result.push_str(
        "Skills can assist you in your work. Corresponding skills are automatically loaded and applied according to the hints provided in the skill description. Unrelated skills do not need to be loaded..\nYou may use any of the following skills by reading their file with the `Read` tool:\n",
    );

    for skill in skills {
        result.push_str(&format!(
            "- folder_name: {}\n  skill_name: {}\n  description: {}\n  path: {}\n",
            skill.folder_name,
            skill.name,
            skill.description,
            skill.path.display(),
        ));
    }

    result.push_str("\nTo use a skill, read its file with `Read` (file_path: \"<path>\") and follow its instructions.\n");

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_temp_skill_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("Failed to create temp dir")
    }

    fn create_skill(base: &Path, folder: &str, content: &str) {
        let dir = base.join(folder);
        fs::create_dir_all(&dir).expect("Failed to create skill dir");
        fs::write(dir.join("SKILL.md"), content).expect("Failed to write SKILL.md");
    }

    #[test]
    fn test_parse_valid_frontmatter() {
        let content = r#"---
name: test-skill
description: A test skill description
---
Content goes here"#;
        let result = parse_skill_frontmatter(content);
        assert!(result.is_some());
        let (name, desc) = result.unwrap();
        assert_eq!(name, "test-skill");
        assert_eq!(desc, "A test skill description");
    }

    #[test]
    fn test_parse_no_frontmatter() {
        let content = "Just a plain markdown file without frontmatter.";
        assert!(parse_skill_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_missing_name() {
        let content = r#"---
description: Only description
---
Content"#;
        assert!(parse_skill_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_missing_description() {
        let content = r#"---
name: only-name
---
Content"#;
        assert!(parse_skill_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_empty_file() {
        assert!(parse_skill_frontmatter("").is_none());
        assert!(parse_skill_frontmatter("   ").is_none());
    }

    #[test]
    fn test_parse_frontmatter_with_extra_fields() {
        let content = r#"---
name: my-skill
description: My skill description
license: MIT
version: 1.0
---
Body"#;
        let result = parse_skill_frontmatter(content);
        assert!(result.is_some());
        let (name, desc) = result.unwrap();
        assert_eq!(name, "my-skill");
        assert_eq!(desc, "My skill description");
    }

    #[test]
    fn test_load_skills_from_directory() {
        let tmp = create_temp_skill_dir();
        create_skill(
            tmp.path(),
            "skill-one",
            r#"---
name: Skill One
description: First test skill
---
Content"#,
        );
        create_skill(
            tmp.path(),
            "skill-two",
            r#"---
name: Skill Two
description: Second test skill
---
Content"#,
        );

        let skills = load_skills_from_dir(tmp.path());
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].folder_name, "skill-one");
        assert_eq!(skills[0].name, "Skill One");
        assert_eq!(skills[1].folder_name, "skill-two");
        assert_eq!(skills[1].name, "Skill Two");
    }

    #[test]
    fn test_load_skills_skips_non_skill_dirs() {
        let tmp = create_temp_skill_dir();

        // Valid skill
        create_skill(
            tmp.path(),
            "valid-skill",
            r#"---
name: Valid
description: A valid skill
---
"#,
        );

        // Dir without SKILL.md should be skipped
        let empty_dir = tmp.path().join("empty-dir");
        fs::create_dir_all(&empty_dir).expect("Failed to create empty dir");

        // File (not dir) should be skipped
        fs::write(tmp.path().join("not-a-dir.md"), "content").expect("Failed to write file");

        let skills = load_skills_from_dir(tmp.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].folder_name, "valid-skill");
    }

    #[test]
    fn test_load_skills_no_frontmatter_skipped() {
        let tmp = create_temp_skill_dir();
        create_skill(tmp.path(), "no-frontmatter", "Just plain markdown");

        let skills = load_skills_from_dir(tmp.path());
        assert!(skills.is_empty());
    }

    #[test]
    fn test_discover_skills_oy_only() {
        let tmp = create_temp_skill_dir();
        create_skill(
            tmp.path(),
            "oy-skill",
            r#"---
name: OY Skill
description: From OY
---
"#,
        );

        // Temporarily override home dir for testing — we test via load_skills_from_dir directly
        // since discover_skills uses dirs::home_dir()
        let skills = load_skills_from_dir(tmp.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].folder_name, "oy-skill");
    }

    #[test]
    fn test_skills_to_prompt_fragment_empty() {
        let fragment = skills_to_prompt_fragment(&[]);
        assert!(fragment.is_empty());
    }

    #[test]
    fn test_skills_to_prompt_fragment_with_skills() {
        let skills = vec![SkillSummary {
            folder_name: "test-folder".to_string(),
            name: "Test Skill".to_string(),
            description: "A test".to_string(),
            path: PathBuf::from("/tmp/test-folder/SKILL.md"),
        }];

        let fragment = skills_to_prompt_fragment(&skills);
        assert!(fragment.contains("## Available Skills"));
        assert!(fragment.contains("test-folder"));
        assert!(fragment.contains("Test Skill"));
        assert!(fragment.contains("A test"));
        assert!(fragment.contains("/tmp/test-folder/SKILL.md"));
    }

    #[test]
    fn test_skills_to_prompt_fragment_multiple() {
        let skills = vec![
            SkillSummary {
                folder_name: "alpha".to_string(),
                name: "Alpha".to_string(),
                description: "First skill".to_string(),
                path: PathBuf::from("/tmp/alpha/SKILL.md"),
            },
            SkillSummary {
                folder_name: "beta".to_string(),
                name: "Beta".to_string(),
                description: "Second skill".to_string(),
                path: PathBuf::from("/tmp/beta/SKILL.md"),
            },
        ];

        let fragment = skills_to_prompt_fragment(&skills);
        assert!(fragment.contains("alpha"));
        assert!(fragment.contains("beta"));
        assert!(fragment.contains("First skill"));
        assert!(fragment.contains("Second skill"));
    }

    #[test]
    fn test_parse_frontmatter_handles_windows_line_endings() {
        let content = "---\r\nname: win-skill\r\ndescription: Windows style\r\n---\r\nBody";
        // Our parser uses lines() which handles \r\n correctly in Rust
        let result = parse_skill_frontmatter(content);
        assert!(result.is_some());
        let (name, desc) = result.unwrap();
        assert_eq!(name, "win-skill");
        assert_eq!(desc, "Windows style");
    }

    #[test]
    fn test_folder_name_as_unique_identifier() {
        let tmp = create_temp_skill_dir();

        // Two skills with same YAML name but different folders
        create_skill(
            tmp.path(),
            "folder-a",
            r#"---
name: duplicate-name
description: First occurrence
---
"#,
        );
        create_skill(
            tmp.path(),
            "folder-b",
            r#"---
name: duplicate-name
description: Second occurrence
---
"#,
        );

        let skills = load_skills_from_dir(tmp.path());
        assert_eq!(skills.len(), 2);
        // Both have same name but different folder_name
        assert_eq!(skills[0].name, "duplicate-name");
        assert_eq!(skills[1].name, "duplicate-name");
        assert_ne!(skills[0].folder_name, skills[1].folder_name);
    }
}
