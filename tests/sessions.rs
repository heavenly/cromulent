use cromulent::protocol::types::{ContentBlock, Message, MessageRole, ModelInfo, ThinkingLevel};
use cromulent::session::store::{SessionHeader, SessionStore};

fn test_model() -> ModelInfo {
    ModelInfo {
        provider: "openai".into(),
        id: "gpt-5.5".into(),
        display_name: "GPT-5.5".into(),
        context_window: 128_000,
        supports_reasoning: false,
        supports_tools: true,
    }
}

fn sample_header(session_id: &str, cwd: &str) -> SessionHeader {
    SessionHeader::new(
        session_id.into(),
        cwd.into(),
        test_model(),
        ThinkingLevel::Medium,
    )
}

fn sample_message(id: &str, role: MessageRole, text: &str) -> Message {
    Message {
        id: id.into(),
        timestamp: "2026-04-28T06:00:00Z".into(),
        role,
        content: vec![ContentBlock::Text { text: text.into() }],
        tool_call_id: None,
        tool_name: None,
        is_error: None,
    }
}

// -----------------------------------------------------------------------
// Session header tests (synchronous, no store needed)
// -----------------------------------------------------------------------

#[test]
fn test_session_header_creation() {
    let header = sample_header("ses_abc", "/home/user/proj");
    assert_eq!(header.session_id, "ses_abc");
    assert_eq!(header.type_field, "session_header");
    assert_eq!(header.cwd, "/home/user/proj");
    assert_eq!(header.schema_version, 1);
    assert_eq!(header.model.id, "gpt-5.5");
    assert_eq!(header.thinking_level, ThinkingLevel::Medium);
    assert!(!header.created.is_empty());
    assert!(!header.updated.is_empty());
}

#[test]
fn test_session_header_custom_provider() {
    let header = SessionHeader::new(
        "ses_custom".into(),
        "/tmp".into(),
        ModelInfo {
            provider: "deepseek".into(),
            id: "deepseek-coder".into(),
            display_name: String::new(),
            context_window: 128_000,
            supports_reasoning: false,
            supports_tools: true,
        },
        ThinkingLevel::High,
    );
    assert_eq!(header.model.provider, "deepseek");
    assert_eq!(header.thinking_level, ThinkingLevel::High);
    assert_eq!(header.cwd, "/tmp");
}

// -----------------------------------------------------------------------
// Session store create / load
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_session_store_create_and_load() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    store.ensure_dir().await.unwrap();

    let header = sample_header("ses_create_test", "/home/user");
    store.create_session(&header).await.unwrap();

    let loaded = store.load_session("ses_create_test").await.unwrap();
    assert_eq!(loaded.header.session_id, "ses_create_test");
    assert_eq!(loaded.header.cwd, "/home/user");
    assert_eq!(loaded.header.model.id, "gpt-5.5");
    assert_eq!(loaded.messages.len(), 0);
}

#[tokio::test]
async fn test_session_store_load_with_messages() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    store.ensure_dir().await.unwrap();

    let header = sample_header("ses_msg_test", "/app");
    store.create_session(&header).await.unwrap();

    let msg1 = sample_message("msg_1", MessageRole::User, "Hello");
    let msg2 = sample_message("msg_2", MessageRole::Assistant, "Hi there");

    store.append_message("ses_msg_test", &msg1).await.unwrap();
    store.append_message("ses_msg_test", &msg2).await.unwrap();

    let loaded = store.load_session("ses_msg_test").await.unwrap();
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[0].id, "msg_1");
    assert_eq!(loaded.messages[1].id, "msg_2");
}

#[tokio::test]
async fn test_session_store_load_nonexistent() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let result = store.load_session("nonexistent").await;
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// Session header update
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_session_store_update_header() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    store.ensure_dir().await.unwrap();

    let header = sample_header("ses_update_test", "/initial");
    store.create_session(&header).await.unwrap();

    let mut updated_header = header.clone();
    updated_header.cwd = "/updated".into();
    updated_header.model.id = "gpt-5".into();
    store.update_header(&updated_header).await.unwrap();

    let loaded = store.load_session("ses_update_test").await.unwrap();
    assert_eq!(loaded.header.cwd, "/updated");
    assert_eq!(loaded.header.model.id, "gpt-5");
}

#[tokio::test]
async fn test_session_store_update_header_preserves_messages() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    store.ensure_dir().await.unwrap();

    let header = sample_header("ses_preserve_test", "/proj");
    store.create_session(&header).await.unwrap();

    let msg = sample_message("msg_keep", MessageRole::User, "keep me");
    store
        .append_message("ses_preserve_test", &msg)
        .await
        .unwrap();

    let mut updated = header;
    updated.cwd = "/newproj".into();
    store.update_header(&updated).await.unwrap();

    let loaded = store.load_session("ses_preserve_test").await.unwrap();
    assert_eq!(loaded.messages.len(), 1);
    assert_eq!(loaded.messages[0].id, "msg_keep");
}

// -----------------------------------------------------------------------
// Session listing
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_session_store_create_and_load_after_ensure_dir() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    store.ensure_dir().await.unwrap();

    let header = sample_header("ses_empty", "/test");
    store.create_session(&header).await.unwrap();
    let loaded = store.load_session("ses_empty").await.unwrap();
    assert_eq!(loaded.header.session_id, "ses_empty");
}

// -----------------------------------------------------------------------
// Session fork
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_session_store_fork() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    store.ensure_dir().await.unwrap();

    let header = sample_header("ses_source", "/src");
    store.create_session(&header).await.unwrap();

    let msgs = vec![
        sample_message("msg_0", MessageRole::User, "First"),
        sample_message("msg_1", MessageRole::Assistant, "Response"),
        sample_message("msg_2", MessageRole::User, "Follow-up"),
    ];
    for msg in &msgs {
        store.append_message("ses_source", msg).await.unwrap();
    }

    let fork_header = SessionHeader::new(
        "ses_fork".into(),
        "/fork".into(),
        test_model(),
        ThinkingLevel::Medium,
    );
    let forked = store
        .fork_session("ses_source", "msg_1", &fork_header)
        .await
        .unwrap();

    assert_eq!(forked.header.session_id, "ses_fork");
    assert_eq!(forked.messages.len(), 2);
    assert_eq!(forked.messages[0].id, "msg_0");
    assert_eq!(forked.messages[1].id, "msg_1");

    // Verify forked session persisted
    let loaded = store.load_session("ses_fork").await.unwrap();
    assert_eq!(loaded.messages.len(), 2);
}

#[tokio::test]
async fn test_session_store_fork_nonexistent_entry() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    store.ensure_dir().await.unwrap();

    let header = sample_header("ses_fail_source", "/src");
    store.create_session(&header).await.unwrap();

    let result = store
        .fork_session(
            "ses_fail_source",
            "nonexistent_msg",
            &sample_header("ses_fail_fork", "/dst"),
        )
        .await;
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// Session file path
// -----------------------------------------------------------------------

#[test]
fn test_session_path() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());

    let path = store.session_path("ses_xyz");
    assert_eq!(path.file_name().unwrap(), "ses_xyz.jsonl");
    assert!(path.starts_with(dir.path()));
}

// -----------------------------------------------------------------------
// Schema version validation
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_load_session_invalid_schema_version() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    store.ensure_dir().await.unwrap();

    let header = sample_header("ses_bad_schema", "/tmp");
    let mut header_json = serde_json::to_string(&header).unwrap();
    header_json = header_json.replace(r#""schemaVersion":1"#, r#""schemaVersion":99"#);
    let path = store.session_path("ses_bad_schema");
    tokio::fs::write(&path, format!("{header_json}\n"))
        .await
        .unwrap();

    let result = store.load_session("ses_bad_schema").await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("99"));
}
