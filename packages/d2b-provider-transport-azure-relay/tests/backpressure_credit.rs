use d2b_provider_transport_azure_relay::{BackpressureError, CreditWindow};

#[test]
fn credit_window_stalls_before_unbounded_growth() {
    let mut window = CreditWindow::new(8).unwrap();
    window.reserve(8).unwrap();
    assert_eq!(window.reserve(1), Err(BackpressureError::CreditExhausted));
    window.acknowledge(8);
    assert_eq!(window.available(), 8);
    assert_eq!(
        window.reserve(64 * 1024 + 1),
        Err(BackpressureError::FrameTooLarge)
    );
}

#[test]
fn remote_grants_cannot_exceed_the_credit_window() {
    let mut window = CreditWindow::new(8).unwrap();
    window.reserve(8).unwrap();
    window.grant(8);
    assert_eq!(window.available() + window.in_flight(), 8);
    window.acknowledge(8);
    assert_eq!(window.available() + window.in_flight(), 8);
}
