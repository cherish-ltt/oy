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
pub fn find_latest_session() -> Result<Option<SessionEntry>, AgentError> {
    let mut all = list_all_sessions()?;
    if all.is_empty() {
        return Ok(None);
    }
    // UUID v7 is time-sorted: newest uuid = highest timestamp bits.
    // But we trust the file-system mtime as the definitive "last used" order.
    // Use mtime descending — the most recently modified file is latest.
    all.sort_by(|a, b| {
        let m_a = std::fs::metadata(&a.path)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let m_b = std::fs::metadata(&b.path)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        m_b.cmp(&m_a)
    });
    Ok(Some(all.remove(0)))
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

    // Sort by mtime descending (newest first)
    entries.sort_by(|a, b| {
        let m_a = std::fs::metadata(&a.path)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let m_b = std::fs::metadata(&b.path)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        m_b.cmp(&m_a)
    });

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
}
