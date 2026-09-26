use std::os::fd::{OwnedFd, RawFd};

use crate::typed_error::TypedError;
use crate::unix_transport::duplicate_fd_cloexec;
use sha2::Digest;

pub fn duplicate_received_fd(
    received_fds: &[RawFd],
    fd_index: u32,
    context: &str,
) -> Result<OwnedFd, TypedError> {
    let Some(fd_slot) = usize::try_from(fd_index)
        .ok()
        .filter(|index| *index < received_fds.len())
    else {
        return Err(TypedError::InternalIo {
            context: context.to_owned(),
            detail: format!("missing SCM_RIGHTS fd at index {fd_index}"),
            source: None,
        });
    };
    duplicate_fd_cloexec(received_fds[fd_slot], context)
}

pub fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn projection_digest_bytes(value: &str) -> Option<[u8; 32]> {
    (!value.is_empty()).then(|| sha2::Sha256::digest(value.as_bytes()).into())
}

/// Monotonic process-lifetime tick in elapsed milliseconds, used to
/// sequence guest admission attempts without trusting guest clocks.
pub(crate) fn monotonic_tick() -> u64 {
    static START: std::sync::LazyLock<std::time::Instant> = std::sync::LazyLock::new(std::time::Instant::now);
    START
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(1)
        .max(1)
}


