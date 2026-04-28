use std::path::PathBuf;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use cromulent::protocol::types::{ContentBlock, ToolContext, ToolResult};
use cromulent::tools::ask_user::AskManagerHandle;
use cromulent::tools::registry::{Tool, ToolRegistry};
use cromulent::tools::{EditTool, FindTool, GrepTool, ReadTool, WriteTool};
use cromulent::transport::writer::OutputItem;

/// Helper: build a minimal ToolContext for testing.
fn test_ctx(cwd: PathBuf) -> (ToolContext, mpsc::UnboundedReceiver<OutputItem>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let ctx = ToolContext {
        cwd,
        run_id: "test_run".into(),
        event_tx: tx,
        ask_manager: AskManagerHandle::new(),
    };
    (ctx, rx)
}

/// Helper: run a tool synchronously in a tokio runtime.
fn run_tool(tool: &dyn cromulent::tools::registry::Tool, ctx: ToolContext, args: serde_json::Value) -> ToolResult {
    let cancel = CancellationToken::new();
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(tool.execute(ctx, args, cancel))
        .unwrap()
}

/// Helper: extract text from a ToolResult.
fn tool_text(result: &ToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// -----------------------------------------------------------------------
// ReadTool basics
// -----------------------------------------------------------------------

#[test]
fn test_read_tool_file_with_content() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("hello.txt");
    std::fs::write(&file_path, "Hello, World!\nLine 2\nLine 3").unwrap();

    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let args = serde_json::json!({ "path": "hello.txt" });
    let result = run_tool(&ReadTool, ctx, args);

    assert!(!result.is_error);
    assert!(tool_text(&result).contains("Hello, World!"));
}

#[test]
fn test_read_tool_with_offset_and_limit() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("lines.txt");
    std::fs::write(
        &file_path,
        "Line 1\nLine 2\nLine 3\nLine 4\nLine 5",
    )
    .unwrap();

    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let args = serde_json::json!({ "path": "lines.txt", "offset": 2, "limit": 3 });
    let result = run_tool(&ReadTool, ctx, args);

    assert!(!result.is_error);
    let text = tool_text(&result);
    assert!(text.contains("Line 2"));
    assert!(text.contains("Line 3"));
    assert!(text.contains("Line 4"));
}

#[test]
fn test_read_tool_nonexistent_file() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let cancel = CancellationToken::new();
    let args = serde_json::json!({ "path": "nonexistent.txt" });

    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(ReadTool.execute(ctx, args, cancel));
    assert!(result.is_err());
}

#[test]
fn test_read_tool_missing_path_arg() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let cancel = CancellationToken::new();
    let args = serde_json::json!({});

    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(ReadTool.execute(ctx, args, cancel));
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("path"));
}

// -----------------------------------------------------------------------
// WriteTool basics
// -----------------------------------------------------------------------

#[test]
fn test_write_tool_creates_file() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let args = serde_json::json!({
        "path": "newfile.txt",
        "content": "Hello from write tool!"
    });
    let result = run_tool(&WriteTool, ctx, args);

    assert!(!result.is_error);
    let content = std::fs::read_to_string(dir.path().join("newfile.txt")).unwrap();
    assert_eq!(content, "Hello from write tool!");
}

#[test]
fn test_write_tool_creates_parent_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let args = serde_json::json!({
        "path": "a/b/c/deep.txt",
        "content": "deeply nested"
    });
    let result = run_tool(&WriteTool, ctx, args);

    assert!(!result.is_error);
    assert!(dir.path().join("a/b/c/deep.txt").exists());
    let content = std::fs::read_to_string(dir.path().join("a/b/c/deep.txt")).unwrap();
    assert_eq!(content, "deeply nested");
}

#[test]
fn test_write_tool_overwrites_existing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("existing.txt"), "old content").unwrap();

    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let args = serde_json::json!({
        "path": "existing.txt",
        "content": "new content"
    });
    let result = run_tool(&WriteTool, ctx, args);

    assert!(!result.is_error);
    let content = std::fs::read_to_string(dir.path().join("existing.txt")).unwrap();
    assert_eq!(content, "new content");
}

#[test]
fn test_write_tool_missing_args() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let cancel = CancellationToken::new();
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Missing path
    let args = serde_json::json!({ "content": "hello" });
    let err = rt
        .block_on(WriteTool.execute(ctx.clone(), args, cancel.clone()))
        .unwrap_err()
        .to_string();
    assert!(err.contains("path") || err.contains("Missing"));

    // Missing content
    let args = serde_json::json!({ "path": "test.txt" });
    let err = rt
        .block_on(WriteTool.execute(ctx, args, cancel))
        .unwrap_err()
        .to_string();
    assert!(err.contains("content") || err.contains("Missing"));
}

// -----------------------------------------------------------------------
// EditTool basics
// -----------------------------------------------------------------------

#[test]
fn test_edit_tool_single_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("edit.txt");
    std::fs::write(&file_path, "Hello old world").unwrap();

    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let args = serde_json::json!({
        "path": "edit.txt",
        "edits": [{
            "oldText": "old",
            "newText": "bright new"
        }]
    });
    let result = run_tool(&EditTool, ctx, args);

    assert!(!result.is_error);
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "Hello bright new world");
}

#[test]
fn test_edit_tool_multiple_replacements() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("multi.txt");
    std::fs::write(&file_path, "Apple Banana Cherry").unwrap();

    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let args = serde_json::json!({
        "path": "multi.txt",
        "edits": [
            { "oldText": "Apple", "newText": "Date" },
            { "oldText": "Cherry", "newText": "Elderberry" }
        ]
    });
    let result = run_tool(&EditTool, ctx, args);

    assert!(!result.is_error);
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "Date Banana Elderberry");
}

#[test]
fn test_edit_tool_old_text_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("notfound.txt");
    std::fs::write(&file_path, "Some content").unwrap();

    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let cancel = CancellationToken::new();
    let args = serde_json::json!({
        "path": "notfound.txt",
        "edits": [{ "oldText": "nonexistent", "newText": "replacement" }]
    });

    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(EditTool.execute(ctx, args, cancel));
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("not found") || err.contains("oldText"));
}

#[test]
fn test_edit_tool_old_text_not_unique() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("duplicate.txt");
    std::fs::write(&file_path, "Hello Hello Hello").unwrap();

    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let cancel = CancellationToken::new();
    let args = serde_json::json!({
        "path": "duplicate.txt",
        "edits": [{ "oldText": "Hello", "newText": "Hi" }]
    });

    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(EditTool.execute(ctx, args, cancel));
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("found"));
}

#[test]
fn test_edit_tool_nonexistent_file() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let cancel = CancellationToken::new();
    let args = serde_json::json!({
        "path": "no-such-file.txt",
        "edits": [{ "oldText": "a", "newText": "b" }]
    });

    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(EditTool.execute(ctx, args, cancel));
    assert!(result.is_err());
}

#[test]
fn test_edit_tool_empty_edits_array() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("empty_edits.txt");
    std::fs::write(&file_path, "content").unwrap();

    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let cancel = CancellationToken::new();
    let args = serde_json::json!({
        "path": "empty_edits.txt",
        "edits": []
    });

    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(EditTool.execute(ctx, args, cancel));
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("empty"));
}

// -----------------------------------------------------------------------
// GrepTool basics
// -----------------------------------------------------------------------

#[test]
fn test_grep_tool_finds_pattern() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("file1.rs"), "fn hello() {}\nfn world() {}").unwrap();

    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let args = serde_json::json!({ "pattern": "hello" });
    let result = run_tool(&GrepTool, ctx, args);

    assert!(!result.is_error);
    let text = tool_text(&result);
    assert!(text.contains("hello"));
}

#[test]
fn test_grep_tool_pattern_not_found() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("file.txt"), "some content").unwrap();

    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let args = serde_json::json!({ "pattern": "nonexistent" });
    let result = run_tool(&GrepTool, ctx, args);

    assert!(!result.is_error);
    let text = tool_text(&result);
    assert!(text.contains("No matches found"));
}

#[test]
fn test_grep_tool_literal_search() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("code.rs"),
        "let re = Regex::new(r\"hello\").unwrap();",
    )
    .unwrap();

    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let args = serde_json::json!({
        "pattern": "hello",
        "literal": true,
    });
    let result = run_tool(&GrepTool, ctx, args);

    assert!(!result.is_error);
    let text = tool_text(&result);
    assert!(text.contains("hello"));
}

#[test]
fn test_grep_tool_ignore_case() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("case.txt"), "Hello HELLO hello").unwrap();

    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let args = serde_json::json!({
        "pattern": "hello",
        "ignoreCase": false,
    });
    let result = run_tool(&GrepTool, ctx, args);

    // Case-sensitive finds one match ("hello" lowercase)
    let text = tool_text(&result);
    assert!(text.contains("1 match"));
}

#[test]
fn test_grep_tool_with_glob_filter() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("match.rs"), "fn find_me() {}").unwrap();
    std::fs::write(dir.path().join("ignore.txt"), "fn find_me() {}").unwrap();

    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let args = serde_json::json!({
        "pattern": "find_me",
        "glob": "*.rs",
    });
    let result = run_tool(&GrepTool, ctx, args);

    assert!(!result.is_error);
    let text = tool_text(&result);
    assert!(text.contains("match.rs"));
    assert!(!text.contains("ignore.txt"));
}

// -----------------------------------------------------------------------
// FindTool basics
// -----------------------------------------------------------------------

#[test]
fn test_find_tool_finds_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "").unwrap();
    std::fs::write(dir.path().join("b.rs"), "").unwrap();
    std::fs::write(dir.path().join("c.txt"), "").unwrap();

    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let args = serde_json::json!({ "pattern": "*.rs" });
    let result = run_tool(&FindTool, ctx, args);

    assert!(!result.is_error);
    let text = tool_text(&result);
    assert!(text.contains("a.rs"));
    assert!(text.contains("b.rs"));
    assert!(!text.contains("c.txt"));
}

#[test]
fn test_find_tool_no_matches() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("readme.md"), "").unwrap();

    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let args = serde_json::json!({ "pattern": "*.py" });
    let result = run_tool(&FindTool, ctx, args);

    assert!(!result.is_error);
    let text = tool_text(&result);
    assert!(text.contains("No files found"));
}

#[test]
fn test_find_tool_with_limit() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..10 {
        std::fs::write(dir.path().join(format!("file_{i}.rs")), "").unwrap();
    }

    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let args = serde_json::json!({ "pattern": "*.rs", "limit": 3 });
    let result = run_tool(&FindTool, ctx, args);

    assert!(!result.is_error);
    let text = tool_text(&result);
    // Should mention the limit was reached
    assert!(text.contains("limit") || text.contains("Reached"));
}

// -----------------------------------------------------------------------
// ToolRegistry basics
// -----------------------------------------------------------------------

#[test]
fn test_tool_registry_has_all_defaults() {
    let registry = ToolRegistry::default();
    let defs = registry.definitions();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"read"));
    assert!(names.contains(&"write"));
    assert!(names.contains(&"edit"));
    assert!(names.contains(&"grep"));
    assert!(names.contains(&"find"));
    assert!(names.contains(&"bash"));
    assert!(names.contains(&"ask_user"));
    assert_eq!(defs.len(), 7);
}

#[test]
fn test_tool_registry_register_and_execute() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("test.txt"), "hello world").unwrap();

    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let registry = ToolRegistry::default();
    let cancel = CancellationToken::new();
    let args = serde_json::json!({ "path": "test.txt" });

    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute("read", ctx, args, cancel))
        .unwrap();
    assert!(!result.is_error);
}

#[test]
fn test_tool_registry_execute_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let registry = ToolRegistry::default();
    let cancel = CancellationToken::new();

    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute("nonexistent_tool", ctx, serde_json::json!({}), cancel));
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Unknown") || err.contains("nonexistent_tool"));
}

// -----------------------------------------------------------------------
// Tool definition structure
// -----------------------------------------------------------------------

#[test]
fn test_tool_definitions_have_required_fields() {
    let registry = ToolRegistry::default();
    for def in registry.definitions() {
        assert!(!def.name.is_empty(), "Tool name should not be empty");
        assert!(!def.description.is_empty(), "Tool '{}' description should not be empty", def.name);
        assert!(
            def.input_schema.get("type").and_then(|v| v.as_str()) == Some("object"),
            "Tool '{}' input_schema should have type='object'",
            def.name
        );
    }
}

#[test]
fn test_read_tool_definition() {
    let def = ReadTool.definition();
    assert_eq!(def.name, "read");
    let required = def.input_schema["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v == "path"));
}

#[test]
fn test_write_tool_definition() {
    let def = WriteTool.definition();
    assert_eq!(def.name, "write");
    let props = def.input_schema["properties"].as_object().unwrap();
    assert!(props.contains_key("path"));
    assert!(props.contains_key("content"));
}

// -----------------------------------------------------------------------
// Cancellation during tool execution
// -----------------------------------------------------------------------

#[test]
fn test_read_tool_cancelled_before_execution() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let cancel = CancellationToken::new();
    cancel.cancel(); // Cancel before execution

    let args = serde_json::json!({ "path": "some-file.txt" });
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(ReadTool.execute(ctx, args, cancel));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("cancelled"));
}

#[test]
fn test_write_tool_cancelled_before_execution() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let cancel = CancellationToken::new();
    cancel.cancel();

    let args = serde_json::json!({
        "path": "test.txt",
        "content": "hello"
    });
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(WriteTool.execute(ctx, args, cancel));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("cancelled"));
}

// -----------------------------------------------------------------------
// Read file with absolute path
// -----------------------------------------------------------------------

#[test]
fn test_read_tool_absolute_path() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("absolute.txt");
    std::fs::write(&file_path, "absolute path content").unwrap();

    let (ctx, _rx) = test_ctx(PathBuf::from("/tmp")); // Different cwd
    // Use the actual absolute path
    let abs = file_path.to_string_lossy().to_string();
    let args = serde_json::json!({ "path": abs });
    let result = run_tool(&ReadTool, ctx, args);

    assert!(!result.is_error);
    let text = tool_text(&result);
    assert!(text.contains("absolute path content"));
}

// -----------------------------------------------------------------------
// EditTool overlapping edits
// -----------------------------------------------------------------------

#[test]
fn test_edit_tool_overlapping_edits_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("overlap.txt");
    std::fs::write(&file_path, "A quick brown fox").unwrap();

    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let cancel = CancellationToken::new();
    let args = serde_json::json!({
        "path": "overlap.txt",
        "edits": [
            { "oldText": "quick", "newText": "slow" },
            { "oldText": "quick brown", "newText": "lazy" }
        ]
    });

    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(EditTool.execute(ctx, args, cancel));
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("overlap") || err.contains("Overlap"));
}

// -----------------------------------------------------------------------
// ToolResult metadata
// -----------------------------------------------------------------------

#[test]
fn test_write_tool_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _rx) = test_ctx(dir.path().to_path_buf());
    let args = serde_json::json!({
        "path": "meta.txt",
        "content": "test content"
    });
    let result = run_tool(&WriteTool, ctx, args);

    let meta = result.metadata.as_ref().unwrap();
    assert!(meta.get("path").and_then(|v| v.as_str()).is_some());
    assert_eq!(meta.get("bytes").and_then(|v| v.as_u64()), Some(12));
}
