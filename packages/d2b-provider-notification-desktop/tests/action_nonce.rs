use d2b_provider_notification_desktop::{ActionNonceError, ActionNonceStore};

#[test]
fn nonce_is_consumed_once() {
    let mut store = ActionNonceStore::new(2, 120);
    let nonce = store.register("observer", "cancel", 10).unwrap();
    assert_eq!(
        store.consume(&nonce.action_key(), "observer", 11),
        Ok("cancel".to_owned())
    );
    assert!(store.consume(&nonce.action_key(), "observer", 11).is_err());
}

#[test]
fn nonce_store_bounds_action_text_and_hashes_large_sessions() {
    let mut store = ActionNonceStore::new(2, 120);
    let session = "session-".repeat(1024);
    let nonce = store.register(&session, "open", 10).unwrap();
    assert_eq!(
        store.consume(&nonce.action_key(), &session, 11),
        Ok("open".to_owned())
    );

    let oversized_action = "a".repeat(65);
    assert_eq!(
        store.register("session", oversized_action, 10),
        Err(ActionNonceError::Invalid)
    );
}
