use std::path::{Path, PathBuf};
use std::str::FromStr;

use oy_ai::ChatMessage;
use uuid::Uuid;

use crate::domain::agent::Agent;
use crate::domain::errors::AgentError;
use crate::infrastructure::agents::main_agent::MainAgent;

/// A summary entry for a saved session.
#[derive(Debug, Clone)]
pub struct SessionEntry {
    pub path: PathBuf,
    pub uuid: Uuid,
    pub project_name: String,
}

/// Find the most recent session across all project directories.
///
/// Relies on `list_all_sessions()` which already returns results sorted
/// by modification time (newest first).
pub fn find_latest_session() -> Result<Option<SessionEntry>, AgentError> {
    let all = list_all_sessions()?;
    Ok(all.into_iter().next())
}

/// List all session files across all project directories,
/// sorted by modification time (newest first).
pub fn list_all_sessions() -> Result<Vec<SessionEntry>, AgentError> {
    let home = dirs::home_dir()
        .ok_or_else(|| AgentError::SessionPersistenceError("Cannot find home directory".into()))?;
    let sessions_root = home.join(".oy-ai-agent").join("sessions");

    if !sessions_root.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    let read_dir = std::fs::read_dir(&sessions_root).map_err(|e| {
        AgentError::SessionPersistenceError(format!("Cannot read sessions dir: {}", e))
    })?;

    for project_dir in read_dir {
        let project_dir = project_dir.map_err(|e| {
            AgentError::SessionPersistenceError(format!("Cannot read entry: {}", e))
        })?;
        let project_path = project_dir.path();
        if !project_path.is_dir() {
            continue;
        }
        let project_name = project_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let session_dir = std::fs::read_dir(&project_path).map_err(|e| {
            AgentError::SessionPersistenceError(format!("Cannot read session dir: {}", e))
        })?;

        for session_file in session_dir {
            let session_file = session_file.map_err(|e| {
                AgentError::SessionPersistenceError(format!("Cannot read entry: {}", e))
            })?;
            let path = session_file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let file_stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if let Ok(uuid) = Uuid::from_str(file_stem) {
                entries.push(SessionEntry {
                    path,
                    uuid,
                    project_name: project_name.clone(),
                });
            }
        }
    }

    // Sort by mtime descending (newest first) — cache metadata to avoid O(N log N) disk I/O
    let mut entries_with_mtime: Vec<(SessionEntry, std::time::SystemTime)> = entries
        .into_iter()
        .map(|entry| {
            let mtime = std::fs::metadata(&entry.path)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            (entry, mtime)
        })
        .collect();

    entries_with_mtime.sort_by(|a, b| b.1.cmp(&a.1));
    let entries: Vec<SessionEntry> = entries_with_mtime.into_iter().map(|(e, _)| e).collect();

    Ok(entries)
}

/// Extract the first user message from a session file for preview purposes.
/// Returns `None` if no user message is found.
///
/// The preview is flattened to a single line (newlines → spaces, whitespace collapsed)
/// and truncated to ~60 characters.
pub fn get_session_preview(path: &Path) -> Result<Option<String>, AgentError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| AgentError::SessionPersistenceError(format!("Read error: {}", e)))?;
    let messages: Vec<ChatMessage> = serde_json::from_str(&content).map_err(|e| {
        AgentError::SessionPersistenceError(format!("Deserialization error: {}", e))
    })?;
    Ok(messages
        .into_iter()
        .find(|m| m.role == oy_ai::Role::User)
        .and_then(|m| m.content)
        .map(|c| {
            // Flatten to single line: collapse all whitespace (newlines, tabs, spaces)
            let joined: Vec<&str> = c.split_ascii_whitespace().collect();
            let flat = joined.join(" ");
            // Truncate to ~60 visible chars
            let truncated: String = flat.chars().take(60).collect();
            if flat.chars().count() > 60 {
                format!("{}...", truncated.trim_end())
            } else {
                truncated
            }
        }))
}

/// Load raw messages from a session file (without creating an Agent).
pub fn load_session_messages(path: &Path) -> Result<(Uuid, Vec<ChatMessage>), AgentError> {
    if !path.is_file() {
        return Err(AgentError::PathIsNotFile(
            path.to_string_lossy().to_string(),
        ));
    }

    let content = std::fs::read_to_string(path)
        .map_err(|e| AgentError::SessionPersistenceError(format!("Read error: {}", e)))?;

    let messages: Vec<ChatMessage> = serde_json::from_str(&content).map_err(|e| {
        AgentError::SessionPersistenceError(format!("Deserialization error: {}", e))
    })?;

    let file_name = path
        .file_stem()
        .ok_or_else(|| AgentError::SessionPersistenceError("Invalid file name".into()))?;
    let uuid = Uuid::from_str(&file_name.to_string_lossy())?;

    Ok((uuid, messages))
}

pub fn save_session(
    uuid: Uuid,
    messages: Vec<&ChatMessage>,
    target_dir: &str,
) -> Result<String, AgentError> {
    let home = dirs::home_dir()
        .ok_or_else(|| AgentError::SessionPersistenceError("Cannot find home directory".into()))?;
    let session_dir = home.join(".oy-ai-agent").join("sessions").join(target_dir);

    std::fs::create_dir_all(&session_dir).map_err(|e| {
        AgentError::SessionPersistenceError(format!("Cannot create session dir: {}", e))
    })?;

    let file_path = session_dir.join(format!("{}.json", uuid));

    let json = serde_json::to_string_pretty(&messages)
        .map_err(|e| AgentError::SessionPersistenceError(format!("Serialization error: {}", e)))?;

    std::fs::write(&file_path, &json)
        .map_err(|e| AgentError::SessionPersistenceError(format!("Write error: {}", e)))?;

    Ok(file_path.to_string_lossy().to_string())
}

pub fn load_session(path: &Path) -> Result<Box<dyn Agent>, AgentError> {
    if !path.is_file() {
        return Err(AgentError::PathIsNotFile(
            path.to_string_lossy().to_string(),
        ));
    }

    let content = std::fs::read_to_string(path)
        .map_err(|e| AgentError::SessionPersistenceError(format!("Read error: {}", e)))?;

    let messages: Vec<ChatMessage> = serde_json::from_str(&content).map_err(|e| {
        AgentError::SessionPersistenceError(format!("Deserialization error: {}", e))
    })?;

    let file_name = path
        .file_stem()
        .ok_or_else(|| AgentError::SessionPersistenceError("Invalid file name".into()))?;
    let uuid = Uuid::from_str(&file_name.to_string_lossy())?;
    let mut agent = Box::new(MainAgent::new(None));
    for msg in messages {
        let _ = agent.push_message_back(uuid, msg);
    }

    Ok(agent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> String {
        format!("_oy_test_{}", name)
    }

    #[test]
    fn test_save_session_returns_path() {
        let dir = test_dir("returns_path");
        let uuid = Uuid::now_v7();
        let msgs: Vec<&ChatMessage> = vec![];
        let result = save_session(uuid, msgs, &dir);
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.contains(&uuid.to_string()));
        assert!(path.ends_with(".json"));
        if let Some(p) = Path::new(&path).parent() {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = test_dir("roundtrip");
        let uuid = Uuid::now_v7();
        let msg = ChatMessage::user("hello from test");
        let msgs: Vec<&ChatMessage> = vec![&msg];
        let save_result = save_session(uuid, msgs, &dir);
        assert!(save_result.is_ok());
        let path_str = save_result.unwrap();
        let path = Path::new(&path_str);
        assert!(path.exists());

        let loaded = load_session(path);
        assert!(loaded.is_ok());

        if let Some(p) = path.parent() {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    #[test]
    fn test_load_session_nonexistent_path() {
        let path = Path::new("/tmp/nonexistent_oy_session.json");
        let result = load_session(path);
        assert!(matches!(result, Err(AgentError::PathIsNotFile(_))));
    }

    #[test]
    fn test_save_session_invalid_dir() {
        let dir = test_dir("invalid");
        let uuid = Uuid::now_v7();
        let msgs = vec![];
        let result = save_session(uuid, msgs, &dir);
        assert!(result.is_ok());
        if let Ok(p) = &result {
            if let Some(parent) = Path::new(p).parent() {
                let _ = std::fs::remove_dir_all(parent);
            }
        }
    }

    // ── get_session_preview tests ──────────────────────────────

    #[test]
    fn test_get_session_preview_finds_first_user() {
        let dir = test_dir("preview_first_user");
        let uuid = Uuid::now_v7();
        let msgs = vec![
            ChatMessage::system("system prompt"),
            ChatMessage::user("hello world"),
            ChatMessage::assistant(Some("response".into()), None, None),
        ];
        let refs: Vec<&ChatMessage> = msgs.iter().collect();
        let path = save_session(uuid, refs, &dir).unwrap();
        let preview = get_session_preview(Path::new(&path)).unwrap();
        assert_eq!(preview, Some("hello world".to_string()));
        if let Some(p) = Path::new(&path).parent() {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    #[test]
    fn test_get_session_preview_skips_system_and_picks_first_user() {
        let dir = test_dir("preview_skip_system");
        let uuid = Uuid::now_v7();
        // Multiple system messages before the first user message
        let msgs = vec![
            ChatMessage::system("system 1"),
            ChatMessage::system("system 2"),
            ChatMessage::user("actual prompt"),
        ];
        let refs: Vec<&ChatMessage> = msgs.iter().collect();
        let path = save_session(uuid, refs, &dir).unwrap();
        let preview = get_session_preview(Path::new(&path)).unwrap();
        assert_eq!(preview, Some("actual prompt".to_string()));
        if let Some(p) = Path::new(&path).parent() {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    #[test]
    fn test_get_session_preview_flattens_newlines_and_tabs() {
        let dir = test_dir("preview_flatten");
        let uuid = Uuid::now_v7();
        let msg = ChatMessage::user("需求:\n添加一个oy -session命令\n\t用于加载session文件");
        let refs = vec![&msg];
        let path = save_session(uuid, refs, &dir).unwrap();
        let preview = get_session_preview(Path::new(&path)).unwrap();
        assert_eq!(
            preview,
            Some("需求: 添加一个oy -session命令 用于加载session文件".to_string())
        );
        if let Some(p) = Path::new(&path).parent() {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    #[test]
    fn test_get_session_preview_collapses_consecutive_whitespace() {
        let dir = test_dir("preview_collapse");
        let uuid = Uuid::now_v7();
        let msg = ChatMessage::user("hello    world\n\n\nmulti   \n  spaced");
        let refs = vec![&msg];
        let path = save_session(uuid, refs, &dir).unwrap();
        let preview = get_session_preview(Path::new(&path)).unwrap();
        assert_eq!(preview, Some("hello world multi spaced".to_string()));
        if let Some(p) = Path::new(&path).parent() {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    #[test]
    fn test_get_session_preview_truncates_long_message() {
        let dir = test_dir("preview_truncate");
        let uuid = Uuid::now_v7();
        let long_body = "a".repeat(100);
        let msg = ChatMessage::user(&long_body);
        let refs = vec![&msg];
        let path = save_session(uuid, refs, &dir).unwrap();
        let preview = get_session_preview(Path::new(&path)).unwrap();
        assert!(preview.is_some());
        let preview = preview.unwrap();
        // Should be 60 chars + "..." = 63
        assert_eq!(preview.chars().count(), 63, "expected 63 chars (60 + ...)");
        assert!(preview.ends_with("..."), "preview should end with ...");
        assert_eq!(
            &preview[..60],
            "a".repeat(60),
            "first 60 chars should be 'a'"
        );
        if let Some(p) = Path::new(&path).parent() {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    #[test]
    fn test_get_session_preview_exactly_60_chars_no_ellipsis() {
        let dir = test_dir("preview_exact_60");
        let uuid = Uuid::now_v7();
        let body = "b".repeat(60);
        let msg = ChatMessage::user(&body);
        let refs = vec![&msg];
        let path = save_session(uuid, refs, &dir).unwrap();
        let preview = get_session_preview(Path::new(&path)).unwrap();
        assert_eq!(preview, Some("b".repeat(60)));
        assert!(!preview.unwrap().ends_with("..."));
        if let Some(p) = Path::new(&path).parent() {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    #[test]
    fn test_get_session_preview_no_user_message_returns_none() {
        let dir = test_dir("preview_no_user");
        let uuid = Uuid::now_v7();
        let msgs = vec![
            ChatMessage::system("system"),
            ChatMessage::assistant(Some("response".into()), None, None),
            ChatMessage::tool("result", "call_1".into(), Some("Read".into()), None),
        ];
        let refs: Vec<&ChatMessage> = msgs.iter().collect();
        let path = save_session(uuid, refs, &dir).unwrap();
        let preview = get_session_preview(Path::new(&path)).unwrap();
        assert_eq!(preview, None);
        if let Some(p) = Path::new(&path).parent() {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    #[test]
    fn test_get_session_preview_only_has_tool_and_assistant() {
        let dir = test_dir("preview_only_tool");
        let uuid = Uuid::now_v7();
        let msgs = vec![
            ChatMessage::assistant(Some("thinking...".into()), None, None),
            ChatMessage::tool("output", "c1".into(), Some("Bash".into()), None),
        ];
        let refs: Vec<&ChatMessage> = msgs.iter().collect();
        let path = save_session(uuid, refs, &dir).unwrap();
        let preview = get_session_preview(Path::new(&path)).unwrap();
        assert_eq!(preview, None);
        if let Some(p) = Path::new(&path).parent() {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    #[test]
    fn test_get_session_preview_empty_user_message() {
        let dir = test_dir("preview_empty_user");
        let uuid = Uuid::now_v7();
        let msg = ChatMessage::user("");
        let refs = vec![&msg];
        let path = save_session(uuid, refs, &dir).unwrap();
        let preview = get_session_preview(Path::new(&path)).unwrap();
        assert_eq!(preview, Some(String::new()));
        if let Some(p) = Path::new(&path).parent() {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    // ── load_session_messages tests ────────────────────────────

    #[test]
    fn test_load_session_messages_roundtrip() {
        let dir = test_dir("load_msgs_roundtrip");
        let uuid = Uuid::now_v7();
        let msgs = vec![
            ChatMessage::system("system prompt"),
            ChatMessage::user("hello"),
            ChatMessage::assistant(Some("world".into()), None, None),
        ];
        let refs: Vec<&ChatMessage> = msgs.iter().collect();
        let path = save_session(uuid, refs, &dir).unwrap();
        let (loaded_uuid, loaded_msgs) = load_session_messages(Path::new(&path)).unwrap();
        assert_eq!(loaded_uuid, uuid);
        assert_eq!(loaded_msgs.len(), 3);
        assert_eq!(loaded_msgs[0].role, oy_ai::Role::System);
        assert_eq!(loaded_msgs[0].content.as_deref(), Some("system prompt"));
        assert_eq!(loaded_msgs[1].role, oy_ai::Role::User);
        assert_eq!(loaded_msgs[1].content.as_deref(), Some("hello"));
        assert_eq!(loaded_msgs[2].role, oy_ai::Role::Assistant);
        if let Some(p) = Path::new(&path).parent() {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    #[test]
    fn test_load_session_messages_preserves_tool_calls() {
        let dir = test_dir("load_msgs_tool_calls");
        let uuid = Uuid::now_v7();
        use oy_ai::ToolCall;
        let tool_call = ToolCall {
            id: "call_1".into(),
            function_name: "Read".into(),
            arguments: serde_json::json!({"file_path": "/tmp/x.txt"}),
        };
        let msgs = vec![
            ChatMessage::user("read a file"),
            ChatMessage::assistant(None, None, Some(vec![tool_call])),
            ChatMessage::tool("content", "call_1".into(), Some("Read".into()), None),
        ];
        let refs: Vec<&ChatMessage> = msgs.iter().collect();
        let path = save_session(uuid, refs, &dir).unwrap();
        let (_, loaded_msgs) = load_session_messages(Path::new(&path)).unwrap();
        assert_eq!(loaded_msgs.len(), 3);
        let assistant = &loaded_msgs[1];
        assert_eq!(assistant.role, oy_ai::Role::Assistant);
        assert!(assistant.tool_calls.is_some());
        assert_eq!(assistant.tool_calls.as_ref().unwrap().len(), 1);
        assert_eq!(
            assistant.tool_calls.as_ref().unwrap()[0].function_name,
            "Read"
        );
        let tool_result = &loaded_msgs[2];
        assert_eq!(tool_result.role, oy_ai::Role::Tool);
        assert_eq!(tool_result.tool_call_id.as_deref(), Some("call_1"));
        if let Some(p) = Path::new(&path).parent() {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    #[test]
    fn test_load_session_messages_nonexistent_path() {
        let path = Path::new("/tmp/_oy_test_nonexistent_session_file.json");
        let result = load_session_messages(path);
        assert!(matches!(result, Err(AgentError::PathIsNotFile(_))));
    }

    #[test]
    fn test_load_session_messages_invalid_json() {
        let dir = test_dir("load_msgs_invalid_json");
        let uuid = Uuid::now_v7();
        // Write invalid JSON to a session file path
        let home = dirs::home_dir().unwrap();
        let session_dir = home.join(".oy-ai-agent").join("sessions").join(&dir);
        std::fs::create_dir_all(&session_dir).unwrap();
        let file_path = session_dir.join(format!("{}.json", uuid));
        std::fs::write(&file_path, "not valid json {").unwrap();
        let result = load_session_messages(&file_path);
        assert!(matches!(
            result,
            Err(AgentError::SessionPersistenceError(_))
        ));
        let _ = std::fs::remove_dir_all(&session_dir);
    }

    #[test]
    fn test_load_session_messages_empty_array() {
        let dir = test_dir("load_msgs_empty");
        let uuid = Uuid::now_v7();
        let refs: Vec<&ChatMessage> = vec![];
        let path = save_session(uuid, refs, &dir).unwrap();
        let (loaded_uuid, loaded_msgs) = load_session_messages(Path::new(&path)).unwrap();
        assert_eq!(loaded_uuid, uuid);
        assert!(loaded_msgs.is_empty());
        if let Some(p) = Path::new(&path).parent() {
            let _ = std::fs::remove_dir_all(p);
        }
    }
}
