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
        content: vec![crate::protocol::types::LlmContentBlock::Text { text: summary_text }],
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
                crate::protocol::types::LlmContentBlock::ToolCall { arguments, .. } => {
                    chars += arguments.to_string().len();
                }
            }
        }
    }
    // ~4 chars per token is a common heuristic for English
    chars / 4
}

#[cfg(test)]
#[path = "../../tests/inline/agent_compaction_tests.rs"]
mod tests;
