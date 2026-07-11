use d2b_contracts::terminal_wire::{TerminalSize, TerminalStatus};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

pub(crate) const SUPERVISOR_PROTOCOL_VERSION: u32 = 1;
pub(crate) const MAX_SUPERVISOR_FRAME_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupervisorRequest {
    pub version: u32,
    pub request_id: u64,
    pub action: SupervisorAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "op",
    content = "args",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum SupervisorAction {
    Status,
    Attach {
        #[serde(default)]
        force: bool,
        initial_terminal_size: TerminalSize,
    },
    Detach,
    Kill,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupervisorResponse {
    pub version: u32,
    pub request_id: u64,
    pub result: SupervisorResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum SupervisorResult {
    Status {
        running: bool,
        attached: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        terminal_status: Option<TerminalStatus>,
    },
    Attached {
        force_evicted: bool,
    },
    Detached {
        detached: bool,
    },
    KillAccepted,
    Rejected {
        code: SupervisorFailure,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SupervisorFailure {
    InvalidRequest,
    AlreadyAttached,
    Closed,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SupervisorProtocolError {
    Io,
    InvalidFrame,
    FrameTooLarge,
    VersionMismatch,
}

pub(crate) fn write_frame<T: Serialize>(
    stream: &mut UnixStream,
    value: &T,
) -> Result<(), SupervisorProtocolError> {
    let body = serde_json::to_vec(value).map_err(|_| SupervisorProtocolError::InvalidFrame)?;
    if body.is_empty() || body.len() > MAX_SUPERVISOR_FRAME_BYTES {
        return Err(SupervisorProtocolError::FrameTooLarge);
    }
    let length = u32::try_from(body.len()).map_err(|_| SupervisorProtocolError::FrameTooLarge)?;
    stream
        .write_all(&length.to_le_bytes())
        .and_then(|()| stream.write_all(&body))
        .map_err(|_| SupervisorProtocolError::Io)
}

pub(crate) fn read_frame<T: DeserializeOwned>(
    stream: &mut UnixStream,
) -> Result<T, SupervisorProtocolError> {
    let mut prefix = [0u8; 4];
    stream
        .read_exact(&mut prefix)
        .map_err(|_| SupervisorProtocolError::Io)?;
    let length = u32::from_le_bytes(prefix) as usize;
    if length == 0 || length > MAX_SUPERVISOR_FRAME_BYTES {
        return Err(SupervisorProtocolError::FrameTooLarge);
    }
    let mut body = vec![0u8; length];
    stream
        .read_exact(&mut body)
        .map_err(|_| SupervisorProtocolError::Io)?;
    serde_json::from_slice(&body).map_err(|_| SupervisorProtocolError::InvalidFrame)
}

pub(crate) fn validate_request(request: &SupervisorRequest) -> Result<(), SupervisorProtocolError> {
    (request.version == SUPERVISOR_PROTOCOL_VERSION)
        .then_some(())
        .ok_or(SupervisorProtocolError::VersionMismatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> SupervisorRequest {
        SupervisorRequest {
            version: SUPERVISOR_PROTOCOL_VERSION,
            request_id: 7,
            action: SupervisorAction::Status,
        }
    }

    #[test]
    fn private_protocol_is_correlated_strict_and_bounded() {
        let encoded = serde_json::to_vec(&request()).unwrap();
        let decoded: SupervisorRequest = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, request());

        let unknown = br#"{"version":1,"requestId":7,"action":{"op":"status"},"path":"no"}"#;
        assert!(serde_json::from_slice::<SupervisorRequest>(unknown).is_err());
        let trailing = [encoded.as_slice(), b"{}"].concat();
        assert!(serde_json::from_slice::<SupervisorRequest>(&trailing).is_err());
    }

    #[test]
    fn stream_decoder_rejects_oversized_and_truncated_frames() {
        let (mut writer, mut reader) = UnixStream::pair().unwrap();
        writer
            .write_all(&((MAX_SUPERVISOR_FRAME_BYTES + 1) as u32).to_le_bytes())
            .unwrap();
        assert_eq!(
            read_frame::<SupervisorRequest>(&mut reader),
            Err(SupervisorProtocolError::FrameTooLarge)
        );

        let (mut writer, mut reader) = UnixStream::pair().unwrap();
        writer.write_all(&10u32.to_le_bytes()).unwrap();
        writer.write_all(b"short").unwrap();
        drop(writer);
        assert_eq!(
            read_frame::<SupervisorRequest>(&mut reader),
            Err(SupervisorProtocolError::Io)
        );
    }

    #[test]
    fn protocol_version_is_closed() {
        let mut old = request();
        old.version = 0;
        assert_eq!(
            validate_request(&old),
            Err(SupervisorProtocolError::VersionMismatch)
        );
    }
}
