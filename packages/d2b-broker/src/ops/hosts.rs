//! `UpdateHostsFile` op.
//!
//! Writes the d2b-managed block of `/etc/hosts` while preserving
//! every foreign line byte-for-byte. Path safety: `openat2` with
//! `O_NOFOLLOW` + `RESOLVE_BENEATH`; replace via `O_TMPFILE` +
//! `linkat` (or `openat2` + `rename`).
//!
//! The implementation here is layered:
//!
//! - [`UpdateHostsRequest`] is the typed input the broker dispatcher
//!   reconstructs from the wire envelope;
//! - [`update_hosts_file`] is the entry point - pure logic + safe FS
//!   ops via [`crate::sys::path_safe`];
//! - audit emission lives in the caller (`runtime.rs` dispatcher) so
//!   one audit-log fd holds the only `O_APPEND` write handle per plan
//!   §"Broker audit event schema".

use crate::ops::exec_reconcile::{ReconcileExecError, ReconcileExecutor};
use crate::sys::path_safe;
use d2b_core::bundle_resolver::ResolvedHostsIntent;
use d2b_core::host_w3::HostsEntry;
use d2b_host::routes::{extract_managed_block, render_hosts_block};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct UpdateHostsRequest {
    pub hosts_path: PathBuf,
    pub entries: Vec<HostsEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateHostsResult {
    pub before_hash: String,
    pub after_hash: String,
    pub replaced: bool,
}

pub fn update_hosts_file(req: &UpdateHostsRequest) -> io::Result<UpdateHostsResult> {
    path_safe::refuse_world_writable_parent(&req.hosts_path)?;
    path_safe::refuse_symlink(&req.hosts_path)?;

    let body = path_safe::read_to_string_nofollow(&req.hosts_path).unwrap_or_default();
    let before_block = extract_managed_block(&body).unwrap_or_default();
    let new_block = render_hosts_block(&req.entries);
    if before_block == new_block && !body.is_empty() {
        return Ok(UpdateHostsResult {
            before_hash: stable_hash(&before_block),
            after_hash: stable_hash(&new_block),
            replaced: false,
        });
    }

    let new_body = splice_managed_block(&body, &new_block);
    path_safe::atomic_replace(&req.hosts_path, new_body.as_bytes())?;
    Ok(UpdateHostsResult {
        before_hash: stable_hash(&before_block),
        after_hash: stable_hash(&new_block),
        replaced: true,
    })
}

fn splice_managed_block(existing: &str, new_block: &str) -> String {
    splice_marker_block(
        existing,
        new_block,
        "# d2b-managed begin",
        "# d2b-managed end",
    )
}

fn splice_marker_block(existing: &str, new_block: &str, begin: &str, end: &str) -> String {
    if let Some(begin_index) = existing.find(begin)
        && let Some(end_rel) = existing[begin_index..].find(end)
    {
        let end_index = begin_index + end_rel + end.len();
        let suffix_start = existing[end_index..]
            .find('\n')
            .map(|offset| end_index + offset + 1)
            .unwrap_or(existing.len());
        let mut out = String::with_capacity(existing.len() + new_block.len());
        out.push_str(&existing[..begin_index]);
        out.push_str(new_block);
        out.push_str(&existing[suffix_start..]);
        return out;
    }

    let mut out = String::from(existing);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(new_block);
    out
}

fn stable_hash(s: &str) -> String {
    stable_hash_str(s)
}

/// FNV-1a 64-bit hex digest, exposed so sibling broker ops can audit
/// path/marker fingerprints with parity to host-side hashing.
pub fn stable_hash_str(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Convenience helper used by the L1c path-safety test binary to
/// validate the parent-dir guards independently of the full request
/// pipeline.
pub fn refuse_unsafe_parent(path: &Path) -> io::Result<()> {
    path_safe::refuse_world_writable_parent(path)?;
    path_safe::refuse_symlink(path)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteMarkerBlockError {
    Io(String),
    ReconcileExec(ReconcileExecError),
    ForeignOwnership,
}

impl std::fmt::Display for WriteMarkerBlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(detail) => write!(f, "update-hosts marker splice: {detail}"),
            Self::ReconcileExec(err) => write!(f, "update-hosts marker splice: {err}"),
            Self::ForeignOwnership => f.write_str("update-hosts foreign ownership marker"),
        }
    }
}

impl std::error::Error for WriteMarkerBlockError {}

/// Runtime entry-point for `UpdateHostsFile`.
///
/// The resolver already renders the canonical marker-delimited block.
/// The broker owns the merge: preserve foreign lines, replace the
/// managed region when present, append it otherwise, then hand the
/// final file to the atomic writer.
pub fn write_marker_block(
    executor: &dyn ReconcileExecutor,
    intent: &ResolvedHostsIntent,
) -> Result<(), WriteMarkerBlockError> {
    refuse_unsafe_parent(&intent.path).map_err(|err| WriteMarkerBlockError::Io(err.to_string()))?;
    let existing = match path_safe::read_to_string_nofollow(&intent.path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(WriteMarkerBlockError::Io(err.to_string())),
    };
    validate_marker_ownership(&existing, intent)?;
    let merged = if intent.ownership_marker.is_some() {
        splice_owned_marker_block(&existing, intent)
    } else {
        splice_marker_block(
            &existing,
            &intent.managed_block,
            &intent.start_marker,
            &intent.end_marker,
        )
    };
    executor
        .write_atomic_file(&intent.path, merged.as_bytes(), intent.mode)
        .map_err(WriteMarkerBlockError::ReconcileExec)
}

pub fn remove_marker_block(
    executor: &dyn ReconcileExecutor,
    intent: &ResolvedHostsIntent,
) -> Result<(), WriteMarkerBlockError> {
    refuse_unsafe_parent(&intent.path).map_err(|err| WriteMarkerBlockError::Io(err.to_string()))?;
    let existing = match path_safe::read_to_string_nofollow(&intent.path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(WriteMarkerBlockError::Io(err.to_string())),
    };
    validate_marker_ownership(&existing, intent)?;
    let merged = if intent.ownership_marker.is_some() {
        remove_owned_marker_block(&existing, intent)?
    } else {
        remove_marker_block_from_existing(&existing, &intent.start_marker, &intent.end_marker)
    };
    if merged == existing {
        return Ok(());
    }
    executor
        .write_atomic_file(&intent.path, merged.as_bytes(), intent.mode)
        .map_err(WriteMarkerBlockError::ReconcileExec)
}

fn validate_marker_ownership(
    existing: &str,
    intent: &ResolvedHostsIntent,
) -> Result<(), WriteMarkerBlockError> {
    if intent.ownership_marker.is_none() {
        return Ok(());
    }
    if !existing.contains(&intent.start_marker) {
        if existing.contains(&intent.end_marker)
            || existing
                .lines()
                .any(|line| line.trim().starts_with("# d2b managed: "))
        {
            return Err(WriteMarkerBlockError::ForeignOwnership);
        }
        return Ok(());
    }
    let mut search_from = 0;
    while let Some(begin_rel) = existing[search_from..].find(&intent.start_marker) {
        let begin = search_from + begin_rel;
        let end = existing[begin..]
            .find(&intent.end_marker)
            .map(|offset| begin + offset + intent.end_marker.len())
            .ok_or(WriteMarkerBlockError::ForeignOwnership)?;
        let markers = existing[begin..end]
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("# d2b managed: "))
            .collect::<Vec<_>>();
        if markers.len() != 1 || !valid_network_marker(markers[0]) {
            return Err(WriteMarkerBlockError::ForeignOwnership);
        }
        search_from = end;
    }
    Ok(())
}

fn valid_network_marker(line: &str) -> bool {
    line.strip_prefix("# d2b managed: network:hosts:zone:")
        .is_some_and(|value| {
            value.contains(":network:")
                && value.contains(":generation:")
                && value.contains(":attachment:")
                && value.contains(":bundle:sha256:")
        })
}

fn find_owned_marker_block(existing: &str, intent: &ResolvedHostsIntent) -> Option<(usize, usize)> {
    let marker = intent.ownership_marker.as_deref()?;
    let expected = format!("# d2b managed: {marker}");
    let mut search_from = 0;
    while let Some(begin_rel) = existing[search_from..].find(&intent.start_marker) {
        let begin = search_from + begin_rel;
        let end_rel = existing[begin..].find(&intent.end_marker)?;
        let end = begin + end_rel + intent.end_marker.len();
        if existing[begin..end]
            .lines()
            .any(|line| line.trim() == expected)
        {
            return Some((begin, end));
        }
        search_from = end;
    }
    None
}

fn splice_owned_marker_block(existing: &str, intent: &ResolvedHostsIntent) -> String {
    if let Some((begin, end)) = find_owned_marker_block(existing, intent) {
        let suffix_start = existing[end..]
            .find('\n')
            .map(|offset| end + offset + 1)
            .unwrap_or(existing.len());
        let mut out = String::with_capacity(existing.len() + intent.managed_block.len());
        out.push_str(&existing[..begin]);
        out.push_str(&intent.managed_block);
        out.push_str(&existing[suffix_start..]);
        return out;
    }
    let mut out = String::from(existing);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&intent.managed_block);
    out
}

fn remove_owned_marker_block(
    existing: &str,
    intent: &ResolvedHostsIntent,
) -> Result<String, WriteMarkerBlockError> {
    let Some((begin, end)) = find_owned_marker_block(existing, intent) else {
        return Ok(existing.to_owned());
    };
    let suffix_start = existing[end..]
        .find('\n')
        .map(|offset| end + offset + 1)
        .unwrap_or(existing.len());
    let mut out = String::with_capacity(existing.len());
    out.push_str(&existing[..begin]);
    out.push_str(&existing[suffix_start..]);
    Ok(out)
}

fn remove_marker_block_from_existing(existing: &str, begin: &str, end: &str) -> String {
    if let Some(begin_index) = existing.find(begin)
        && let Some(end_rel) = existing[begin_index..].find(end)
    {
        let end_index = begin_index + end_rel + end.len();
        let suffix_start = existing[end_index..]
            .find('\n')
            .map(|offset| end_index + offset + 1)
            .unwrap_or(existing.len());
        let mut out = String::with_capacity(existing.len());
        out.push_str(&existing[..begin_index]);
        out.push_str(&existing[suffix_start..]);
        return out;
    }
    existing.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::exec_reconcile::{FakeReconcileExecutor, ReconcileOp};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("target")
            .join("test-scratch")
            .join(format!(
                "d2b-w3-s2-{}-{}-{}",
                name,
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        dir
    }

    #[test]
    fn writes_managed_block_into_fresh_file() {
        let dir = scratch_dir("hosts-fresh");
        let path = dir.join("hosts");
        fs::write(&path, "127.0.0.1 localhost\n").unwrap();
        let req = UpdateHostsRequest {
            hosts_path: path.clone(),
            entries: vec![HostsEntry {
                address: "10.0.0.10".into(),
                hostname: "vm-a".into(),
                aliases: vec![],
            }],
        };
        let res = update_hosts_file(&req).unwrap();
        assert!(res.replaced);
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("127.0.0.1 localhost"));
        assert!(body.contains("# d2b-managed begin"));
        assert!(body.contains("10.0.0.10 vm-a"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn idempotent_when_block_matches() {
        let dir = scratch_dir("hosts-idempotent");
        let path = dir.join("hosts");
        let entries = vec![HostsEntry {
            address: "10.0.0.10".into(),
            hostname: "vm-a".into(),
            aliases: vec![],
        }];
        fs::write(
            &path,
            format!("127.0.0.1 localhost\n{}", render_hosts_block(&entries)),
        )
        .unwrap();
        let res = update_hosts_file(&UpdateHostsRequest {
            hosts_path: path.clone(),
            entries,
        })
        .unwrap();
        assert!(!res.replaced);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn refuses_symlink_target() {
        let dir = scratch_dir("hosts-symlink");
        let real = dir.join("real-hosts");
        fs::write(&real, "").unwrap();
        let link = dir.join("hosts");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let err = refuse_unsafe_parent(&link).unwrap_err();
        assert!(err.kind() == io::ErrorKind::PermissionDenied);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn write_marker_block_preserves_foreign_lines_in_executor_payload() {
        let dir = scratch_dir("hosts-merge");
        let path = dir.join("hosts");
        fs::write(
            &path,
            "127.0.0.1 localhost\n# d2b-managed begin\n10.0.0.2 old\n# d2b-managed end\n192.168.1.1 router\n",
        )
        .unwrap();
        let intent = ResolvedHostsIntent {
            intent_id: "hosts:host".to_owned(),
            path: path.clone(),
            start_marker: "# d2b-managed begin".to_owned(),
            end_marker: "# d2b-managed end".to_owned(),
            managed_block: "# d2b-managed begin\n10.0.0.3 new\n# d2b-managed end\n".to_owned(),
            mode: 0o644,
            provenance: None,
            ownership_marker: None,
        };
        let exec = FakeReconcileExecutor::new();
        write_marker_block(&exec, &intent).unwrap();

        let log = exec.take_log();
        assert_eq!(log.len(), 1);
        match &log[0] {
            ReconcileOp::WriteAtomicFile {
                path: written_path,
                contents,
                mode,
            } => {
                assert_eq!(written_path, &path);
                assert_eq!(*mode, 0o644);
                let merged = String::from_utf8_lossy(contents);
                assert!(merged.contains("127.0.0.1 localhost"));
                assert!(merged.contains("192.168.1.1 router"));
                assert!(merged.contains("10.0.0.3 new"));
                assert!(!merged.contains("10.0.0.2 old"));
            }
            other => panic!("unexpected op: {other:?}"),
        }
    }

    #[test]
    fn network_marker_block_refuses_foreign_existing_block() {
        let dir = scratch_dir("hosts-foreign-marker");
        let path = dir.join("hosts");
        fs::write(
            &path,
            "127.0.0.1 localhost\n# d2b-managed begin\n# d2b managed: foreign\n# d2b-managed end\n",
        )
        .unwrap();
        let intent = ResolvedHostsIntent {
            intent_id: "network-hosts:zone:network:name".to_owned(),
            path: path.clone(),
            start_marker: "# d2b-managed begin".to_owned(),
            end_marker: "# d2b-managed end".to_owned(),
            managed_block: "# d2b-managed begin\n# d2b managed: network:hosts:zone:a:network:b:generation:1:attachment:1:bundle:sha256:0000000000000000000000000000000000000000000000000000000000000000\n# d2b-managed end\n".to_owned(),
            mode: 0o644,
            provenance: None,
            ownership_marker: Some(
                "network:hosts:zone:a:network:b:generation:1:attachment:1:bundle:sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_owned(),
            ),
        };
        let exec = FakeReconcileExecutor::new();
        assert!(matches!(
            write_marker_block(&exec, &intent),
            Err(WriteMarkerBlockError::ForeignOwnership)
        ));
        assert!(exec.take_log().is_empty());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn network_marker_block_preserves_another_network_marker() {
        let dir = scratch_dir("hosts-swapped-marker");
        let path = dir.join("hosts");
        let foreign_marker = "network:hosts:zone:foreign-zone:network:foreign-network:generation:2:attachment:3:bundle:sha256:1111111111111111111111111111111111111111111111111111111111111111";
        let expected_marker = "network:hosts:zone:expected-zone:network:expected-network:generation:4:attachment:7:bundle:sha256:0000000000000000000000000000000000000000000000000000000000000000";
        let before = format!(
            "127.0.0.1 localhost\n# d2b-managed begin\n# d2b managed: {foreign_marker}\n# d2b-managed end\n"
        );
        fs::write(&path, &before).unwrap();
        let intent = ResolvedHostsIntent {
            intent_id: "network-hosts:zone:network:name".to_owned(),
            path: path.clone(),
            start_marker: "# d2b-managed begin".to_owned(),
            end_marker: "# d2b-managed end".to_owned(),
            managed_block: format!(
                "# d2b-managed begin\n# d2b managed: {expected_marker}\n# d2b-managed end\n"
            ),
            mode: 0o644,
            provenance: None,
            ownership_marker: Some(expected_marker.to_owned()),
        };
        let exec = FakeReconcileExecutor::new();
        write_marker_block(&exec, &intent).unwrap();
        let log = exec.take_log();
        assert_eq!(log.len(), 1);
        let ReconcileOp::WriteAtomicFile { contents, .. } = &log[0] else {
            panic!("expected an atomic hosts write");
        };
        let merged = String::from_utf8_lossy(contents);
        assert!(merged.contains(foreign_marker));
        assert!(merged.contains(expected_marker));
        assert!(merged.starts_with(&before));
        fs::remove_dir_all(dir).ok();
    }
}
