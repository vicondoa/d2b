//! Shared admission mapping for Process resource owner sessions.

use crate::{
    exec_session,
    typed_error::{ProcessExecErrorKind, TypedError},
};

pub fn map_exec_reserve_error(error: exec_session::SessionReserveError) -> TypedError {
    use exec_session::SessionReserveError as Error;
    let kind = match error {
        Error::RateLimited => ProcessExecErrorKind::RateLimited,
        _ => ProcessExecErrorKind::SessionCapacity,
    };
    TypedError::ProcessExecFailed { kind }
}
