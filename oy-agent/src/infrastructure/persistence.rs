use std::path::Path;
use std::str::FromStr;

use oy_ai::ChatMessage;
use uuid::Uuid;

use crate::domain::agent::Agent;
use crate::domain::errors::AgentError;
use crate::infrastructure::agents::main_agent::MainAgent;

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
