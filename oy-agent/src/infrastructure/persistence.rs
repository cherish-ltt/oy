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

    let uuid = Uuid::from_str(&path.file_name().unwrap().to_string_lossy())?;
    let mut agent = Box::new(MainAgent::new_with_max_iterations(None));
    for msg in messages {
        let _ = agent.push_message_back(uuid, msg);
    }

    Ok(agent)
}
