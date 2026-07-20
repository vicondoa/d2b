//! Canonical broker-to-guest configured-launch catalog.

use crate::v2_identity::{RealmId, WorkloadId};
use d2b_core::configured_argv::{
    ConfiguredArgv, MAX_CONFIGURED_ARG_BYTES, MAX_CONFIGURED_ARG_LEN, MAX_CONFIGURED_ARGC,
};
use d2b_realm_core::{ProtocolToken, token::MAX_PROTOCOL_TOKEN_LEN};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    error::Error,
    fmt,
    io::{self, Write},
};
use zeroize::{Zeroize, Zeroizing};

pub const GUEST_CONFIGURED_LAUNCHES_MAGIC: [u8; 8] = *b"D2BCLV2\0";
pub const GUEST_CONFIGURED_LAUNCHES_SCHEMA_VERSION: u16 = 1;
pub const GUEST_CONFIGURED_LAUNCHES_CODEC_VERSION: u16 = 1;
pub const GUEST_CONFIGURED_LAUNCHES_HEADER_BYTES: usize = 96;
pub const MAX_GUEST_CONFIGURED_LAUNCH_ITEMS: usize = 64;
pub const MAX_GUEST_CONFIGURED_LAUNCHES_BYTES: usize = 2 * 1024 * 1024;

const ENTRY_FLAG_GRAPHICAL: u16 = 1;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GuestConfiguredLaunchesError {
    Truncated,
    TrailingBytes,
    InvalidMagic,
    UnsupportedSchema,
    UnsupportedVersion,
    InvalidFlags,
    InvalidReserved,
    LengthExceeded,
    InvalidIdentity,
    InvalidDigest,
    InvalidCount,
    InvalidItemId,
    DuplicateItemId,
    InvalidArgv,
    InvalidUtf8,
}

impl GuestConfiguredLaunchesError {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Truncated => "guest-configured-launches-truncated",
            Self::TrailingBytes => "guest-configured-launches-trailing-bytes",
            Self::InvalidMagic => "guest-configured-launches-invalid-magic",
            Self::UnsupportedSchema => "guest-configured-launches-unsupported-schema",
            Self::UnsupportedVersion => "guest-configured-launches-unsupported-version",
            Self::InvalidFlags => "guest-configured-launches-invalid-flags",
            Self::InvalidReserved => "guest-configured-launches-invalid-reserved",
            Self::LengthExceeded => "guest-configured-launches-length-exceeded",
            Self::InvalidIdentity => "guest-configured-launches-invalid-identity",
            Self::InvalidDigest => "guest-configured-launches-invalid-digest",
            Self::InvalidCount => "guest-configured-launches-invalid-count",
            Self::InvalidItemId => "guest-configured-launches-invalid-item-id",
            Self::DuplicateItemId => "guest-configured-launches-duplicate-item-id",
            Self::InvalidArgv => "guest-configured-launches-invalid-argv",
            Self::InvalidUtf8 => "guest-configured-launches-invalid-utf8",
        }
    }
}

impl fmt::Debug for GuestConfiguredLaunchesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Display for GuestConfiguredLaunchesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Error for GuestConfiguredLaunchesError {}

/// One configured item in the workload-bound catalog.
///
/// ```compile_fail
/// use d2b_contracts::v2_guest_configured_launches::GuestConfiguredLaunchEntryV1;
/// fn requires_clone<T: Clone>() {}
/// requires_clone::<GuestConfiguredLaunchEntryV1>();
/// ```
///
/// ```compile_fail
/// use d2b_contracts::v2_guest_configured_launches::GuestConfiguredLaunchEntryV1;
/// fn requires_serialize<T: serde::Serialize>() {}
/// requires_serialize::<GuestConfiguredLaunchEntryV1>();
/// ```
pub struct GuestConfiguredLaunchEntryV1 {
    item_id: ProtocolToken,
    argv: ConfiguredArgv,
    graphical: bool,
}

impl GuestConfiguredLaunchEntryV1 {
    pub fn new(
        item_id: ProtocolToken,
        argv: ConfiguredArgv,
        graphical: bool,
    ) -> Result<Self, GuestConfiguredLaunchesError> {
        let value = Self {
            item_id,
            argv,
            graphical,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn item_id(&self) -> &ProtocolToken {
        &self.item_id
    }

    pub fn argv(&self) -> &ConfiguredArgv {
        &self.argv
    }

    pub const fn graphical(&self) -> bool {
        self.graphical
    }

    fn validate(&self) -> Result<(), GuestConfiguredLaunchesError> {
        let item_id = self.item_id.as_str();
        if item_id.is_empty()
            || item_id.len() > MAX_PROTOCOL_TOKEN_LEN
            || item_id.as_bytes().contains(&0)
        {
            return Err(GuestConfiguredLaunchesError::InvalidItemId);
        }
        validate_argv(self.argv.as_slice())
    }
}

impl fmt::Debug for GuestConfiguredLaunchEntryV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GuestConfiguredLaunchEntryV1(REDACTED)")
    }
}

/// One workload's private configured-launch catalog.
///
/// ```compile_fail
/// use d2b_contracts::v2_guest_configured_launches::GuestConfiguredLaunchesV1;
/// fn requires_clone<T: Clone>() {}
/// requires_clone::<GuestConfiguredLaunchesV1>();
/// ```
///
/// ```compile_fail
/// use d2b_contracts::v2_guest_configured_launches::GuestConfiguredLaunchesV1;
/// fn requires_serialize<T: serde::Serialize>() {}
/// requires_serialize::<GuestConfiguredLaunchesV1>();
/// ```
pub struct GuestConfiguredLaunchesV1 {
    schema_version: u16,
    codec_version: u16,
    realm_id: RealmId,
    workload_id: WorkloadId,
    workload_digest: [u8; 32],
    entries: Vec<GuestConfiguredLaunchEntryV1>,
}

impl GuestConfiguredLaunchesV1 {
    pub fn new(
        realm_id: RealmId,
        workload_id: WorkloadId,
        workload_digest: [u8; 32],
        entries: Vec<GuestConfiguredLaunchEntryV1>,
    ) -> Result<Self, GuestConfiguredLaunchesError> {
        let value = Self {
            schema_version: GUEST_CONFIGURED_LAUNCHES_SCHEMA_VERSION,
            codec_version: GUEST_CONFIGURED_LAUNCHES_CODEC_VERSION,
            realm_id,
            workload_id,
            workload_digest,
            entries,
        };
        value.validate()?;
        Ok(value)
    }

    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    pub const fn codec_version(&self) -> u16 {
        self.codec_version
    }

    pub fn realm_id(&self) -> &RealmId {
        &self.realm_id
    }

    pub fn workload_id(&self) -> &WorkloadId {
        &self.workload_id
    }

    pub const fn workload_digest(&self) -> &[u8; 32] {
        &self.workload_digest
    }

    pub fn entries(&self) -> &[GuestConfiguredLaunchEntryV1] {
        &self.entries
    }

    pub fn resolve(&self, item_id: &ProtocolToken) -> Option<&GuestConfiguredLaunchEntryV1> {
        self.entries.iter().find(|entry| entry.item_id() == item_id)
    }

    pub fn resolve_id(&self, item_id: &str) -> Option<&GuestConfiguredLaunchEntryV1> {
        self.entries
            .iter()
            .find(|entry| entry.item_id().as_str() == item_id)
    }

    pub fn encode(&self) -> Result<GuestConfiguredLaunchesBytes, GuestConfiguredLaunchesError> {
        self.validate()?;
        let mut entry_lengths = Vec::with_capacity(self.entries.len());
        let mut total_bytes = GUEST_CONFIGURED_LAUNCHES_HEADER_BYTES;
        for entry in &self.entries {
            let mut entry_bytes = 8usize
                .checked_add(entry.item_id.as_str().len())
                .ok_or(GuestConfiguredLaunchesError::LengthExceeded)?;
            for argument in entry.argv.as_slice() {
                entry_bytes = entry_bytes
                    .checked_add(2)
                    .and_then(|value| value.checked_add(argument.len()))
                    .ok_or(GuestConfiguredLaunchesError::LengthExceeded)?;
            }
            total_bytes = total_bytes
                .checked_add(4)
                .and_then(|value| value.checked_add(entry_bytes))
                .ok_or(GuestConfiguredLaunchesError::LengthExceeded)?;
            entry_lengths.push(entry_bytes);
        }
        if total_bytes > MAX_GUEST_CONFIGURED_LAUNCHES_BYTES {
            return Err(GuestConfiguredLaunchesError::LengthExceeded);
        }

        let mut writer = ConfiguredLaunchWriter::with_capacity(total_bytes);
        writer.bytes(&GUEST_CONFIGURED_LAUNCHES_MAGIC);
        writer.u16(self.schema_version);
        writer.u16(self.codec_version);
        writer.u16(0);
        writer.u16(0);
        writer.u32(
            u32::try_from(total_bytes).map_err(|_| GuestConfiguredLaunchesError::LengthExceeded)?,
        );
        writer.bytes(self.realm_id.as_str().as_bytes());
        writer.bytes(self.workload_id.as_str().as_bytes());
        writer.bytes(&self.workload_digest);
        writer.u16(
            u16::try_from(self.entries.len())
                .map_err(|_| GuestConfiguredLaunchesError::LengthExceeded)?,
        );
        writer.u16(0);

        for (entry, entry_bytes) in self.entries.iter().zip(entry_lengths) {
            writer.u32(
                u32::try_from(entry_bytes)
                    .map_err(|_| GuestConfiguredLaunchesError::LengthExceeded)?,
            );
            writer.u16(
                u16::try_from(entry.item_id.as_str().len())
                    .map_err(|_| GuestConfiguredLaunchesError::LengthExceeded)?,
            );
            writer.bytes(entry.item_id.as_str().as_bytes());
            writer.u16(if entry.graphical {
                ENTRY_FLAG_GRAPHICAL
            } else {
                0
            });
            writer.u16(
                u16::try_from(entry.argv.as_slice().len())
                    .map_err(|_| GuestConfiguredLaunchesError::LengthExceeded)?,
            );
            writer.u16(0);
            for argument in entry.argv.as_slice() {
                writer.u16(
                    u16::try_from(argument.len())
                        .map_err(|_| GuestConfiguredLaunchesError::LengthExceeded)?,
                );
                writer.bytes(argument.as_bytes());
            }
        }
        if writer.len() != total_bytes {
            return Err(GuestConfiguredLaunchesError::LengthExceeded);
        }
        Ok(writer.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, GuestConfiguredLaunchesError> {
        if bytes.len() > MAX_GUEST_CONFIGURED_LAUNCHES_BYTES {
            return Err(GuestConfiguredLaunchesError::LengthExceeded);
        }
        let mut reader = ConfiguredLaunchReader::new(bytes);
        let magic = reader.array::<8>()?;
        if magic != GUEST_CONFIGURED_LAUNCHES_MAGIC {
            return Err(GuestConfiguredLaunchesError::InvalidMagic);
        }
        let schema_version = reader.u16()?;
        if schema_version != GUEST_CONFIGURED_LAUNCHES_SCHEMA_VERSION {
            return Err(GuestConfiguredLaunchesError::UnsupportedSchema);
        }
        let codec_version = reader.u16()?;
        if codec_version != GUEST_CONFIGURED_LAUNCHES_CODEC_VERSION {
            return Err(GuestConfiguredLaunchesError::UnsupportedVersion);
        }
        if reader.u16()? != 0 {
            return Err(GuestConfiguredLaunchesError::InvalidFlags);
        }
        if reader.u16()? != 0 {
            return Err(GuestConfiguredLaunchesError::InvalidReserved);
        }
        let declared_bytes = usize::try_from(reader.u32()?)
            .map_err(|_| GuestConfiguredLaunchesError::LengthExceeded)?;
        if declared_bytes > MAX_GUEST_CONFIGURED_LAUNCHES_BYTES {
            return Err(GuestConfiguredLaunchesError::LengthExceeded);
        }
        if declared_bytes > bytes.len() {
            return Err(GuestConfiguredLaunchesError::Truncated);
        }
        if declared_bytes < bytes.len() {
            return Err(GuestConfiguredLaunchesError::TrailingBytes);
        }

        let realm_id = parse_realm_id(reader.take(20)?)?;
        let workload_id = parse_workload_id(reader.take(20)?)?;
        let workload_digest = reader.array::<32>()?;
        let entry_count = usize::from(reader.u16()?);
        if entry_count == 0 || entry_count > MAX_GUEST_CONFIGURED_LAUNCH_ITEMS {
            return Err(GuestConfiguredLaunchesError::InvalidCount);
        }
        if reader.u16()? != 0 {
            return Err(GuestConfiguredLaunchesError::InvalidReserved);
        }

        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            let entry_bytes = usize::try_from(reader.u32()?)
                .map_err(|_| GuestConfiguredLaunchesError::LengthExceeded)?;
            let encoded_entry = reader.take(entry_bytes)?;
            let mut entry_reader = ConfiguredLaunchReader::new(encoded_entry);
            let item_id_bytes = usize::from(entry_reader.u16()?);
            if item_id_bytes == 0 || item_id_bytes > MAX_PROTOCOL_TOKEN_LEN {
                return Err(GuestConfiguredLaunchesError::InvalidItemId);
            }
            let item_id = parse_item_id(entry_reader.take(item_id_bytes)?)?;
            let flags = entry_reader.u16()?;
            if flags & !ENTRY_FLAG_GRAPHICAL != 0 {
                return Err(GuestConfiguredLaunchesError::InvalidFlags);
            }
            let argument_count = usize::from(entry_reader.u16()?);
            if argument_count == 0 || argument_count > MAX_CONFIGURED_ARGC {
                return Err(GuestConfiguredLaunchesError::InvalidArgv);
            }
            if entry_reader.u16()? != 0 {
                return Err(GuestConfiguredLaunchesError::InvalidReserved);
            }
            let mut argv = Vec::with_capacity(argument_count);
            let mut argument_bytes = 0usize;
            for index in 0..argument_count {
                let argument_len = usize::from(entry_reader.u16()?);
                if argument_len > MAX_CONFIGURED_ARG_LEN || (index == 0 && argument_len == 0) {
                    return Err(GuestConfiguredLaunchesError::InvalidArgv);
                }
                argument_bytes = argument_bytes
                    .checked_add(argument_len)
                    .ok_or(GuestConfiguredLaunchesError::LengthExceeded)?;
                if argument_bytes > MAX_CONFIGURED_ARG_BYTES {
                    return Err(GuestConfiguredLaunchesError::InvalidArgv);
                }
                argv.push(parse_argument(entry_reader.take(argument_len)?)?);
            }
            entry_reader.finish()?;
            let argv =
                ConfiguredArgv::new(argv).map_err(|_| GuestConfiguredLaunchesError::InvalidArgv)?;
            entries.push(GuestConfiguredLaunchEntryV1::new(
                item_id,
                argv,
                flags & ENTRY_FLAG_GRAPHICAL != 0,
            )?);
        }
        reader.finish()?;
        let value = Self {
            schema_version,
            codec_version,
            realm_id,
            workload_id,
            workload_digest,
            entries,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), GuestConfiguredLaunchesError> {
        if self.schema_version != GUEST_CONFIGURED_LAUNCHES_SCHEMA_VERSION {
            return Err(GuestConfiguredLaunchesError::UnsupportedSchema);
        }
        if self.codec_version != GUEST_CONFIGURED_LAUNCHES_CODEC_VERSION {
            return Err(GuestConfiguredLaunchesError::UnsupportedVersion);
        }
        if self.realm_id.as_str().len() != 20 || self.workload_id.as_str().len() != 20 {
            return Err(GuestConfiguredLaunchesError::InvalidIdentity);
        }
        if self.workload_digest == [0; 32] {
            return Err(GuestConfiguredLaunchesError::InvalidDigest);
        }
        if self.entries.is_empty() || self.entries.len() > MAX_GUEST_CONFIGURED_LAUNCH_ITEMS {
            return Err(GuestConfiguredLaunchesError::InvalidCount);
        }
        let mut ids = BTreeSet::new();
        for entry in &self.entries {
            entry.validate()?;
            if !ids.insert(entry.item_id.as_str()) {
                return Err(GuestConfiguredLaunchesError::DuplicateItemId);
            }
        }
        Ok(())
    }
}

impl fmt::Debug for GuestConfiguredLaunchesV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GuestConfiguredLaunchesV1(REDACTED)")
    }
}

/// Opaque encoded configured-launch bytes.
///
/// ```compile_fail
/// use d2b_contracts::v2_guest_configured_launches::GuestConfiguredLaunchesBytes;
/// fn requires_clone<T: Clone>() {}
/// requires_clone::<GuestConfiguredLaunchesBytes>();
/// ```
///
/// ```compile_fail
/// use d2b_contracts::v2_guest_configured_launches::GuestConfiguredLaunchesBytes;
/// fn requires_serialize<T: serde::Serialize>() {}
/// requires_serialize::<GuestConfiguredLaunchesBytes>();
/// ```
pub struct GuestConfiguredLaunchesBytes {
    bytes: Zeroizing<Vec<u8>>,
}

impl GuestConfiguredLaunchesBytes {
    fn new(bytes: Zeroizing<Vec<u8>>) -> Self {
        Self { bytes }
    }

    pub fn as_slice(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    pub fn write_to(&self, writer: &mut impl Write) -> io::Result<()> {
        writer.write_all(self.as_slice())
    }

    pub fn sha256(&self) -> [u8; 32] {
        Sha256::digest(self.as_slice()).into()
    }
}

impl fmt::Debug for GuestConfiguredLaunchesBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GuestConfiguredLaunchesBytes(REDACTED)")
    }
}

impl Drop for GuestConfiguredLaunchesBytes {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

fn validate_argv(argv: &[String]) -> Result<(), GuestConfiguredLaunchesError> {
    if argv.is_empty() || argv.len() > MAX_CONFIGURED_ARGC || argv[0].is_empty() {
        return Err(GuestConfiguredLaunchesError::InvalidArgv);
    }
    let mut bytes = 0usize;
    for argument in argv {
        if argument.len() > MAX_CONFIGURED_ARG_LEN || argument.as_bytes().contains(&0) {
            return Err(GuestConfiguredLaunchesError::InvalidArgv);
        }
        bytes = bytes
            .checked_add(argument.len())
            .ok_or(GuestConfiguredLaunchesError::LengthExceeded)?;
    }
    if bytes > MAX_CONFIGURED_ARG_BYTES {
        return Err(GuestConfiguredLaunchesError::InvalidArgv);
    }
    Ok(())
}

fn parse_realm_id(bytes: &[u8]) -> Result<RealmId, GuestConfiguredLaunchesError> {
    let value =
        std::str::from_utf8(bytes).map_err(|_| GuestConfiguredLaunchesError::InvalidUtf8)?;
    RealmId::parse(value).map_err(|_| GuestConfiguredLaunchesError::InvalidIdentity)
}

fn parse_workload_id(bytes: &[u8]) -> Result<WorkloadId, GuestConfiguredLaunchesError> {
    let value =
        std::str::from_utf8(bytes).map_err(|_| GuestConfiguredLaunchesError::InvalidUtf8)?;
    WorkloadId::parse(value).map_err(|_| GuestConfiguredLaunchesError::InvalidIdentity)
}

fn parse_item_id(bytes: &[u8]) -> Result<ProtocolToken, GuestConfiguredLaunchesError> {
    let value =
        std::str::from_utf8(bytes).map_err(|_| GuestConfiguredLaunchesError::InvalidUtf8)?;
    ProtocolToken::parse(value).map_err(|_| GuestConfiguredLaunchesError::InvalidItemId)
}

fn parse_argument(bytes: &[u8]) -> Result<String, GuestConfiguredLaunchesError> {
    if bytes.contains(&0) {
        return Err(GuestConfiguredLaunchesError::InvalidArgv);
    }
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| GuestConfiguredLaunchesError::InvalidUtf8)
}

struct ConfiguredLaunchWriter {
    bytes: Zeroizing<Vec<u8>>,
}

impl ConfiguredLaunchWriter {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: Zeroizing::new(Vec::with_capacity(capacity)),
        }
    }

    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }

    fn finish(self) -> GuestConfiguredLaunchesBytes {
        GuestConfiguredLaunchesBytes::new(self.bytes)
    }
}

struct ConfiguredLaunchReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> ConfiguredLaunchReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], GuestConfiguredLaunchesError> {
        let end = self
            .cursor
            .checked_add(len)
            .ok_or(GuestConfiguredLaunchesError::LengthExceeded)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or(GuestConfiguredLaunchesError::Truncated)?;
        self.cursor = end;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, GuestConfiguredLaunchesError> {
        Ok(u16::from_be_bytes(
            self.take(2)?.try_into().expect("fixed slice"),
        ))
    }

    fn u32(&mut self) -> Result<u32, GuestConfiguredLaunchesError> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().expect("fixed slice"),
        ))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], GuestConfiguredLaunchesError> {
        Ok(self.take(N)?.try_into().expect("fixed slice"))
    }

    fn finish(self) -> Result<(), GuestConfiguredLaunchesError> {
        if self.cursor == self.bytes.len() {
            Ok(())
        } else {
            Err(GuestConfiguredLaunchesError::TrailingBytes)
        }
    }
}
