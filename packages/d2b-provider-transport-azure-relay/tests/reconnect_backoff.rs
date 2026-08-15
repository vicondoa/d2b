use d2b_provider_transport_azure_relay::{ReconnectBackoff, ReconnectDecision};

#[test]
fn reconnect_backoff_is_bounded_and_resets_after_stability() {
    let mut backoff = ReconnectBackoff::new(4_000, 30_000);
    assert_eq!(backoff.failed(), ReconnectDecision::RetryAfter(1_000));
    assert_eq!(backoff.failed(), ReconnectDecision::RetryAfter(2_000));
    assert_eq!(backoff.failed(), ReconnectDecision::RetryAfter(4_000));
    assert_eq!(backoff.failed(), ReconnectDecision::RetryAfter(4_000));
    backoff.stable(30_000);
    assert_eq!(backoff.attempts(), 0);
}
