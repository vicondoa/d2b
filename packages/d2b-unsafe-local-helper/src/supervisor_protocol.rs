use std::fmt;

pub(crate) const CREATE_METHOD_ID: u32 = 2_967_550_554;
pub(crate) const ATTACH_METHOD_ID: u32 = 2_881_761_703;
pub(crate) const DETACH_METHOD_ID: u32 = 4_237_767_817;
pub(crate) const LIST_METHOD_ID: u32 = 2_008_733_898;
pub(crate) const INSPECT_METHOD_ID: u32 = 535_990_562;
pub(crate) const KILL_METHOD_ID: u32 = 2_470_444_226;
pub(crate) const CANCEL_METHOD_ID: u32 = 299_551_284;

pub(crate) const MAX_ID_BYTES: usize = 64;
pub(crate) const MAX_REQUEST_LIFETIME_MS: u64 = 15 * 60 * 1_000;
pub(crate) const MAX_FUTURE_CLOCK_SKEW_MS: u64 = 30 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellMethod {
    Create,
    Attach,
    Detach,
    List,
    Inspect,
    Kill,
    Cancel,
}

impl ShellMethod {
    pub const fn method_id(self) -> u32 {
        match self {
            Self::Create => CREATE_METHOD_ID,
            Self::Attach => ATTACH_METHOD_ID,
            Self::Detach => DETACH_METHOD_ID,
            Self::List => LIST_METHOD_ID,
            Self::Inspect => INSPECT_METHOD_ID,
            Self::Kill => KILL_METHOD_ID,
            Self::Cancel => CANCEL_METHOD_ID,
        }
    }

    pub const fn mutating(self) -> bool {
        matches!(
            self,
            Self::Create | Self::Attach | Self::Detach | Self::Kill
        )
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ShellRequest {
    pub method: ShellMethod,
    pub request_id: [u8; 16],
    pub idempotency_key: Option<[u8; 32]>,
    pub issued_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub session_generation: u64,
    pub realm_id: String,
    pub workload_id: String,
    pub resource_id: String,
    pub operation_id: String,
    pub stream_id: String,
    pub attachment_indexes: Vec<u32>,
    pub output_ring_bytes: usize,
}

impl fmt::Debug for ShellRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShellRequest")
            .field("method", &self.method)
            .field("has_idempotency_key", &self.idempotency_key.is_some())
            .field("attachment_count", &self.attachment_indexes.len())
            .field("output_ring_bytes", &self.output_ring_bytes)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellState {
    Running,
    Attached,
    Exited,
    Degraded,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ShellSummary {
    pub resource_id: String,
    pub state: ShellState,
}

impl fmt::Debug for ShellSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShellSummary")
            .field("resource_id", &"<redacted>")
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ShellResponse {
    pub state: ShellState,
    pub operation_id: String,
    pub resource_id: String,
    pub stream_id: String,
    pub attachment_indexes: Vec<u32>,
    pub shells: Vec<ShellSummary>,
}

impl fmt::Debug for ShellResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShellResponse")
            .field("state", &self.state)
            .field("attachment_count", &self.attachment_indexes.len())
            .field("shell_count", &self.shells.len())
            .finish_non_exhaustive()
    }
}

pub(crate) fn valid_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && matches!(bytes.next(), Some(first) if first.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_method_ids_match_inventory() {
        assert_eq!(ShellMethod::Create.method_id(), CREATE_METHOD_ID);
        assert_eq!(ShellMethod::Attach.method_id(), ATTACH_METHOD_ID);
        assert_eq!(ShellMethod::Detach.method_id(), DETACH_METHOD_ID);
        assert_eq!(ShellMethod::List.method_id(), LIST_METHOD_ID);
        assert_eq!(ShellMethod::Inspect.method_id(), INSPECT_METHOD_ID);
        assert_eq!(ShellMethod::Kill.method_id(), KILL_METHOD_ID);
        assert_eq!(ShellMethod::Cancel.method_id(), CANCEL_METHOD_ID);
    }

    #[test]
    fn request_debug_redacts_all_identifiers() {
        let canary = "private-shell-canary";
        let request = ShellRequest {
            method: ShellMethod::Create,
            request_id: [7; 16],
            idempotency_key: Some([8; 32]),
            issued_at_unix_ms: 1,
            expires_at_unix_ms: 2,
            session_generation: 3,
            realm_id: canary.into(),
            workload_id: canary.into(),
            resource_id: canary.into(),
            operation_id: canary.into(),
            stream_id: canary.into(),
            attachment_indexes: vec![],
            output_ring_bytes: 1,
        };
        assert!(!format!("{request:?}").contains(canary));
    }
}
