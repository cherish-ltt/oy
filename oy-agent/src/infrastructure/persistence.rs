use oy_ai::ChatMessage;
use uuid::Uuid;

use crate::domain::agent::Agent;
use crate::domain::errors::AgentError;
use crate::infrastructure::agents::main_agent::MainAgent;

pub fn save_session(agent: &mut Box<dyn Agent>, project_dir: &str) -> Result<String, AgentError> {
    let home = dirs::home_dir()
        .ok_or_else(|| AgentError::SessionPersistenceError("Cannot find home directory".into()))?;
    let session_dir = home.join(".oy-ai-agent").join("sessions").join(project_dir);

    std::fs::create_dir_all(&session_dir).map_err(|e| {
        AgentError::SessionPersistenceError(format!("Cannot create session dir: {}", e))
    })?;

    // UUID v7 是按时间排序的（可以按创建时间进行排序），而 v4 则是随机的。
    // 这使得会话列表能够自然排序，并简化调试工作。
    let uuid = Uuid::now_v7();
    let file_path = session_dir.join(format!("{}.json", uuid));

    let messages: Vec<&ChatMessage> = agent.messages().iter().collect();
    let json = serde_json::to_string_pretty(&messages)
        .map_err(|e| AgentError::SessionPersistenceError(format!("Serialization error: {}", e)))?;

    std::fs::write(&file_path, &json)
        .map_err(|e| AgentError::SessionPersistenceError(format!("Write error: {}", e)))?;

    Ok(file_path.to_string_lossy().to_string())
}

pub fn load_session(path: &str) -> Result<Box<dyn Agent>, AgentError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| AgentError::SessionPersistenceError(format!("Read error: {}", e)))?;

    let messages: Vec<ChatMessage> = serde_json::from_str(&content).map_err(|e| {
        AgentError::SessionPersistenceError(format!("Deserialization error: {}", e))
    })?;

    let mut agent = Box::new(MainAgent::new_with_max_iterations(None));
    for msg in messages {
        agent.push_message_back(msg);
    }

    Ok(agent)
}
