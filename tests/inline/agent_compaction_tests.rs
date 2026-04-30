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
        msgs.push(text_msg(
            if i % 2 == 0 { "user" } else { "assistant" },
            &long_text,
        ));
    }

    let result = compact(&msgs);

    assert!(!result.is_empty());
    // First message should be a summary (user role with compaction note)
    assert_eq!(result[0].role, "user");

    if let LlmContentBlock::Text { text } = &result[0].content[0] {
        assert!(
            text.contains("compacted"),
            "Expected compaction note, got: {text}"
        );
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
