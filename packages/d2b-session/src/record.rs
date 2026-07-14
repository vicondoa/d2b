use std::{collections::VecDeque, fmt};

use d2b_contracts::v2_component_session::{
    LimitProfile, NOISE_TAG_BYTES, RECORD_HEADER_LEN, RECORD_LENGTH_BYTES, ReceiveSequence,
    RecordHeader, RecordKind, SendSequence, SessionErrorCode,
};
use sha2::{Digest, Sha256};
use snow::TransportState;

use crate::{EstablishedHandshake, Result, SessionError};

const REPLAY_CACHE_ENTRIES: usize = 1_024;

pub struct ProtectedRecord(Vec<u8>);

impl ProtectedRecord {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl fmt::Debug for ProtectedRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProtectedRecord")
            .field("ciphertext", &"<redacted>")
            .field("wire_len", &self.0.len())
            .finish()
    }
}

pub struct RecordProtector {
    transport: TransportState,
    limits: LimitProfile,
    generation: u64,
    send_sequence: SendSequence,
    receive_sequence: ReceiveSequence,
    replay_digests: VecDeque<[u8; 32]>,
}

impl RecordProtector {
    pub fn from_handshake(handshake: EstablishedHandshake) -> Self {
        Self {
            transport: handshake.transport,
            limits: handshake.limits,
            generation: handshake.generation,
            send_sequence: SendSequence::new(),
            receive_sequence: ReceiveSequence::new(),
            replay_digests: VecDeque::with_capacity(REPLAY_CACHE_ENTRIES),
        }
    }

    pub fn protect(
        &mut self,
        kind: RecordKind,
        channel: d2b_contracts::v2_component_session::ChannelId,
        payload: &[u8],
    ) -> Result<ProtectedRecord> {
        let payload_len = u32::try_from(payload.len())
            .map_err(|_| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        self.limits
            .checked_ciphertext_allocation(payload_len, RECORD_HEADER_LEN as u32)?;
        let sequence = self.send_sequence.take()?;
        let header = RecordHeader {
            kind,
            flags: 0,
            channel,
            sequence,
            reconnect_generation: self.generation,
            payload_len,
        };
        let header = header.encode(self.limits)?;
        let plaintext_len = header
            .len()
            .checked_add(payload.len())
            .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        let ciphertext_len = plaintext_len
            .checked_add(NOISE_TAG_BYTES as usize)
            .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        if ciphertext_len > self.limits.protected_ciphertext_bytes as usize {
            return Err(SessionError::new(SessionErrorCode::QueueBackpressure));
        }
        let mut plaintext = Vec::with_capacity(plaintext_len);
        plaintext.extend_from_slice(&header);
        plaintext.extend_from_slice(payload);
        let mut wire = vec![0_u8; RECORD_LENGTH_BYTES as usize + ciphertext_len];
        let written = self
            .transport
            .write_message(&plaintext, &mut wire[RECORD_LENGTH_BYTES as usize..])
            .map_err(|_| SessionError::new(SessionErrorCode::AuthenticationFailed))?;
        let encoded_len = u16::try_from(written)
            .map_err(|_| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        wire[..2].copy_from_slice(&encoded_len.to_be_bytes());
        wire.truncate(RECORD_LENGTH_BYTES as usize + written);
        Ok(ProtectedRecord(wire))
    }

    pub fn unprotect(&mut self, wire: &[u8]) -> Result<(RecordHeader, Vec<u8>)> {
        if wire.len() < RECORD_LENGTH_BYTES as usize {
            return Err(SessionError::new(SessionErrorCode::RecordTruncated));
        }
        let declared = usize::from(u16::from_be_bytes([wire[0], wire[1]]));
        let ciphertext = &wire[RECORD_LENGTH_BYTES as usize..];
        if declared != ciphertext.len() {
            return Err(SessionError::new(SessionErrorCode::RecordTruncated));
        }
        if declared > self.limits.protected_ciphertext_bytes as usize {
            return Err(SessionError::new(SessionErrorCode::RecordMalformed));
        }
        let digest: [u8; 32] = Sha256::digest(ciphertext).into();
        if self.replay_digests.contains(&digest) {
            return Err(SessionError::new(SessionErrorCode::RecordReplay));
        }

        let plaintext_limit = self.limits.protected_plaintext_bytes()? as usize;
        let mut plaintext = vec![0_u8; plaintext_limit];
        let read = self
            .transport
            .read_message(ciphertext, &mut plaintext)
            .map_err(|_| SessionError::new(SessionErrorCode::AuthenticationFailed))?;
        plaintext.truncate(read);
        if plaintext.len() < RECORD_HEADER_LEN {
            return Err(SessionError::new(SessionErrorCode::RecordMalformed));
        }
        let header = RecordHeader::decode(&plaintext[..RECORD_HEADER_LEN], self.limits)?;
        if header.reconnect_generation != self.generation {
            return Err(SessionError::new(SessionErrorCode::GenerationMismatch));
        }
        let payload = &plaintext[RECORD_HEADER_LEN..];
        if usize::try_from(header.payload_len).ok() != Some(payload.len()) {
            return Err(SessionError::new(SessionErrorCode::RecordMalformed));
        }
        self.receive_sequence.accept(header.sequence)?;
        if self.replay_digests.len() == REPLAY_CACHE_ENTRIES {
            self.replay_digests.pop_front();
        }
        self.replay_digests.push_back(digest);
        Ok((header, payload.to_vec()))
    }
}

impl fmt::Debug for RecordProtector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecordProtector")
            .field("generation", &"<redacted>")
            .field("cryptographic_state", &"<redacted>")
            .finish_non_exhaustive()
    }
}
