use super::*;

#[test]
fn test_build_system_prompt_includes_cwd() {
    let ctx = PromptContext {
        cwd: "/home/user/project".into(),
        date: "2026-04-28".into(),
        tools: vec![ToolDefinition {
            name: "read".into(),
            description: "Read a file".into(),
            input_schema: serde_json::json!({}),
        }],
    };
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("/home/user/project"));
    assert!(prompt.contains("2026-04-28"));
    assert!(prompt.contains("read"));
    assert!(prompt.contains("- `read`"));
}

#[test]
fn test_build_system_prompt_no_tools() {
    let ctx = PromptContext {
        cwd: "/tmp".into(),
        date: "2026-04-28".into(),
        tools: vec![],
    };
    let prompt = build_system_prompt(&ctx);
    assert!(!prompt.contains("## Available tools"));
}
