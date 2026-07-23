//! Display transport preface gate (ADR 0032, P0).
//!
//! The display byte stream is distinct from the guest-control stream. Before
//! any Wayland/Waypipe byte is forwarded over the reserved AF_VSOCK display
//! port, the endpoint consumes a small credential preface bound to the gateway
//! CID, daemon generation, and authorized display stream id. A raw AF_VSOCK
//! connection is reachability only; this preface is the local display-stream
//! token binding that prevents "connect to the port" from becoming
//! authorization.

use d2b_realm_core::{ConstellationError, ErrorKind, StreamId};
use subtle::ConstantTimeEq;

/// Reserved AF_VSOCK port for the dedicated display stream.
pub const DISPLAY_VSOCK_PORT: u32 = 14_319;

/// Fixed-length display stream token (redacted in `Debug`).
pub const DISPLAY_TOKEN_LEN: usize = 32;

const MAGIC: &[u8; 12] = b"D2B-DISPLAY\0";
const VERSION: u8 = 1;

/// Non-secret binding context for one display stream credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayTransportBinding {
    /// Gateway guest CID expected on the local AF_VSOCK path.
    pub gateway_cid: u32,
    /// Gateway daemon generation that minted the token.
    pub generation: u64,
    /// Authorized display stream id.
    pub stream: StreamId,
}

/// Secret display token. `Debug` redacts the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct DisplayTransportToken([u8; DISPLAY_TOKEN_LEN]);

impl DisplayTransportToken {
    /// Build from exact bytes.
    pub fn from_bytes(bytes: [u8; DISPLAY_TOKEN_LEN]) -> Self {
        Self(bytes)
    }

    /// Borrow for token delivery / verification internals.
    pub fn expose(&self) -> &[u8; DISPLAY_TOKEN_LEN] {
        &self.0
    }
}

impl core::fmt::Debug for DisplayTransportToken {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("DisplayTransportToken(<redacted>)")
    }
}

/// Encode the credential preface that must be the first bytes on the dedicated
/// display transport.
pub fn encode_display_preface(
    binding: &DisplayTransportBinding,
    token: &DisplayTransportToken,
) -> Vec<u8> {
    let stream = binding.stream.as_str().as_bytes();
    let mut out =
        Vec::with_capacity(MAGIC.len() + 1 + 4 + 8 + 2 + stream.len() + DISPLAY_TOKEN_LEN);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&binding.gateway_cid.to_be_bytes());
    out.extend_from_slice(&binding.generation.to_be_bytes());
    out.extend_from_slice(&(stream.len() as u16).to_be_bytes());
    out.extend_from_slice(stream);
    out.extend_from_slice(token.expose());
    out
}

/// Verify the first bytes on the display transport. Returns the byte offset
/// after the preface on success, so callers can forward any remaining bytes as
/// display payload. Any mismatch fails closed.
pub fn verify_display_preface(
    frame: &[u8],
    expected_binding: &DisplayTransportBinding,
    expected_token: &DisplayTransportToken,
) -> Result<usize, ConstellationError> {
    let mut cursor = 0;
    take(frame, &mut cursor, MAGIC.len())
        .filter(|got| *got == MAGIC)
        .ok_or_else(malformed)?;
    let version = take(frame, &mut cursor, 1)
        .and_then(|b| b.first().copied())
        .ok_or_else(malformed)?;
    if version != VERSION {
        return Err(malformed());
    }
    let cid = read_u32(frame, &mut cursor)?;
    let generation = read_u64(frame, &mut cursor)?;
    let stream_len = read_u16(frame, &mut cursor)? as usize;
    let stream = take(frame, &mut cursor, stream_len).ok_or_else(malformed)?;
    let token = take(frame, &mut cursor, DISPLAY_TOKEN_LEN).ok_or_else(malformed)?;

    if cid != expected_binding.gateway_cid
        || generation != expected_binding.generation
        || stream != expected_binding.stream.as_str().as_bytes()
        || !bool::from(token.ct_eq(expected_token.expose()))
    {
        return Err(malformed());
    }
    Ok(cursor)
}

fn take<'a>(frame: &'a [u8], cursor: &mut usize, len: usize) -> Option<&'a [u8]> {
    let end = cursor.checked_add(len)?;
    let bytes = frame.get(*cursor..end)?;
    *cursor = end;
    Some(bytes)
}

fn read_u16(frame: &[u8], cursor: &mut usize) -> Result<u16, ConstellationError> {
    let bytes: [u8; 2] = take(frame, cursor, 2)
        .ok_or_else(malformed)?
        .try_into()
        .expect("slice length is fixed");
    Ok(u16::from_be_bytes(bytes))
}

fn read_u32(frame: &[u8], cursor: &mut usize) -> Result<u32, ConstellationError> {
    let bytes: [u8; 4] = take(frame, cursor, 4)
        .ok_or_else(malformed)?
        .try_into()
        .expect("slice length is fixed");
    Ok(u32::from_be_bytes(bytes))
}

fn read_u64(frame: &[u8], cursor: &mut usize) -> Result<u64, ConstellationError> {
    let bytes: [u8; 8] = take(frame, cursor, 8)
        .ok_or_else(malformed)?
        .try_into()
        .expect("slice length is fixed");
    Ok(u64::from_be_bytes(bytes))
}

fn malformed() -> ConstellationError {
    ConstellationError::new(
        ErrorKind::MalformedFrame,
        "display transport preface did not match the authorized session",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> DisplayTransportBinding {
        DisplayTransportBinding {
            gateway_cid: 42,
            generation: 7,
            stream: StreamId::parse("display-1").unwrap(),
        }
    }

    fn token(byte: u8) -> DisplayTransportToken {
        DisplayTransportToken::from_bytes([byte; DISPLAY_TOKEN_LEN])
    }

    #[test]
    fn preface_verifies_and_returns_payload_offset() {
        let binding = binding();
        let token = token(9);
        let mut frame = encode_display_preface(&binding, &token);
        let payload_offset = frame.len();
        frame.extend_from_slice(b"wayland-bytes");
        assert_eq!(
            verify_display_preface(&frame, &binding, &token).unwrap(),
            payload_offset
        );
    }

    #[test]
    fn wrong_cid_generation_stream_or_token_rejects() {
        let binding = binding();
        let good_token = token(9);
        let frame = encode_display_preface(&binding, &good_token);

        let mut wrong = binding.clone();
        wrong.gateway_cid += 1;
        assert!(verify_display_preface(&frame, &wrong, &good_token).is_err());

        let mut wrong = binding.clone();
        wrong.generation += 1;
        assert!(verify_display_preface(&frame, &wrong, &good_token).is_err());

        let mut wrong = binding.clone();
        wrong.stream = StreamId::parse("display-2").unwrap();
        assert!(verify_display_preface(&frame, &wrong, &good_token).is_err());

        assert!(verify_display_preface(&frame, &binding, &token(8)).is_err());
    }

    #[test]
    fn malformed_or_truncated_preface_rejects() {
        let binding = binding();
        let token = token(9);
        let frame = encode_display_preface(&binding, &token);
        for len in 0..frame.len() {
            assert!(verify_display_preface(&frame[..len], &binding, &token).is_err());
        }
    }

    #[test]
    fn token_debug_redacts_bytes() {
        assert_eq!(
            format!("{:?}", token(1)),
            "DisplayTransportToken(<redacted>)"
        );
    }

    #[test]
    fn display_vsock_port_is_stable() {
        assert_eq!(DISPLAY_VSOCK_PORT, 14_319);
    }
}
