use std::fmt;

use d2b_contracts::v2_component_session::{
    FRAGMENT_HEADER_LEN, FragmentHeader, FragmentSequence, LimitProfile, RECORD_HEADER_LEN,
    SessionErrorCode,
};

use crate::{Result, SessionError};

pub struct Fragment {
    pub header: FragmentHeader,
    bytes: Vec<u8>,
}

impl Fragment {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn from_parts(header: FragmentHeader, bytes: Vec<u8>) -> Self {
        Self { header, bytes }
    }
}

impl fmt::Debug for Fragment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Fragment")
            .field("message_id", &"<redacted>")
            .field("index", &self.header.index)
            .field("count", &self.header.count)
            .field("payload", &"<redacted>")
            .field("payload_len", &self.bytes.len())
            .finish()
    }
}

pub struct Fragmenter {
    max_fragment_bytes: usize,
    logical_limit: u32,
}

impl Fragmenter {
    pub fn new(limits: LimitProfile, logical_limit: u32) -> Result<Self> {
        let protected = usize::try_from(limits.protected_plaintext_bytes()?)
            .map_err(|_| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        let overhead = RECORD_HEADER_LEN
            .checked_add(FRAGMENT_HEADER_LEN)
            .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        let max_fragment_bytes = protected
            .checked_sub(overhead)
            .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        if max_fragment_bytes == 0 || logical_limit == 0 {
            return Err(SessionError::new(SessionErrorCode::ReassemblyLimitExceeded));
        }
        Ok(Self {
            max_fragment_bytes,
            logical_limit,
        })
    }

    pub fn fragment(&self, message_id: u64, message: &[u8]) -> Result<Vec<Fragment>> {
        if message_id == 0 || message.is_empty() || message.len() > self.logical_limit as usize {
            return Err(SessionError::new(SessionErrorCode::ReassemblyLimitExceeded));
        }
        let count = message
            .len()
            .checked_add(self.max_fragment_bytes - 1)
            .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?
            / self.max_fragment_bytes;
        let count = u32::try_from(count)
            .map_err(|_| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        let total_plaintext_len = u32::try_from(message.len())
            .map_err(|_| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        let mut fragments = Vec::with_capacity(count as usize);
        for (index, bytes) in message.chunks(self.max_fragment_bytes).enumerate() {
            let offset = index
                .checked_mul(self.max_fragment_bytes)
                .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
            let header = FragmentHeader {
                message_id,
                index: u32::try_from(index)
                    .map_err(|_| SessionError::new(SessionErrorCode::ArithmeticOverflow))?,
                count,
                total_plaintext_len,
                offset: u32::try_from(offset)
                    .map_err(|_| SessionError::new(SessionErrorCode::ArithmeticOverflow))?,
            };
            header.validate(bytes.len() as u32, self.logical_limit)?;
            fragments.push(Fragment {
                header,
                bytes: bytes.to_vec(),
            });
        }
        Ok(fragments)
    }
}

pub struct Reassembler {
    logical_limit: u32,
    active: Option<ActiveReassembly>,
}

struct ActiveReassembly {
    sequence: FragmentSequence,
    bytes: Vec<u8>,
    total: usize,
}

impl Reassembler {
    pub fn new(logical_limit: u32) -> Result<Self> {
        if logical_limit == 0 {
            return Err(SessionError::new(SessionErrorCode::ReassemblyLimitExceeded));
        }
        Ok(Self {
            logical_limit,
            active: None,
        })
    }

    pub fn accept(&mut self, fragment: Fragment) -> Result<Option<Vec<u8>>> {
        let fragment_len = u32::try_from(fragment.bytes.len())
            .map_err(|_| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        fragment.header.validate(fragment_len, self.logical_limit)?;
        if self.active.is_none() {
            if fragment.header.index != 0 || fragment.header.offset != 0 {
                return Err(SessionError::new(SessionErrorCode::FragmentReordered));
            }
            if fragment.header.count == 1 {
                return Ok(Some(fragment.bytes));
            }
            let total = usize::try_from(fragment.header.total_plaintext_len)
                .map_err(|_| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
            let sequence =
                FragmentSequence::begin(fragment.header, fragment_len, self.logical_limit)?;
            let mut bytes = Vec::with_capacity(total);
            bytes.extend_from_slice(&fragment.bytes);
            self.active = Some(ActiveReassembly {
                sequence,
                bytes,
                total,
            });
            return Ok(None);
        }

        let active = self
            .active
            .as_mut()
            .ok_or_else(|| SessionError::new(SessionErrorCode::InternalInvariant))?;
        let complete = active
            .sequence
            .accept(fragment.header, fragment_len, self.logical_limit)?;
        let next_len = active
            .bytes
            .len()
            .checked_add(fragment.bytes.len())
            .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        if next_len > active.total {
            return Err(SessionError::new(SessionErrorCode::ReassemblyLimitExceeded));
        }
        active.bytes.extend_from_slice(&fragment.bytes);
        if complete {
            let complete = self
                .active
                .take()
                .ok_or_else(|| SessionError::new(SessionErrorCode::InternalInvariant))?;
            if complete.bytes.len() != complete.total {
                return Err(SessionError::new(SessionErrorCode::FragmentTruncated));
            }
            Ok(Some(complete.bytes))
        } else {
            Ok(None)
        }
    }

    pub fn abort(&mut self) {
        self.active = None;
    }
}
