use d2b_provider_notification_desktop::ActionNonceStore;

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
