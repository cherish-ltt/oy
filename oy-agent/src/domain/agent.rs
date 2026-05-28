use oy_ai::ChatMessage;

pub trait Agent: Send + Sync {
    fn push_message_back(&mut self, msg: ChatMessage);
    fn messages(&mut self) -> &[ChatMessage];
    fn max_iterations(&self) -> u32;
    fn clear_messages(&mut self);
    fn get_system_prompt(&self, tools_description: &str) -> String;
}
