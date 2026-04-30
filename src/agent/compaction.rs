use crate::protocol::types::LlmMessage;

/// Transcript compaction manages context window pressure by truncating older
/// portions of the conversation while keeping recent messages exact.
///
/// Full transcript is always preserved on disk; compaction only affects what
/// the LLM provider sees at request time.
///
/// Strategy: keep the last `KEEP_RECENT` messages in full, and replace older
/// messages with a brief summary placeholder noting that earlier conversation
/// exists in the session transcript.

/// Number of most-recent messages to keep in full.
const KEEP_RECENT: usize = 30;

/// Approximate token threshold before compaction is applied.
/// 64k tokens is roughly half of a 128k context window, leaving room for
/// system prompt, tool definitions, and the response.
const TOKEN_THRESHOLD: usize = 64_000;

/// Apply compaction to the LLM-form transcript if needed.
///
/// Returns a new `Vec<LlmMessage>` suitable for the provider request,
/// potentially replacing older messages with placeholder summaries.
/// The full transcript remains unchanged on disk.
pub fn compact(messages: &[LlmMessage]) -> Vec<LlmMessage> {
    if messages.len() <= KEEP_RECENT {
        return messages.to_vec();
    }

    let token_estimate = estimate_tokens(messages);
    if token_estimate <= TOKEN_THRESHOLD {
        return messages.to_vec();
    }

    // Split: older portion → summary placeholder, recent portion → exact
    let split_point = messages.len().saturating_sub(KEEP_RECENT);
    let older = &messages[..split_point];
    let recent = &messages[split_point..];

    // Approximate token count for the older portion
    let older_tokens = estimate_tokens(older);
    let older_msg_count = older.len();

    // Build a summary placeholder line
    let summary_text = format!(
        "[Earlier conversation: {} messages, ~{} tokens. This portion has been compacted \
         to reduce context size. The full transcript is available on disk. \
         Key context from the compacted section: the conversation continues below.]",
        older_msg_count, older_tokens
    );

    let summary = LlmMessage {
        role: "user".to_string(),
        content: vec![crate::protocol::types::LlmContentBlock::Text {
            text: summary_text,
        }],
    };

    let mut compacted = vec![summary];
    compacted.extend_from_slice(recent);
    compacted
}

/// Rough token estimate: ~4 chars per token for English text.
/// This is a heuristic; exact tokenization depends on the model.
fn estimate_tokens(messages: &[LlmMessage]) -> usize {
    let mut chars = 0usize;
    for msg in messages {
        for block in &msg.content {
            match block {
                crate::protocol::types::LlmContentBlock::Text { text }
                | crate::protocol::types::LlmContentBlock::Thinking { text } => {
                    chars += text.len();
                }
                crate::protocol::types::LlmContentBlock::ToolResult { content, .. } => {
                    chars += content.len();
                }
                crate::protocol::types::LlmContentBlock::ToolCall {
                    arguments, ..
                } => {
                    chars += arguments.to_string().len();
                }
            }
        }
    }
    // ~4 chars per token is a common heuristic for English
    chars / 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::types::LlmContentBlock;

    fn text_msg(role: &str, text: &str) -> LlmMessage {
        LlmMessage {
            role: role.to_string(),
            content: vec![LlmContentBlock::Text {
                text: text.to_string(),
            }],
        }
    }

    #[test]
    fn test_no_compaction_for_small_transcript() {
        let msgs: Vec<LlmMessage> = (0..5)
            .map(|i| text_msg(if i % 2 == 0 { "user" } else { "assistant" }, "short"))
            .collect();
        let result = compact(&msgs);
        assert_eq!(result.len(), 5);
    }

    #[test]
    fn test_compaction_for_large_transcript() {
        // Create enough data to exceed the 64k token threshold.
        // 100 messages × ~2000 chars each = ~200k chars ≈ 50k tokens
        // Already close; 200 messages pushes well past threshold.
        let mut msgs = Vec::new();
        for i in 0..200 {
            let long_text = format!("Message number {i}: {}", "x".repeat(2000));
            msgs.push(text_msg(if i % 2 == 0 { "user" } else { "assistant" }, &long_text));
        }

        let result = compact(&msgs);

        assert!(!result.is_empty());
        // First message should be a summary (user role with compaction note)
        assert_eq!(result[0].role, "user");

        if let LlmContentBlock::Text { text } = &result[0].content[0] {
            assert!(text.contains("compacted"), "Expected compaction note, got: {text}");
        } else {
            panic!("Expected text content block");
        }

        // Should have summary + KEEP_RECENT messages
        assert!(result.len() <= KEEP_RECENT + 1);
    }

    #[test]
    fn test_compaction_preserves_recent_messages() {
        let mut msgs = Vec::new();
        for i in 0..100 {
            let long_text = format!("Unique message {i}: {}", "x".repeat(2000));
            msgs.push(text_msg("user", &long_text));
        }

        let result = compact(&msgs);

        // The last message of result should match the last message of msgs
        assert!(!result.is_empty());
        let last_original = &msgs[msgs.len() - 1];
        let last_compacted = &result[result.len() - 1];

        if let (LlmContentBlock::Text { text: orig }, LlmContentBlock::Text { text: comp }) =
            (&last_original.content[0], &last_compacted.content[0])
        {
            assert_eq!(orig, comp, "Last message should be preserved exactly");
        } else {
            panic!("Expected text content blocks");
        }
    }
}
