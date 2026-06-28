use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

pub const PICKER_TO_DAEMON_MAX_FRAME_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenRequestFrameCaps {
    pub max_candidates: usize,
    pub max_preview_bytes: usize,
    pub max_thumbnail_bytes: usize,
    pub max_metadata_bytes: usize,
}

impl Default for OpenRequestFrameCaps {
    fn default() -> Self {
        Self {
            max_candidates: 64,
            max_preview_bytes: 2048,
            max_thumbnail_bytes: 0,
            max_metadata_bytes: 1024,
        }
    }
}

impl OpenRequestFrameCaps {
    pub fn max_frame_bytes(self) -> usize {
        const ENVELOPE_BYTES: usize = 4096;
        let per_candidate = self
            .max_preview_bytes
            .saturating_add(self.max_thumbnail_bytes.saturating_mul(4).div_ceil(3))
            .saturating_add(self.max_metadata_bytes);
        ENVELOPE_BYTES.saturating_add(self.max_candidates.saturating_mul(per_candidate))
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FramingError {
    #[error("ndjson frame exceeds {max} bytes")]
    FrameTooLong { max: usize },
    #[error("ndjson frame is incomplete")]
    Incomplete,
    #[error("ndjson frame is not utf-8")]
    InvalidUtf8,
    #[error("json error: {0}")]
    Json(String),
}

pub fn encode_frame<T: Serialize>(
    value: &T,
    max_frame_bytes: usize,
) -> Result<Vec<u8>, FramingError> {
    let mut encoded =
        serde_json::to_vec(value).map_err(|err| FramingError::Json(err.to_string()))?;
    if encoded.len() > max_frame_bytes {
        return Err(FramingError::FrameTooLong {
            max: max_frame_bytes,
        });
    }
    encoded.push(b'\n');
    Ok(encoded)
}

pub fn decode_frame<T: DeserializeOwned>(
    bytes: &[u8],
    max_frame_bytes: usize,
) -> Result<T, FramingError> {
    let line = bounded_line(bytes, max_frame_bytes)?;
    serde_json::from_str(line).map_err(|err| FramingError::Json(err.to_string()))
}

pub fn bounded_line(bytes: &[u8], max_frame_bytes: usize) -> Result<&str, FramingError> {
    match bytes.iter().position(|byte| *byte == b'\n') {
        Some(line_len) if line_len <= max_frame_bytes => {
            std::str::from_utf8(&bytes[..line_len]).map_err(|_| FramingError::InvalidUtf8)
        }
        Some(_) => Err(FramingError::FrameTooLong {
            max: max_frame_bytes,
        }),
        None if bytes.len() > max_frame_bytes => Err(FramingError::FrameTooLong {
            max: max_frame_bytes,
        }),
        None => Err(FramingError::Incomplete),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ClientHello, PickerToDaemonMessage, ProtocolVersionRange};

    #[test]
    fn rejects_overlong_picker_frame_before_json() {
        let mut bytes = vec![b'a'; PICKER_TO_DAEMON_MAX_FRAME_BYTES + 1];
        bytes.push(b'\n');

        let err = bounded_line(&bytes, PICKER_TO_DAEMON_MAX_FRAME_BYTES).expect_err("overlong");
        assert_eq!(
            err,
            FramingError::FrameTooLong {
                max: PICKER_TO_DAEMON_MAX_FRAME_BYTES
            }
        );
    }

    #[test]
    fn accepts_valid_maxish_picker_line() {
        let mut bytes = vec![b'a'; PICKER_TO_DAEMON_MAX_FRAME_BYTES];
        bytes.push(b'\n');

        let line = bounded_line(&bytes, PICKER_TO_DAEMON_MAX_FRAME_BYTES).expect("line");
        assert_eq!(line.len(), PICKER_TO_DAEMON_MAX_FRAME_BYTES);
    }

    #[test]
    fn accepts_valid_maxish_open_request_line() {
        let max = OpenRequestFrameCaps::default().max_frame_bytes();
        let mut bytes = vec![b'b'; max];
        bytes.push(b'\n');

        let line = bounded_line(&bytes, max).expect("line");
        assert_eq!(line.len(), max);
    }

    #[test]
    fn encode_decode_picker_message() {
        let message = PickerToDaemonMessage::ClientHello(ClientHello {
            protocol_version_range: ProtocolVersionRange { min: 1, max: 1 },
            picker_version: "picker-test".to_owned(),
        });

        let frame = encode_frame(&message, PICKER_TO_DAEMON_MAX_FRAME_BYTES).expect("encode");
        let decoded: PickerToDaemonMessage =
            decode_frame(&frame, PICKER_TO_DAEMON_MAX_FRAME_BYTES).expect("decode");
        assert_eq!(decoded, message);
    }
}
