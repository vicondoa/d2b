use std::{
    collections::BTreeSet,
    fmt,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::fd::{AsFd, OwnedFd},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
};

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::{
    guest_material_store::{EnrollmentSuccessAudit, EnrollmentSuccessAuditIdentity},
    guest_session_material::{
        GuestMaterialAuditRecord, GuestMaterialAuditSink, GuestMaterialError, GuestMaterialOutcome,
    },
};

const V3_MAGIC: &[u8; 8] = b"D2BGMA3\0";
const V3_COMMIT_MAGIC: &[u8; 8] = b"D2BGMC3\0";
const V3_PAYLOAD_BYTES: usize = 1 + 8 + 32 * 5;
const V3_PHASE_BYTES: usize = 8 + 4 + V3_PAYLOAD_BYTES + 32;
const V3_TRAILER_BYTES: usize = 8 + 32;
const V3_FRAME_BYTES: usize = V3_PHASE_BYTES + V3_TRAILER_BYTES;
const MAX_AUDIT_BYTES: u64 = 64 * 1024 * 1024;
const SEGMENT_MAX_BYTES: u64 = (MAX_AUDIT_BYTES / V3_FRAME_BYTES as u64) * V3_FRAME_BYTES as u64;
const MAX_RETAINED_SEGMENTS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
struct AuditRecordData {
    outcome: u8,
    session_generation: u64,
    request_digest: [u8; 32],
    credential_digest: [u8; 32],
    configured_digest: [u8; 32],
    error_digest: [u8; 32],
    dedup_key: [u8; 32],
}

struct AuditState {
    file: File,
    path: PathBuf,
    parent_fd: OwnedFd,
    segment_limit: u64,
    next_segment: u64,
    success_keys: BTreeSet<[u8; 32]>,
}

pub struct FileGuestMaterialAuditSink {
    state: Mutex<AuditState>,
}

impl FileGuestMaterialAuditSink {
    pub fn open(
        path: PathBuf,
        expected_uid: u32,
        expected_gid: u32,
    ) -> Result<Self, GuestMaterialError> {
        Self::open_inner(path, expected_uid, expected_gid, SEGMENT_MAX_BYTES)
    }

    #[cfg(test)]
    fn open_with_limit(
        path: PathBuf,
        expected_uid: u32,
        expected_gid: u32,
        segment_limit: u64,
    ) -> Result<Self, GuestMaterialError> {
        Self::open_inner(path, expected_uid, expected_gid, segment_limit)
    }

    fn open_inner(
        path: PathBuf,
        expected_uid: u32,
        expected_gid: u32,
        segment_limit: u64,
    ) -> Result<Self, GuestMaterialError> {
        if segment_limit < V3_FRAME_BYTES as u64
            || segment_limit > MAX_AUDIT_BYTES
            || !segment_limit.is_multiple_of(V3_FRAME_BYTES as u64)
        {
            return Err(GuestMaterialError::StorageContractMismatch);
        }
        let parent = path
            .parent()
            .ok_or(GuestMaterialError::StorageContractMismatch)?;
        let parent_fd = crate::sys::path_safe::open_dir_path_safe(parent)
            .map_err(|_| GuestMaterialError::AuditFailed)?;
        let parent_stat =
            rustix::fs::fstat(&parent_fd).map_err(|_| GuestMaterialError::AuditFailed)?;
        if parent_stat.st_uid != expected_uid || parent_stat.st_mode & 0o022 != 0 {
            return Err(GuestMaterialError::StorageContractMismatch);
        }
        let mut success_keys = prepare_v3_path(&path, &parent_fd)?;
        let segments = segment_paths(&path)?;
        for (_, segment) in &segments {
            let encoded = read_bounded_file(segment, segment_limit, expected_uid, expected_gid)?;
            let parsed = parse_v3(&encoded)?;
            if parsed.truncate_to != encoded.len() {
                return Err(GuestMaterialError::AuditFailed);
            }
            success_keys.extend(parsed.success_keys);
        }
        let mut file = open_audit_file(&path)?;
        let metadata = file
            .metadata()
            .map_err(|_| GuestMaterialError::AuditFailed)?;
        if !metadata.is_file()
            || metadata.uid() != expected_uid
            || metadata.gid() != expected_gid
            || metadata.mode() & 0o777 != 0o600
            || metadata.len() > segment_limit
        {
            return Err(GuestMaterialError::StorageContractMismatch);
        }
        file.seek(SeekFrom::Start(0))
            .map_err(|_| GuestMaterialError::AuditFailed)?;
        let mut encoded = Zeroizing::new(Vec::with_capacity(metadata.len() as usize));
        Read::by_ref(&mut file)
            .take(MAX_AUDIT_BYTES + 1)
            .read_to_end(&mut encoded)
            .map_err(|_| GuestMaterialError::AuditFailed)?;
        if encoded.len() as u64 > MAX_AUDIT_BYTES {
            return Err(GuestMaterialError::AuditFailed);
        }
        let parsed = parse_v3(&encoded)?;
        if parsed.truncate_to != encoded.len() {
            file.set_len(parsed.truncate_to as u64)
                .and_then(|()| file.sync_data())
                .map_err(|_| GuestMaterialError::AuditFailed)?;
        }
        success_keys.extend(parsed.success_keys);
        file.sync_all()
            .map_err(|_| GuestMaterialError::AuditFailed)?;
        rustix::fs::fsync(parent_fd.as_fd()).map_err(|_| GuestMaterialError::AuditFailed)?;
        prune_segments(&parent_fd, &segments)?;
        let next_segment = segments
            .last()
            .map(|(index, _)| index.saturating_add(1))
            .unwrap_or(0);
        Ok(Self {
            state: Mutex::new(AuditState {
                file,
                path,
                parent_fd,
                segment_limit,
                next_segment,
                success_keys,
            }),
        })
    }

    fn append_success(
        &self,
        identity: EnrollmentSuccessAuditIdentity,
    ) -> Result<(), GuestMaterialError> {
        identity.validate()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| GuestMaterialError::AuditFailed)?;
        if state.success_keys.contains(&identity.dedup_key) {
            return Ok(());
        }
        let record = AuditRecordData {
            outcome: identity.outcome,
            session_generation: identity.session_generation,
            request_digest: identity.request_digest,
            credential_digest: identity.credential_digest,
            configured_digest: identity.configured_digest,
            error_digest: [0; 32],
            dedup_key: identity.dedup_key,
        };
        append_rotating(&mut state, record)?;
        state.success_keys.insert(identity.dedup_key);
        Ok(())
    }
}

impl EnrollmentSuccessAudit for FileGuestMaterialAuditSink {
    fn ensure_success(
        &self,
        identity: EnrollmentSuccessAuditIdentity,
    ) -> Result<(), GuestMaterialError> {
        self.append_success(identity)
    }
}

impl GuestMaterialAuditSink for FileGuestMaterialAuditSink {
    fn record(&self, record: &GuestMaterialAuditRecord) -> Result<(), GuestMaterialError> {
        match record.outcome {
            GuestMaterialOutcome::Succeeded => {
                let credential_digest = record
                    .credential_digest
                    .ok_or(GuestMaterialError::AuditFailed)?;
                if record.error_kind.is_some() {
                    return Err(GuestMaterialError::AuditFailed);
                }
                self.append_success(EnrollmentSuccessAuditIdentity::new(
                    record.session_generation,
                    record.request_digest,
                    credential_digest,
                    record.configured_launch_digest,
                )?)
            }
            GuestMaterialOutcome::Failed => {
                let error_digest = record
                    .error_kind
                    .map(|kind| <[u8; 32]>::from(Sha256::digest(kind.as_bytes())))
                    .unwrap_or([0; 32]);
                let mut audit = AuditRecordData {
                    outcome: 2,
                    session_generation: record.session_generation,
                    request_digest: record.request_digest,
                    credential_digest: record.credential_digest.unwrap_or([0; 32]),
                    configured_digest: record.configured_launch_digest,
                    error_digest,
                    dedup_key: [0; 32],
                };
                audit.dedup_key = canonical_dedup(audit)?;
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| GuestMaterialError::AuditFailed)?;
                append_rotating(&mut state, audit)
            }
        }
    }
}

fn open_audit_file(path: &Path) -> Result<File, GuestMaterialError> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW);
    options
        .open(path)
        .map_err(|_| GuestMaterialError::AuditFailed)
}

fn prepare_v3_path(
    path: &Path,
    parent_fd: &OwnedFd,
) -> Result<BTreeSet<[u8; 32]>, GuestMaterialError> {
    match read_magic(path)? {
        None => ensure_empty_active(path, parent_fd)?,
        Some(magic) if magic.is_empty() => {}
        Some(magic) if magic.starts_with(V3_MAGIC) || V3_MAGIC.starts_with(magic.as_slice()) => {
            repair_active_v3(path)?;
        }
        Some(_) => return Err(GuestMaterialError::AuditFailed),
    }
    Ok(BTreeSet::new())
}

fn repair_active_v3(path: &Path) -> Result<(), GuestMaterialError> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| GuestMaterialError::AuditFailed)?;
    let length = file
        .metadata()
        .map_err(|_| GuestMaterialError::AuditFailed)?
        .len() as usize;
    let full_frames = length / V3_FRAME_BYTES;
    let mut frame = Zeroizing::new(vec![0_u8; V3_FRAME_BYTES]);
    for index in 0..full_frames {
        file.read_exact(&mut frame)
            .map_err(|_| GuestMaterialError::AuditFailed)?;
        let parsed = parse_v3(&frame)?;
        if parsed.truncate_to != frame.len() {
            if index + 1 != full_frames || !length.is_multiple_of(V3_FRAME_BYTES) {
                return Err(GuestMaterialError::AuditFailed);
            }
            file.set_len((index * V3_FRAME_BYTES) as u64)
                .and_then(|()| file.sync_data())
                .map_err(|_| GuestMaterialError::AuditFailed)?;
            return Ok(());
        }
    }
    let committed_len = full_frames * V3_FRAME_BYTES;
    if committed_len != length {
        file.set_len(committed_len as u64)
            .and_then(|()| file.sync_data())
            .map_err(|_| GuestMaterialError::AuditFailed)?;
    }
    Ok(())
}

fn ensure_empty_active(path: &Path, parent_fd: &OwnedFd) -> Result<(), GuestMaterialError> {
    if !path.exists() {
        let file = open_audit_file(path)?;
        file.sync_all()
            .map_err(|_| GuestMaterialError::AuditFailed)?;
        rustix::fs::fsync(parent_fd.as_fd()).map_err(|_| GuestMaterialError::AuditFailed)?;
    }
    Ok(())
}

fn read_magic(path: &Path) -> Result<Option<Zeroizing<Vec<u8>>>, GuestMaterialError> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(GuestMaterialError::AuditFailed),
    };
    let metadata = file
        .metadata()
        .map_err(|_| GuestMaterialError::AuditFailed)?;
    if !metadata.is_file() {
        return Err(GuestMaterialError::AuditFailed);
    }
    let mut magic = Zeroizing::new(Vec::with_capacity(8));
    file.take(8)
        .read_to_end(&mut magic)
        .map_err(|_| GuestMaterialError::AuditFailed)?;
    Ok(Some(magic))
}

fn audit_name(path: &Path) -> Result<&str, GuestMaterialError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && !name.contains('/'))
        .ok_or(GuestMaterialError::StorageContractMismatch)
}

fn remove_regular_file(path: &Path) -> Result<(), GuestMaterialError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            std::fs::remove_file(path).map_err(|_| GuestMaterialError::AuditFailed)
        }
        Ok(_) => Err(GuestMaterialError::AuditFailed),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(GuestMaterialError::AuditFailed),
    }
}

fn segment_paths(path: &Path) -> Result<Vec<(u64, PathBuf)>, GuestMaterialError> {
    let parent = path
        .parent()
        .ok_or(GuestMaterialError::StorageContractMismatch)?;
    let prefix = format!("{}.v3-segment-", audit_name(path)?);
    let mut segments = Vec::new();
    for entry in std::fs::read_dir(parent).map_err(|_| GuestMaterialError::AuditFailed)? {
        let entry = entry.map_err(|_| GuestMaterialError::AuditFailed)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| GuestMaterialError::AuditFailed)?;
        if !name.starts_with(&prefix) {
            continue;
        }
        let suffix = &name[prefix.len()..];
        if suffix.len() != 20 || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(GuestMaterialError::AuditFailed);
        }
        let index = suffix
            .parse::<u64>()
            .map_err(|_| GuestMaterialError::AuditFailed)?;
        let metadata =
            std::fs::symlink_metadata(entry.path()).map_err(|_| GuestMaterialError::AuditFailed)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(GuestMaterialError::AuditFailed);
        }
        segments.push((index, entry.path()));
    }
    segments.sort_by_key(|(index, _)| *index);
    if segments.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(GuestMaterialError::AuditFailed);
    }
    Ok(segments)
}

fn segment_path(path: &Path, index: u64) -> Result<PathBuf, GuestMaterialError> {
    Ok(path.with_file_name(format!("{}.v3-segment-{index:020}", audit_name(path)?)))
}

fn read_bounded_file(
    path: &Path,
    limit: u64,
    expected_uid: u32,
    expected_gid: u32,
) -> Result<Zeroizing<Vec<u8>>, GuestMaterialError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| GuestMaterialError::AuditFailed)?;
    let metadata = file
        .metadata()
        .map_err(|_| GuestMaterialError::AuditFailed)?;
    if !metadata.is_file()
        || metadata.uid() != expected_uid
        || metadata.gid() != expected_gid
        || metadata.mode() & 0o777 != 0o600
        || metadata.len() > limit
    {
        return Err(GuestMaterialError::AuditFailed);
    }
    let mut encoded = Zeroizing::new(Vec::with_capacity(metadata.len() as usize));
    file.take(limit + 1)
        .read_to_end(&mut encoded)
        .map_err(|_| GuestMaterialError::AuditFailed)?;
    if encoded.len() as u64 > limit {
        return Err(GuestMaterialError::AuditFailed);
    }
    Ok(encoded)
}

fn prune_segments(
    parent_fd: &OwnedFd,
    segments: &[(u64, PathBuf)],
) -> Result<(), GuestMaterialError> {
    let remove = segments.len().saturating_sub(MAX_RETAINED_SEGMENTS);
    for (_, segment) in segments.iter().take(remove) {
        remove_regular_file(segment)?;
        rustix::fs::fsync(parent_fd.as_fd()).map_err(|_| GuestMaterialError::AuditFailed)?;
    }
    Ok(())
}

fn append_rotating(
    state: &mut AuditState,
    record: AuditRecordData,
) -> Result<(), GuestMaterialError> {
    let current_len = state
        .file
        .metadata()
        .map_err(|_| GuestMaterialError::AuditFailed)?
        .len();
    if current_len
        .checked_add(V3_FRAME_BYTES as u64)
        .is_none_or(|next| next > state.segment_limit)
    {
        rotate_segment(state)?;
    }
    let current_len = state
        .file
        .metadata()
        .map_err(|_| GuestMaterialError::AuditFailed)?
        .len();
    if current_len + V3_FRAME_BYTES as u64 > state.segment_limit {
        return Err(GuestMaterialError::AuditFailed);
    }
    append_committed_frame(&mut state.file, record)
}

fn rotate_segment(state: &mut AuditState) -> Result<(), GuestMaterialError> {
    let current_len = state
        .file
        .metadata()
        .map_err(|_| GuestMaterialError::AuditFailed)?
        .len();
    if current_len == 0 || current_len > state.segment_limit {
        return Err(GuestMaterialError::AuditFailed);
    }
    state
        .file
        .sync_all()
        .map_err(|_| GuestMaterialError::AuditFailed)?;
    let segment = segment_path(&state.path, state.next_segment)?;
    if segment.exists() {
        return Err(GuestMaterialError::AuditFailed);
    }
    std::fs::rename(&state.path, &segment).map_err(|_| GuestMaterialError::AuditFailed)?;
    rustix::fs::fsync(state.parent_fd.as_fd()).map_err(|_| GuestMaterialError::AuditFailed)?;
    state.file = open_audit_file(&state.path)?;
    state
        .file
        .sync_all()
        .map_err(|_| GuestMaterialError::AuditFailed)?;
    rustix::fs::fsync(state.parent_fd.as_fd()).map_err(|_| GuestMaterialError::AuditFailed)?;
    state.next_segment = state
        .next_segment
        .checked_add(1)
        .ok_or(GuestMaterialError::AuditFailed)?;
    let segments = segment_paths(&state.path)?;
    prune_segments(&state.parent_fd, &segments)
}

fn append_committed_frame(
    file: &mut File,
    record: AuditRecordData,
) -> Result<(), GuestMaterialError> {
    validate_record(record)?;
    let payload = encode_payload(record);
    let checksum = frame_checksum(&payload);
    let mut phase = Vec::with_capacity(V3_PHASE_BYTES);
    phase.extend_from_slice(V3_MAGIC);
    phase.extend_from_slice(&(V3_PAYLOAD_BYTES as u32).to_be_bytes());
    phase.extend_from_slice(&payload);
    phase.extend_from_slice(&checksum);
    let prior_len = file
        .metadata()
        .map_err(|_| GuestMaterialError::AuditFailed)?
        .len();
    if file
        .write_all(&phase)
        .and_then(|()| file.sync_data())
        .is_err()
    {
        truncate_failed_append(file, prior_len)?;
        return Err(GuestMaterialError::AuditFailed);
    }
    let mut trailer = Vec::with_capacity(V3_TRAILER_BYTES);
    trailer.extend_from_slice(V3_COMMIT_MAGIC);
    trailer.extend_from_slice(&checksum);
    if file
        .write_all(&trailer)
        .and_then(|()| file.sync_data())
        .is_err()
    {
        truncate_failed_append(file, prior_len)?;
        return Err(GuestMaterialError::AuditFailed);
    }
    Ok(())
}

fn truncate_failed_append(file: &File, prior_len: u64) -> Result<(), GuestMaterialError> {
    file.set_len(prior_len)
        .and_then(|()| file.sync_data())
        .map_err(|_| GuestMaterialError::AuditFailed)
}

fn encode_payload(record: AuditRecordData) -> Vec<u8> {
    let mut payload = Vec::with_capacity(V3_PAYLOAD_BYTES);
    payload.push(record.outcome);
    payload.extend_from_slice(&record.session_generation.to_be_bytes());
    payload.extend_from_slice(&record.request_digest);
    payload.extend_from_slice(&record.credential_digest);
    payload.extend_from_slice(&record.configured_digest);
    payload.extend_from_slice(&record.error_digest);
    payload.extend_from_slice(&record.dedup_key);
    payload
}

fn decode_payload(payload: &[u8]) -> Result<AuditRecordData, GuestMaterialError> {
    if payload.len() != V3_PAYLOAD_BYTES {
        return Err(GuestMaterialError::AuditFailed);
    }
    let record = AuditRecordData {
        outcome: payload[0],
        session_generation: u64::from_be_bytes(
            payload[1..9]
                .try_into()
                .map_err(|_| GuestMaterialError::AuditFailed)?,
        ),
        request_digest: payload[9..41]
            .try_into()
            .map_err(|_| GuestMaterialError::AuditFailed)?,
        credential_digest: payload[41..73]
            .try_into()
            .map_err(|_| GuestMaterialError::AuditFailed)?,
        configured_digest: payload[73..105]
            .try_into()
            .map_err(|_| GuestMaterialError::AuditFailed)?,
        error_digest: payload[105..137]
            .try_into()
            .map_err(|_| GuestMaterialError::AuditFailed)?,
        dedup_key: payload[137..169]
            .try_into()
            .map_err(|_| GuestMaterialError::AuditFailed)?,
    };
    validate_record(record)?;
    Ok(record)
}

fn validate_record(record: AuditRecordData) -> Result<(), GuestMaterialError> {
    if record.session_generation == 0 || record.request_digest == [0; 32] {
        return Err(GuestMaterialError::AuditFailed);
    }
    if record.outcome == 1 {
        let identity = EnrollmentSuccessAuditIdentity::new(
            record.session_generation,
            record.request_digest,
            record.credential_digest,
            record.configured_digest,
        )?;
        if record.error_digest != [0; 32] || record.dedup_key != identity.dedup_key {
            return Err(GuestMaterialError::AuditFailed);
        }
    } else if record.outcome != 2 || record.dedup_key != canonical_dedup(record)? {
        return Err(GuestMaterialError::AuditFailed);
    }
    Ok(())
}

fn canonical_dedup(record: AuditRecordData) -> Result<[u8; 32], GuestMaterialError> {
    if record.outcome == 1 {
        return Ok(EnrollmentSuccessAuditIdentity::new(
            record.session_generation,
            record.request_digest,
            record.credential_digest,
            record.configured_digest,
        )?
        .dedup_key);
    }
    if record.outcome != 2 {
        return Err(GuestMaterialError::AuditFailed);
    }
    let mut digest = Sha256::new();
    digest.update(b"d2b-guest-material-audit-v3\0");
    digest.update([record.outcome]);
    digest.update(record.session_generation.to_be_bytes());
    digest.update(record.request_digest);
    digest.update(record.credential_digest);
    digest.update(record.configured_digest);
    digest.update(record.error_digest);
    Ok(digest.finalize().into())
}

fn frame_checksum(payload: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2b-guest-material-audit-frame-v3\0");
    digest.update(payload);
    digest.finalize().into()
}

struct ParsedV3 {
    success_keys: BTreeSet<[u8; 32]>,
    records: Vec<AuditRecordData>,
    truncate_to: usize,
}

fn parse_v3(encoded: &[u8]) -> Result<ParsedV3, GuestMaterialError> {
    let mut success_keys = BTreeSet::new();
    let mut records = Vec::new();
    let mut offset = 0;
    while offset < encoded.len() {
        let remaining = &encoded[offset..];
        if remaining.len() < 12 {
            break;
        }
        if &remaining[..8] != V3_MAGIC {
            if has_valid_commit_at_standard_offset(remaining) || remaining.len() > V3_FRAME_BYTES {
                return Err(GuestMaterialError::AuditFailed);
            }
            break;
        }
        let payload_len = u32::from_be_bytes(
            remaining[8..12]
                .try_into()
                .map_err(|_| GuestMaterialError::AuditFailed)?,
        ) as usize;
        if payload_len != V3_PAYLOAD_BYTES {
            if has_valid_commit_at_standard_offset(remaining) || remaining.len() > V3_FRAME_BYTES {
                return Err(GuestMaterialError::AuditFailed);
            }
            break;
        }
        if remaining.len() < V3_PHASE_BYTES || remaining.len() < V3_FRAME_BYTES {
            break;
        }
        let payload = &remaining[12..12 + V3_PAYLOAD_BYTES];
        let checksum: [u8; 32] = remaining[12 + V3_PAYLOAD_BYTES..V3_PHASE_BYTES]
            .try_into()
            .map_err(|_| GuestMaterialError::AuditFailed)?;
        let trailer = &remaining[V3_PHASE_BYTES..V3_FRAME_BYTES];
        if &trailer[..8] != V3_COMMIT_MAGIC || trailer[8..] != checksum {
            if remaining.len() > V3_FRAME_BYTES {
                return Err(GuestMaterialError::AuditFailed);
            }
            break;
        }
        if checksum != frame_checksum(payload) {
            return Err(GuestMaterialError::AuditFailed);
        }
        let record = decode_payload(payload)?;
        if record.outcome == 1 {
            success_keys.insert(record.dedup_key);
        }
        records.push(record);
        offset += V3_FRAME_BYTES;
    }
    Ok(ParsedV3 {
        success_keys,
        records,
        truncate_to: offset,
    })
}

fn has_valid_commit_at_standard_offset(remaining: &[u8]) -> bool {
    remaining.len() >= V3_FRAME_BYTES
        && &remaining[V3_PHASE_BYTES..V3_PHASE_BYTES + 8] == V3_COMMIT_MAGIC
}

impl fmt::Debug for FileGuestMaterialAuditSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FileGuestMaterialAuditSink(REDACTED)")
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    struct PermissionRestore {
        path: PathBuf,
        permissions: std::fs::Permissions,
    }

    impl Drop for PermissionRestore {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.path, self.permissions.clone());
        }
    }

    fn success_data(marker: u8) -> AuditRecordData {
        let identity = EnrollmentSuccessAuditIdentity::new(
            7,
            [marker; 32],
            [marker.wrapping_add(1); 32],
            [marker.wrapping_add(2); 32],
        )
        .unwrap();
        AuditRecordData {
            outcome: 1,
            session_generation: identity.session_generation,
            request_digest: identity.request_digest,
            credential_digest: identity.credential_digest,
            configured_digest: identity.configured_digest,
            error_digest: [0; 32],
            dedup_key: identity.dedup_key,
        }
    }

    fn success_record(marker: u8) -> GuestMaterialAuditRecord {
        let data = success_data(marker);
        GuestMaterialAuditRecord {
            realm_id: "private-realm".to_owned(),
            workload_id: "private-workload".to_owned(),
            operation_id: "private-operation".to_owned(),
            session_storage_ref: "/private/session/path".to_owned(),
            configured_storage_ref: "/private/configured/path".to_owned(),
            session_generation: data.session_generation,
            request_digest: data.request_digest,
            credential_digest: Some(data.credential_digest),
            configured_launch_digest: data.configured_digest,
            outcome: GuestMaterialOutcome::Succeeded,
            error_kind: None,
        }
    }

    fn committed_frame(record: AuditRecordData) -> Vec<u8> {
        let payload = encode_payload(record);
        let checksum = frame_checksum(&payload);
        let mut encoded = Vec::new();
        encoded.extend_from_slice(V3_MAGIC);
        encoded.extend_from_slice(&(V3_PAYLOAD_BYTES as u32).to_be_bytes());
        encoded.extend_from_slice(&payload);
        encoded.extend_from_slice(&checksum);
        encoded.extend_from_slice(V3_COMMIT_MAGIC);
        encoded.extend_from_slice(&checksum);
        encoded
    }

    #[test]
    fn success_audit_is_path_free_and_deduplicated() {
        let root = crate::test_tempdir("guest-material-audit");
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();
        let path = root.path().join("material.audit");
        let sink = FileGuestMaterialAuditSink::open(path.clone(), uid, gid).unwrap();
        sink.record(&success_record(1)).unwrap();
        sink.record(&success_record(1)).unwrap();
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(bytes.len(), V3_FRAME_BYTES);
        assert!(!bytes.windows(7).any(|window| window == b"private"));
    }

    #[test]
    fn non_rotating_append_does_not_enumerate_parent_directory() {
        let root = crate::test_tempdir("guest-material-audit-no-append-scan");
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();
        let path = root.path().join("material.audit");
        let sink = FileGuestMaterialAuditSink::open(path.clone(), uid, gid).unwrap();
        let restore = PermissionRestore {
            path: root.path().to_path_buf(),
            permissions: std::fs::metadata(root.path()).unwrap().permissions(),
        };
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o300)).unwrap();

        sink.record(&success_record(1)).unwrap();

        drop(restore);
        let parsed = parse_v3(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(parsed.records.len(), 1);
        assert!(parsed.records[0] == success_data(1));
    }

    #[test]
    fn unshipped_legacy_formats_are_rejected_without_mutation() {
        for magic in [b"D2BGMA1\0", b"D2BGMA2\0"] {
            let root = crate::test_tempdir("guest-material-audit-legacy-rejected");
            let uid = rustix::process::getuid().as_raw();
            let gid = rustix::process::getgid().as_raw();
            let path = root.path().join("material.audit");
            let mut encoded = magic.to_vec();
            encoded.extend_from_slice(&[0x5a; 64]);
            std::fs::write(&path, &encoded).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

            assert!(FileGuestMaterialAuditSink::open(path.clone(), uid, gid).is_err());
            assert_eq!(std::fs::read(path).unwrap(), encoded);
        }
    }

    #[test]
    fn rotation_honors_exact_fit_one_over_and_restarts_without_duplication() {
        let root = crate::test_tempdir("guest-material-audit-rotation");
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();
        let path = root.path().join("material.audit");
        let limit = (2 * V3_FRAME_BYTES) as u64;
        let sink =
            FileGuestMaterialAuditSink::open_with_limit(path.clone(), uid, gid, limit).unwrap();
        sink.record(&success_record(1)).unwrap();
        sink.record(&success_record(4)).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), limit);
        assert!(segment_paths(&path).unwrap().is_empty());
        sink.record(&success_record(7)).unwrap();
        let segments = segment_paths(&path).unwrap();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].0, 0);
        assert_eq!(std::fs::metadata(&segments[0].1).unwrap().len(), limit);
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            V3_FRAME_BYTES as u64
        );
        drop(sink);

        let reopened =
            FileGuestMaterialAuditSink::open_with_limit(path.clone(), uid, gid, limit).unwrap();
        reopened.record(&success_record(7)).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            V3_FRAME_BYTES as u64
        );
        assert_eq!(segment_paths(&path).unwrap().len(), 1);
    }

    #[test]
    fn rotation_crash_and_retention_recover_deterministically() {
        let root = crate::test_tempdir("guest-material-audit-rotation-crash");
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();
        let path = root.path().join("material.audit");
        let limit = V3_FRAME_BYTES as u64;
        {
            let sink =
                FileGuestMaterialAuditSink::open_with_limit(path.clone(), uid, gid, limit).unwrap();
            sink.record(&success_record(1)).unwrap();
        }
        let stranded = segment_path(&path, 0).unwrap();
        std::fs::rename(&path, &stranded).unwrap();
        let sink =
            FileGuestMaterialAuditSink::open_with_limit(path.clone(), uid, gid, limit).unwrap();
        sink.record(&success_record(4)).unwrap();
        assert_eq!(segment_paths(&path).unwrap().len(), 1);
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            V3_FRAME_BYTES as u64
        );
        for marker in 5..=(MAX_RETAINED_SEGMENTS as u8 + 8) {
            sink.record(&success_record(marker)).unwrap();
        }
        let segments = segment_paths(&path).unwrap();
        assert_eq!(segments.len(), MAX_RETAINED_SEGMENTS);
        assert!(segments.windows(2).all(|pair| pair[0].0 < pair[1].0));
        drop(sink);
        FileGuestMaterialAuditSink::open_with_limit(path.clone(), uid, gid, limit).unwrap();
        assert_eq!(segment_paths(&path).unwrap().len(), MAX_RETAINED_SEGMENTS);
    }

    #[test]
    fn every_torn_offset_and_full_uncommitted_allocation_truncates_only_tail() {
        let first = committed_frame(success_data(1));
        let second = committed_frame(success_data(5));
        for cut in 1..V3_FRAME_BYTES {
            let root = crate::test_tempdir("guest-material-audit-torn");
            let uid = rustix::process::getuid().as_raw();
            let gid = rustix::process::getgid().as_raw();
            let path = root.path().join("material.audit");
            let mut bytes = first.clone();
            bytes.extend_from_slice(&second[..cut]);
            std::fs::write(&path, bytes).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            FileGuestMaterialAuditSink::open(path.clone(), uid, gid).unwrap();
            assert_eq!(std::fs::metadata(path).unwrap().len() as usize, first.len());
        }

        let root = crate::test_tempdir("guest-material-audit-full-torn");
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();
        let path = root.path().join("material.audit");
        let mut bytes = first.clone();
        bytes.extend_from_slice(&second[..V3_PHASE_BYTES]);
        bytes.extend_from_slice(&[0; V3_TRAILER_BYTES]);
        std::fs::write(&path, bytes).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        FileGuestMaterialAuditSink::open(path.clone(), uid, gid).unwrap();
        assert_eq!(std::fs::metadata(path).unwrap().len() as usize, first.len());
    }

    #[test]
    fn committed_or_nonfinal_corruption_fails_closed() {
        let root = crate::test_tempdir("guest-material-audit-corrupt");
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();
        let path = root.path().join("material.audit");
        let mut first = committed_frame(success_data(1));
        first[20] ^= 1;
        first.extend_from_slice(&committed_frame(success_data(5)));
        std::fs::write(&path, first).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            FileGuestMaterialAuditSink::open(path, uid, gid).unwrap_err(),
            GuestMaterialError::AuditFailed
        );
    }
}
