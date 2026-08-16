use d2b_provider_runtime_azure_virtual_machine::idempotency::operation_id;

#[test]
fn operation_id_is_stable_and_bounded() {
    let first = operation_id("zone", "guest", 4, "provision");
    let second = operation_id("zone", "guest", 4, "provision");
    assert_eq!(first, second);
    assert_eq!(first.len(), 20);
    assert_ne!(first, operation_id("zone", "guest", 5, "provision"));
}
