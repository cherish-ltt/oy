use oy_ai::ChatMessage;
use oy_ai::Role;
use std::sync::LazyLock;
use tiktoken_rs::CoreBPE;
use tiktoken_rs::cl100k_base_singleton;

/// Global singleton for the cl100k_base tokenizer (GPT-4, GPT-3.5-turbo, etc.)
static CL100K_BASE: LazyLock<CoreBPE> = LazyLock::new(|| cl100k_base_singleton().clone());

/// Token usage broken down by role side (input vs output).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsage {
    /// User-side tokens: System + User + Tool role messages
    pub input_tokens: u64,
    /// AI-side tokens: Assistant role messages (content + reasoning_content)
    pub output_tokens: u64,
    /// Total conversation tokens: input_tokens + output_tokens
    pub context_tokens: u64,
}

impl TokenUsage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add input tokens to the cumulative total.
    pub fn add_input(&mut self, tokens: u64) {
        self.input_tokens = self.input_tokens.saturating_add(tokens);
    }

    /// Add output tokens to the cumulative total.
    pub fn add_output(&mut self, tokens: u64) {
        self.output_tokens = self.output_tokens.saturating_add(tokens);
    }
}

/// Count the number of tokens in a text string using the cl100k_base encoding.
///
/// This is a simple raw count of BPE tokens in the text, without the per-message
/// overhead tokens that OpenAI's API adds. For exact API billing counts, use
/// `count_input_tokens_for_api` which accounts for the message framing tokens.
pub fn count_tokens(text: &str) -> u64 {
    if text.is_empty() {
        return 0;
    }
    CL100K_BASE.count_with_special_tokens(text) as u64
}

/// Count tokens in a single ChatMessage's content + reasoning_content.
pub fn count_message_tokens(msg: &ChatMessage) -> u64 {
    let mut total = 0u64;
    if let Some(ref content) = msg.content {
        total = total.saturating_add(count_tokens(content));
    }
    if let Some(ref reasoning) = msg.reasoning_content {
        total = total.saturating_add(count_tokens(reasoning));
    }
    total
}

/// Count the total tokens across all messages (input side).
///
/// This counts the raw text tokens in each message's content and reasoning_content.
pub fn count_input_tokens(messages: &[ChatMessage]) -> u64 {
    messages.iter().map(count_message_tokens).sum()
}

/// Count output tokens from a response message (content + reasoning_content).
pub fn count_output_tokens(msg: &ChatMessage) -> u64 {
    count_message_tokens(msg)
}

/// Count tokens in messages on the user/input side (System, User, Tool roles).
pub fn count_input_side_tokens(messages: &[ChatMessage]) -> u64 {
    messages
        .iter()
        .filter(|m| m.role != Role::Assistant)
        .map(count_message_tokens)
        .sum()
}

/// Count tokens in messages on the AI/output side (Assistant role).
pub fn count_output_side_tokens(messages: &[ChatMessage]) -> u64 {
    messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .map(count_message_tokens)
        .sum()
}

/// Format a token count for display (e.g., 3140 → "3.1k", 500 → "500")
pub fn format_token_count(count: u64) -> String {
    if count >= 1000 {
        let whole = count / 1000;
        let frac = (count % 1000) / 100;
        if frac > 0 {
            format!("{}.{}k", whole, frac)
        } else {
            format!("{}k", whole)
        }
    } else {
        count.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oy_ai::ChatMessage;

    #[test]
    fn test_count_tokens_empty() {
        assert_eq!(count_tokens(""), 0);
    }

    #[test]
    fn test_count_tokens_simple() {
        let count = count_tokens("Hello, world!");
        assert!(count > 0, "Should have at least 1 token");
    }

    #[test]
    fn test_count_tokens_english() {
        let count = count_tokens("The quick brown fox jumps over the lazy dog");
        assert!(count > 0);
    }

    #[test]
    fn test_count_message_tokens_content_only() {
        let msg = ChatMessage::user("Hello, world!");
        let count = count_message_tokens(&msg);
        assert!(count > 0);
    }

    #[test]
    fn test_count_message_tokens_with_reasoning() {
        let msg = ChatMessage::assistant(
            Some("Final answer".into()),
            Some("Let me think step by step...".into()),
            None,
        );
        let count = count_message_tokens(&msg);
        assert!(count > 0);
        // reasoning + content should have more tokens than just content
        let content_only = ChatMessage::assistant(Some("Final answer".into()), None, None);
        let count_content_only = count_message_tokens(&content_only);
        assert!(
            count >= count_content_only,
            "Message with reasoning should have >= tokens than without"
        );
    }

    #[test]
    fn test_count_message_tokens_empty() {
        let msg = ChatMessage::assistant(None, None, None);
        assert_eq!(count_message_tokens(&msg), 0);
    }

    #[test]
    fn test_count_input_tokens() {
        let messages = vec![
            ChatMessage::system("You are a helpful assistant."),
            ChatMessage::user("Hello!"),
            ChatMessage::assistant(Some("Hi there!".into()), None, None),
        ];
        let count = count_input_tokens(&messages);
        assert!(count > 0);
        // Each message with content should contribute, so count >= 3 messages' worth
        let single = count_message_tokens(&messages[0])
            + count_message_tokens(&messages[1])
            + count_message_tokens(&messages[2]);
        assert_eq!(count, single);
    }

    #[test]
    fn test_count_output_tokens() {
        let msg = ChatMessage::assistant(
            Some("Here is the answer".into()),
            Some("Thinking...".into()),
            None,
        );
        let output = count_output_tokens(&msg);
        let manual = count_message_tokens(&msg);
        assert_eq!(output, manual);
    }

    #[test]
    fn test_token_usage_default() {
        let usage = TokenUsage::default();
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
    }

    #[test]
    fn test_token_usage_add_input() {
        let mut usage = TokenUsage::default();
        usage.add_input(100);
        assert_eq!(usage.input_tokens, 100);
        usage.add_input(50);
        assert_eq!(usage.input_tokens, 150);
    }

    #[test]
    fn test_token_usage_add_output() {
        let mut usage = TokenUsage::default();
        usage.add_output(200);
        assert_eq!(usage.output_tokens, 200);
        usage.add_output(75);
        assert_eq!(usage.output_tokens, 275);
    }

    #[test]
    fn test_token_usage_saturation() {
        let mut usage = TokenUsage::default();
        usage.add_input(u64::MAX);
        usage.add_input(1);
        assert_eq!(usage.input_tokens, u64::MAX);
    }

    #[test]
    fn test_format_token_count_under_1k() {
        assert_eq!(format_token_count(0), "0");
        assert_eq!(format_token_count(500), "500");
        assert_eq!(format_token_count(999), "999");
    }

    #[test]
    fn test_format_token_count_exact_k() {
        assert_eq!(format_token_count(1000), "1k");
        assert_eq!(format_token_count(5000), "5k");
        assert_eq!(format_token_count(10000), "10k");
    }

    #[test]
    fn test_format_token_count_with_fraction() {
        assert_eq!(format_token_count(3100), "3.1k");
        assert_eq!(format_token_count(3150), "3.1k");
        assert_eq!(format_token_count(44900), "44.9k");
    }

    #[test]
    fn test_format_token_count_rounding() {
        // 3140 → whole=3, frac=1 → "3.1k"
        assert_eq!(format_token_count(3140), "3.1k");
        // 44000 → whole=44, frac=0 → "44k"
        assert_eq!(format_token_count(44000), "44k");
    }

    #[test]
    fn test_cl100k_base_singleton_consistent() {
        let count1 = count_tokens("Hello, world! Test");
        let count2 = count_tokens("Hello, world! Test");
        assert_eq!(count1, count2);
    }
}
