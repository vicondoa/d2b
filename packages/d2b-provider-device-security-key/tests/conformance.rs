use d2b_provider_device_security_key::{
    DEFAULT_LEASE_TIMEOUT_SECS, DEFAULT_SESSION_RING_SIZE, DEFAULT_VSOCK_PORT,
    MAX_LEASE_TIMEOUT_SECS, MAX_SESSION_RING_SIZE, MIN_LEASE_TIMEOUT_SECS, MIN_SESSION_RING_SIZE,
    SessionRing,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Settings {
    #[serde(default = "default_port")]
    vsock_port: u16,
    #[serde(default = "default_ring")]
    session_ring_size: usize,
    #[serde(default = "default_timeout")]
    lease_timeout_secs: u64,
}

fn default_port() -> u16 {
    DEFAULT_VSOCK_PORT
}
fn default_ring() -> usize {
    DEFAULT_SESSION_RING_SIZE
}
fn default_timeout() -> u64 {
    DEFAULT_LEASE_TIMEOUT_SECS
}

#[test]
fn settings_defaults_and_unknown_fields_are_closed() {
    let settings: Settings = serde_json::from_str("{}").unwrap();
    assert_eq!(settings.vsock_port, DEFAULT_VSOCK_PORT);
    assert_eq!(settings.session_ring_size, DEFAULT_SESSION_RING_SIZE);
    assert_eq!(settings.lease_timeout_secs, DEFAULT_LEASE_TIMEOUT_SECS);
    assert!(serde_json::from_str::<Settings>(r#"{"path":"/dev/hidraw0"}"#).is_err());
    assert!(SessionRing::new(MIN_SESSION_RING_SIZE - 1).is_err());
    assert!(SessionRing::new(MAX_SESSION_RING_SIZE + 1).is_err());
    assert!(
        (MIN_LEASE_TIMEOUT_SECS..=MAX_LEASE_TIMEOUT_SECS).contains(&settings.lease_timeout_secs)
    );
}
