//! Minimal, dependency-free standard (RFC 4648) base64 codec with padding.
//!
//! The guest-control config-read transport carries raw guest bytes as a
//! base64 string in the public.sock `ReadGuestConfig` response (the wire
//! envelope is JSON, so raw `Vec<u8>` would be serialized as a number array
//! and balloon the frame). This module keeps the framework free of an extra
//! third-party base64 dependency. It is pure safe Rust.

const ENCODE_TABLE: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const PAD: u8 = b'=';

/// Encode `input` as standard padded base64.
pub fn encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ENCODE_TABLE[((triple >> 18) & 0x3f) as usize] as char);
        out.push(ENCODE_TABLE[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(ENCODE_TABLE[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push(PAD as char);
        }
        if chunk.len() > 2 {
            out.push(ENCODE_TABLE[(triple & 0x3f) as usize] as char);
        } else {
            out.push(PAD as char);
        }
    }
    out
}

/// Error decoding a base64 string: a non-alphabet byte, bad padding, or a
/// length that is not a multiple of four.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError;

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid base64")
    }
}

impl std::error::Error for DecodeError {}

fn decode_symbol(byte: u8) -> Result<u8, DecodeError> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(DecodeError),
    }
}

/// Decode a standard padded base64 string. Rejects any non-alphabet byte,
/// misplaced padding, or non-multiple-of-four length (no whitespace allowed).
pub fn decode(input: &str) -> Result<Vec<u8>, DecodeError> {
    let bytes = input.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err(DecodeError);
    }
    let quad_count = bytes.len() / 4;
    let mut out = Vec::with_capacity(quad_count * 3);
    for (quad_idx, quad) in bytes.chunks(4).enumerate() {
        let is_last = quad_idx + 1 == quad_count;
        let mut buf = [0u8; 4];
        let mut pad = 0usize;
        for (i, &b) in quad.iter().enumerate() {
            if b == PAD {
                // Padding is only valid in the last one or two positions of
                // the FINAL quad. A padded quad anywhere before the end (e.g.
                // `Zg==AAAA`) is malformed: it would silently splice a short
                // group into the middle of the stream.
                if i < 2 || !is_last {
                    return Err(DecodeError);
                }
                pad += 1;
                buf[i] = 0;
            } else {
                if pad > 0 {
                    // A non-pad byte after a pad byte is malformed.
                    return Err(DecodeError);
                }
                buf[i] = decode_symbol(b)?;
            }
        }
        let triple = ((buf[0] as u32) << 18)
            | ((buf[1] as u32) << 12)
            | ((buf[2] as u32) << 6)
            | (buf[3] as u32);
        // Reject non-canonical padding: the bits that the dropped output
        // byte(s) would carry MUST be zero, so each padded input has exactly
        // one canonical encoding (e.g. `Zh==` and `Zm9=` are rejected even
        // though they are alphabet-valid).
        if (pad == 1 && triple & 0xff != 0) || (pad == 2 && triple & 0xffff != 0) {
            return Err(DecodeError);
        }
        out.push((triple >> 16) as u8);
        if pad < 2 {
            out.push((triple >> 8) as u8);
        }
        if pad < 1 {
            out.push(triple as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_known_vectors() {
        // RFC 4648 §10 test vectors.
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg==");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
        for vector in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foob",
            b"fooba",
            b"foobar",
        ] {
            assert_eq!(decode(&encode(vector)).unwrap(), vector);
        }
    }

    #[test]
    fn round_trips_arbitrary_bytes() {
        let data: Vec<u8> = (0u32..=511).map(|n| (n % 256) as u8).collect();
        assert_eq!(decode(&encode(&data)).unwrap(), data);
    }

    #[test]
    fn rejects_malformed_input() {
        assert_eq!(decode("Zg="), Err(DecodeError)); // wrong length
        assert_eq!(decode("Zg=a"), Err(DecodeError)); // data after pad
        assert_eq!(decode("====").err(), Some(DecodeError)); // pad at pos 0
        assert_eq!(decode("Zm9 v").err(), Some(DecodeError)); // whitespace
        assert_eq!(decode("Z!=="), Err(DecodeError)); // non-alphabet
    }

    #[test]
    fn rejects_padding_before_final_quad() {
        // A padded quad must be the LAST quad. `Zg==` alone decodes to "f",
        // but `Zg==AAAA` must be rejected rather than decode "f" + 3 bytes:
        // padding in a non-final quad would splice a short group mid-stream.
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(decode("Zg=="), Ok(b"f".to_vec()));
        assert_eq!(decode("Zg==AAAA"), Err(DecodeError));
        // Two-char padding mid-stream.
        assert_eq!(decode("Zm8=AAAA"), Err(DecodeError)); // "fo" + ...
        // One-char padding mid-stream.
        assert_eq!(decode("Zm9vYg==Zm9v"), Err(DecodeError));
        // A long run with a padded interior quad.
        assert_eq!(decode("Zm9vZg==Zm9v"), Err(DecodeError));
    }

    #[test]
    fn rejects_non_canonical_padding() {
        // `Zg==` is the canonical encoding of "f"; `Zh==` carries the same
        // leading symbol but non-zero discarded bits and MUST be rejected.
        assert_eq!(decode("Zg=="), Ok(b"f".to_vec()));
        assert_eq!(decode("Zh=="), Err(DecodeError));
        assert_eq!(decode("ZP=="), Err(DecodeError));
        // `Zm8=` is canonical for "fo"; `Zm9=` mutates the dropped low bits.
        assert_eq!(decode("Zm8="), Ok(b"fo".to_vec()));
        assert_eq!(decode("Zm9="), Err(DecodeError));
        // Every non-canonical two-pad variant of a single byte must fail;
        // only the four canonical second symbols (low nibble zero: A/Q/g/w)
        // decode (each to its own distinct byte).
        for second in b'A'..=b'z' {
            if !second.is_ascii_alphanumeric() {
                continue;
            }
            let candidate = format!("Z{}==", second as char);
            let expected = decode_symbol(second).unwrap().is_multiple_of(16);
            assert_eq!(
                decode(&candidate).is_ok(),
                expected,
                "non-canonical pad acceptance for {candidate}"
            );
        }
    }
}
