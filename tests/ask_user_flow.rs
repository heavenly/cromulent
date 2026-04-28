use cromulent::protocol::types::AskUserResponse;
use cromulent::tools::ask_user::AskManagerHandle;

/// Helper: block on the async register call to get a oneshot Receiver.
fn register_sync(manager: &AskManagerHandle, ask_id: &str) -> tokio::sync::oneshot::Receiver<AskUserResponse> {
    let id = ask_id.to_string();
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(manager.register(id))
}

// -----------------------------------------------------------------------
// Resolve unknown ask ID returns error
// -----------------------------------------------------------------------

#[test]
fn test_ask_resolve_unknown_id() {
    let manager = AskManagerHandle::new();

    let response = AskUserResponse {
        selected: vec![],
        freeform: None,
        comment: None,
    };

    let result = manager.resolve("nonexistent_ask", response);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("Unknown") || err.contains("nonexistent_ask"));
}

// -----------------------------------------------------------------------
// Register then resolve delivers value via oneshot
// -----------------------------------------------------------------------

#[test]
fn test_ask_register_and_resolve() {
    let manager = AskManagerHandle::new();
    let ask_id = "ask_test_1";

    let mut rx = register_sync(&manager, ask_id);

    let response = AskUserResponse {
        selected: vec!["Option A".into()],
        freeform: Some("custom input".into()),
        comment: None,
    };

    assert!(manager.resolve(ask_id, response).is_ok());

    let received = rx.try_recv().expect("should have value immediately");
    assert_eq!(received.selected, vec!["Option A"]);
    assert_eq!(received.freeform, Some("custom input".into()));
    assert!(received.comment.is_none());
}

#[test]
fn test_ask_register_resolve_with_comment() {
    let manager = AskManagerHandle::new();
    let ask_id = "ask_comment";

    let mut rx = register_sync(&manager, ask_id);

    let response = AskUserResponse {
        selected: vec![],
        freeform: None,
        comment: Some("Looks good!".into()),
    };

    assert!(manager.resolve(ask_id, response).is_ok());

    let received = rx.try_recv().expect("should have value immediately");
    assert!(received.selected.is_empty());
    assert!(received.freeform.is_none());
    assert_eq!(received.comment, Some("Looks good!".into()));
}

// -----------------------------------------------------------------------
// Register multiple asks and resolve independently
// -----------------------------------------------------------------------

#[test]
fn test_ask_multiple_independent() {
    let manager = AskManagerHandle::new();

    let mut rx1 = register_sync(&manager, "ask_a");
    let mut rx2 = register_sync(&manager, "ask_b");

    assert!(manager
        .resolve(
            "ask_a",
            AskUserResponse {
                selected: vec!["A".into()],
                freeform: None,
                comment: None,
            },
        )
        .is_ok());

    assert!(manager
        .resolve(
            "ask_b",
            AskUserResponse {
                selected: vec!["B".into()],
                freeform: None,
                comment: None,
            },
        )
        .is_ok());

    assert_eq!(rx1.try_recv().unwrap().selected, vec!["A"]);
    assert_eq!(rx2.try_recv().unwrap().selected, vec!["B"]);
}

// -----------------------------------------------------------------------
// Duplicate resolve returns error
// -----------------------------------------------------------------------

#[test]
fn test_ask_duplicate_resolve_error() {
    let manager = AskManagerHandle::new();
    let ask_id = "ask_dup";

    let _rx = register_sync(&manager, ask_id);

    let response = AskUserResponse {
        selected: vec!["First".into()],
        freeform: None,
        comment: None,
    };

    assert!(manager.resolve(ask_id, response).is_ok());

    let response2 = AskUserResponse {
        selected: vec!["Second".into()],
        freeform: None,
        comment: None,
    };
    let result = manager.resolve(ask_id, response2);
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// Cancel all pending asks
// -----------------------------------------------------------------------

#[test]
fn test_ask_cancel_all() {
    let manager = AskManagerHandle::new();

    let mut rx1 = register_sync(&manager, "ask_cancel_1");
    let mut rx2 = register_sync(&manager, "ask_cancel_2");

    manager.cancel_all();

    assert!(rx1.try_recv().is_err(), "First ask should be cancelled");
    assert!(rx2.try_recv().is_err(), "Second ask should be cancelled");
}

#[test]
fn test_ask_cancel_then_resolve_returns_error() {
    let manager = AskManagerHandle::new();
    let ask_id = "ask_cancel_resolve";

    let _rx = register_sync(&manager, ask_id);

    manager.cancel_all();

    let response = AskUserResponse {
        selected: vec![],
        freeform: None,
        comment: None,
    };

    let result = manager.resolve(ask_id, response);
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// Register same id twice — second overwrites first
// -----------------------------------------------------------------------

#[test]
fn test_ask_register_twice_same_id() {
    let manager = AskManagerHandle::new();
    let ask_id = "ask_dup_id";

    let mut rx1 = register_sync(&manager, ask_id);

    // Second registration overwrites the pending entry
    let mut rx2 = register_sync(&manager, ask_id);

    // rx1 should get nothing (channel replaced)
    assert!(rx1.try_recv().is_err());

    assert!(manager
        .resolve(
            ask_id,
            AskUserResponse {
                selected: vec!["B".into()],
                freeform: None,
                comment: None,
            },
        )
        .is_ok());

    assert!(rx1.try_recv().is_err());
    assert_eq!(rx2.try_recv().unwrap().selected, vec!["B"]);
}

// -----------------------------------------------------------------------
// AskManagerHandle is Clone — both handles share the same state
// -----------------------------------------------------------------------

#[test]
fn test_ask_handle_clone_shares_state() {
    let manager = AskManagerHandle::new();
    let manager2 = manager.clone();

    let mut rx = register_sync(&manager2, "ask_clone");

    let response = AskUserResponse {
        selected: vec!["cloned".into()],
        freeform: None,
        comment: None,
    };
    assert!(manager.resolve("ask_clone", response).is_ok());

    let received = rx.try_recv().expect("should have value immediately");
    assert_eq!(received.selected, vec!["cloned"]);
}
