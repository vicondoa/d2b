//! Guest-control authentication transcript helpers.
//!
//! This module is intentionally transport/runtime free so guestd, d2bd,
//! and the privileged broker can share one canonical HMAC transcript encoder
//! without pulling ttRPC into `d2b-contracts`.

pub const AUTH_TRANSCRIPT_VERSION: u32 = 1;
pub const AUTH_NONCE_LEN: usize = 32;
pub const AUTH_TAG_LEN: usize = 32;
pub const GUEST_CONTROL_AUTH_PORT: u32 = 14_318;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuthDirection {
    HostToGuest,
}

impl AuthDirection {
    pub const fn label(self) -> &'static [u8] {
        match self {
            Self::HostToGuest => b"host-to-guest",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuthPurpose {
    GuestControlAuthV1,
}

impl AuthPurpose {
    pub const fn label(self) -> &'static [u8] {
        match self {
            Self::GuestControlAuthV1 => b"guest-control-auth-v1",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProofRole {
    Host,
    Guest,
}

impl ProofRole {
    pub const fn label(self) -> &'static [u8] {
        match self {
            Self::Host => b"host-proof",
            Self::Guest => b"guest-proof",
        }
    }
}

pub struct GuestAuthTranscript<'a> {
    pub role: ProofRole,
    pub direction: AuthDirection,
    pub purpose: AuthPurpose,
    pub vm_id: &'a str,
    pub protocol_version: u32,
    pub guest_control_port: u32,
    pub peer_cid: Option<u32>,
    pub host_nonce: &'a [u8; AUTH_NONCE_LEN],
    pub guest_nonce: &'a [u8; AUTH_NONCE_LEN],
    pub guest_boot_id: &'a str,
    pub capabilities_hash: Option<&'a [u8]>,
}

pub fn encode_transcript(transcript: &GuestAuthTranscript<'_>) -> Vec<u8> {
    let mut out = Vec::new();
    push_field(&mut out, 1, b"guest-control-auth-v1");
    push_field(&mut out, 2, transcript.role.label());
    push_field(&mut out, 3, transcript.direction.label());
    push_field(&mut out, 4, transcript.purpose.label());
    push_field(&mut out, 5, transcript.vm_id.as_bytes());
    push_field(&mut out, 6, &transcript.protocol_version.to_be_bytes());
    push_field(&mut out, 7, &transcript.guest_control_port.to_be_bytes());
    if let Some(peer_cid) = transcript.peer_cid {
        push_field(&mut out, 8, &peer_cid.to_be_bytes());
    }
    push_field(&mut out, 10, transcript.host_nonce);
    push_field(&mut out, 11, transcript.guest_nonce);
    push_field(&mut out, 12, transcript.guest_boot_id.as_bytes());
    if let Some(capabilities_hash) = transcript.capabilities_hash {
        push_field(&mut out, 13, capabilities_hash);
    }
    out
}

fn push_field(out: &mut Vec<u8>, tag: u8, value: &[u8]) {
    out.push(tag);
    out.extend_from_slice(&(value.len() as u32).to_be_bytes());
    out.extend_from_slice(value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_encoding_is_stable() {
        let host_nonce = [0x11; AUTH_NONCE_LEN];
        let guest_nonce = [0x22; AUTH_NONCE_LEN];
        let encoded = encode_transcript(&GuestAuthTranscript {
            role: ProofRole::Host,
            direction: AuthDirection::HostToGuest,
            purpose: AuthPurpose::GuestControlAuthV1,
            vm_id: "corp-vm",
            protocol_version: 1,
            guest_control_port: GUEST_CONTROL_AUTH_PORT,
            peer_cid: Some(2),
            host_nonce: &host_nonce,
            guest_nonce: &guest_nonce,
            guest_boot_id: "boot-1",
            capabilities_hash: None,
        });
        assert_eq!(&encoded[..5], &[1, 0, 0, 0, 21]);
        assert_eq!(encoded[5..26].as_ref(), b"guest-control-auth-v1");
        assert!(encoded.windows(16).all(|window| window != [0x33; 16]));
    }
}
