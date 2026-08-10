//! Guest/relay CTAPHID CID translation.

use core::fmt;

/// Guest-originated CTAPHID channel identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GuestCid(u32);

impl GuestCid {
    /// Construct a non-broadcast Guest CID.
    pub const fn new(value: u32) -> Result<Self, CidTranslationError> {
        if value == 0 || value == u32::MAX {
            Err(CidTranslationError::ReservedCid)
        } else {
            Ok(Self(value))
        }
    }

    /// Return the wire value.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Relay-local CTAPHID channel identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RelayCid(u32);

impl RelayCid {
    /// Return the wire value.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Reversible per-session CID translator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityKeyCidTranslator {
    mask: u32,
}

impl SecurityKeyCidTranslator {
    /// Construct a translator from a Core-provided nonzero session mask.
    pub const fn from_core(mask: u32) -> Result<Self, CidTranslationError> {
        if mask == 0 || mask == u32::MAX {
            Err(CidTranslationError::ReservedCid)
        } else {
            Ok(Self { mask })
        }
    }

    /// Translate a Guest CID to a relay CID.
    pub const fn to_relay(self, cid: GuestCid) -> RelayCid {
        RelayCid(cid.0 ^ self.mask)
    }

    /// Translate a relay CID back to the Guest CID.
    pub const fn to_guest(self, cid: RelayCid) -> Result<GuestCid, CidTranslationError> {
        GuestCid::new(cid.0 ^ self.mask)
    }
}

/// CID translation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CidTranslationError {
    /// Zero and broadcast CIDs are not admitted.
    ReservedCid,
}

impl fmt::Display for CidTranslationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("security-key-cid-reserved")
    }
}

impl std::error::Error for CidTranslationError {}
