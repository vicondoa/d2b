use std::future::Future;
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
        });
    };
    duplicate_fd_cloexec(received_fds[fd_slot], context)
}

pub fn block_on_future<T>(future: impl Future<Output = T>) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build temporary tokio runtime")
            .block_on(future),
    }
}

pub fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn projection_digest_bytes(value: &str) -> Option<[u8; 32]> {
    (!value.is_empty()).then(|| sha2::Sha256::digest(value.as_bytes()).into())
}
