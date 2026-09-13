//! Deterministic Azure operation identifiers.

use sha2::{Digest, Sha256};

/// Derive a stable 20-character operation identifier.
pub fn operation_id(
    zone_uid: &str,
    guest_uid: &str,
    generation: u64,
    operation_class: &str,
) -> String {
    let mut digest = Sha256::new();
    digest.update(zone_uid.as_bytes());
    digest.update([0]);
    digest.update(guest_uid.as_bytes());
    digest.update([0]);
    digest.update(generation.to_be_bytes());
    digest.update([0]);
    digest.update(operation_class.as_bytes());
    base32(&digest.finalize())[..20].to_owned()
}

fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut output = String::with_capacity((bytes.len() * 8).div_ceil(5));
    let mut buffer = 0u16;
    let mut bits = 0u8;
    for byte in bytes {
        buffer = (buffer << 8) | u16::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            output.push(ALPHABET[((buffer >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits != 0 {
        output.push(ALPHABET[((buffer << (5 - bits)) & 0x1f) as usize] as char);
    }
    output
}
