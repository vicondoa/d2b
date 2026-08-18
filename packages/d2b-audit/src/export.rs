//! Bounded audit export with inline chain-break records.

use std::{
    fs,
    io::{self, BufRead},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

use crate::{
    record_types::AuditRecord,
    segment::{checkpoint_anchor, checkpoint_pending},
};

/// Maximum records returned by one export call.
pub const MAX_EXPORT_RECORDS: usize = 100_000;
/// Maximum encoded bytes returned by one export call.
///
/// This is deliberately below the 1 MiB seqpacket frame limit so an export
/// page can be wrapped by the broker wire envelope without truncation.
pub const MAX_EXPORT_BYTES: usize = 768 * 1024;
/// Maximum bytes in one input JSONL line.
pub const MAX_EXPORT_LINE_BYTES: usize = 64 * 1024;
/// Maximum directory entries inspected by one export.
pub const MAX_EXPORT_DIRECTORY_ENTRIES: usize = 4096;
/// Maximum input lines inspected by one bounded export.
pub const MAX_EXPORT_SCAN_LINES: usize = 200_000;
/// Maximum input bytes inspected by one bounded export.
pub const MAX_EXPORT_SCAN_BYTES: usize = 64 * 1024 * 1024;

/// One exported NDJSON line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportLine {
    /// A valid record.
    Record(String),
    /// An inline chain or decode failure with no echoed input.
    Error {
        /// Sequence position in the exported stream.
        sequence: u64,
        /// Stable error class.
        error_code: &'static str,
    },
}

impl ExportLine {
    /// Serialize the line as NDJSON.
    pub fn to_json(&self) -> String {
        match self {
            Self::Record(line) => line.clone(),
            Self::Error {
                sequence,
                error_code,
            } => serde_json::json!({
                "export_error": error_code,
                "sequence": sequence,
            })
            .to_string(),
        }
    }
}

/// Export all owned segments in lexical order.
pub fn export_segments(directory: impl AsRef<Path>) -> io::Result<Vec<ExportLine>> {
    export_segments_range(directory, None, None)
}

/// Export a lexical segment range while validating the selected chain.
///
/// `after` and `before` are exclusive filename boundaries. They accept only
/// the basename shape produced by [`SegmentWriter`](crate::SegmentWriter);
/// accepting a path here would turn an export filter into a filesystem
/// traversal surface.
pub fn export_segments_range(
    directory: impl AsRef<Path>,
    after: Option<&str>,
    before: Option<&str>,
) -> io::Result<Vec<ExportLine>> {
    if after.is_some_and(|name| !is_segment_name(name))
        || before.is_some_and(|name| !is_segment_name(name))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "audit-segment-boundary-invalid",
        ));
    }
    let mut paths = Vec::new();
    for (index, entry) in fs::read_dir(directory.as_ref())?.enumerate() {
        if index >= MAX_EXPORT_DIRECTORY_ENTRIES {
            return Err(io::Error::other("audit-export-directory-limit"));
        }
        let entry = entry?;
        let path = entry.path();
        if path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(is_segment_name)
        {
            paths.push(path);
        }
    }
    if checkpoint_pending(directory.as_ref())? {
        return Err(io::Error::other("audit-retention-checkpoint-pending"));
    }
    paths.retain(|path| {
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            return false;
        };
        is_segment_name(name)
    });
    paths.sort();
    let mut lines = Vec::new();
    let mut previous = checkpoint_anchor(directory.as_ref())?;
    let mut export_previous = crate::genesis_hash();
    let mut chain_valid = true;
    let mut sequence = 0_u64;
    let mut encoded_bytes = 0_usize;
    let mut scanned_lines = 0_usize;
    let mut scanned_bytes = 0_usize;
    for path in paths {
        let in_range = path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| {
                after.is_none_or(|boundary| name > boundary)
                    && before.is_none_or(|boundary| name < boundary)
            });
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)?;
        let mut reader = io::BufReader::new(file);
        loop {
            let bytes = match read_bounded_line(&mut reader) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => break,
                Err(_) => {
                    if in_range {
                        let error = ExportLine::Error {
                            sequence,
                            error_code: "read-failed",
                        };
                        let size = error.to_json().len().saturating_add(1);
                        if lines.len() >= MAX_EXPORT_RECORDS
                            || encoded_bytes.saturating_add(size) > MAX_EXPORT_BYTES
                        {
                            return Err(io::Error::other("audit-export-limit"));
                        }
                        encoded_bytes = encoded_bytes.saturating_add(size);
                        lines.push(error);
                        sequence = sequence.saturating_add(1);
                    }
                    chain_valid = false;
                    break;
                }
            };
            scanned_lines = scanned_lines.saturating_add(1);
            scanned_bytes = scanned_bytes.saturating_add(bytes.len());
            if scanned_lines > MAX_EXPORT_SCAN_LINES || scanned_bytes > MAX_EXPORT_SCAN_BYTES {
                return Err(io::Error::other("audit-export-scan-limit"));
            }
            let line = match String::from_utf8(bytes) {
                Ok(line) => line,
                Err(_) => {
                    if in_range {
                        let error = ExportLine::Error {
                            sequence,
                            error_code: "read-failed",
                        };
                        let size = error.to_json().len().saturating_add(1);
                        if lines.len() >= MAX_EXPORT_RECORDS
                            || encoded_bytes.saturating_add(size) > MAX_EXPORT_BYTES
                        {
                            return Err(io::Error::other("audit-export-limit"));
                        }
                        encoded_bytes = encoded_bytes.saturating_add(size);
                        lines.push(error);
                        sequence = sequence.saturating_add(1);
                    }
                    chain_valid = false;
                    continue;
                }
            };
            match serde_json::from_str::<AuditRecord>(&line) {
                Ok(record) if chain_valid && record.verify(&previous).is_ok() => {
                    previous = record.record_hash().clone();
                    if in_range {
                        let normalized = record
                            .redacted_for_export(export_previous.clone())
                            .map_err(|_| io::Error::other("audit-export-record-invalid"))?;
                        let normalized_line =
                            String::from_utf8(normalized.to_json_line().map_err(|_| {
                                io::Error::other("audit-export-record-encode-failed")
                            })?)
                            .map_err(|_| io::Error::other("audit-export-record-invalid"))?;
                        if lines.len() >= MAX_EXPORT_RECORDS
                            || encoded_bytes.saturating_add(normalized_line.len())
                                > MAX_EXPORT_BYTES
                        {
                            return Err(io::Error::other("audit-export-limit"));
                        }
                        encoded_bytes = encoded_bytes.saturating_add(normalized_line.len());
                        export_previous = normalized.record_hash().clone();
                        lines.push(ExportLine::Record(normalized_line));
                        sequence = sequence.saturating_add(1);
                    }
                }
                Ok(_record) => {
                    chain_valid = false;
                    if in_range {
                        let error = ExportLine::Error {
                            sequence,
                            error_code: "hash-break",
                        };
                        let size = error.to_json().len().saturating_add(1);
                        if lines.len() >= MAX_EXPORT_RECORDS
                            || encoded_bytes.saturating_add(size) > MAX_EXPORT_BYTES
                        {
                            return Err(io::Error::other("audit-export-limit"));
                        }
                        encoded_bytes = encoded_bytes.saturating_add(size);
                        lines.push(error);
                        sequence = sequence.saturating_add(1);
                    }
                }
                Err(error) => {
                    if in_range {
                        let error = ExportLine::Error {
                            sequence,
                            error_code: if error.to_string().contains("audit-record-hash-mismatch")
                            {
                                "hash-break"
                            } else {
                                "record-invalid"
                            },
                        };
                        let size = error.to_json().len().saturating_add(1);
                        if lines.len() >= MAX_EXPORT_RECORDS
                            || encoded_bytes.saturating_add(size) > MAX_EXPORT_BYTES
                        {
                            return Err(io::Error::other("audit-export-limit"));
                        }
                        encoded_bytes = encoded_bytes.saturating_add(size);
                        sequence = sequence.saturating_add(1);
                        lines.push(error);
                    }
                    chain_valid = false;
                }
            }
        }
    }
    Ok(lines)
}

fn read_bounded_line<R: BufRead>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "audit-export-line-truncated",
                ))
            };
        }
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(chunk.len(), |index| index + 1);
        if bytes.len().saturating_add(take) > MAX_EXPORT_LINE_BYTES {
            return Err(io::Error::other("audit-export-line-limit"));
        }
        bytes.extend_from_slice(&chunk[..take]);
        reader.consume(take);
        if newline.is_some() {
            bytes.pop();
            return Ok(Some(bytes));
        }
    }
}

/// Return whether a value is an owned audit segment basename.
pub fn is_segment_name(name: &str) -> bool {
    let Some(digits) = name
        .strip_prefix("audit-")
        .and_then(|value| value.strip_suffix(".jsonl"))
    else {
        return false;
    };
    digits.len() == 20 && digits.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hash_chain::genesis_hash,
        record_types::{AuditRecord, AuditRecordFields, ProcessEffectFields},
    };
    use std::io::Write;

    fn writable_manifest_dir() -> std::path::PathBuf {
        std::env::var_os("TEST_TMPDIR")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("CARGO_MANIFEST_DIR").map(std::path::PathBuf::from))
            .or_else(|| std::env::current_dir().ok())
            .expect("resolve test writable directory")
    }

    #[test]
    fn export_reports_hash_breaks_inline_without_old_fields() {
        let directory = writable_manifest_dir()
            .join("target")
            .join(format!("d2b-audit-export-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let record = AuditRecord::new(
            1,
            "work",
            "op",
            "corr",
            None,
            "test",
            genesis_hash(),
            AuditRecordFields::ProcessEffect(ProcessEffectFields {
                event: "launch".to_owned(),
                provider: "systemd".to_owned(),
                domain: "system".to_owned(),
                no_isolation: false,
                execution_ref_digest:
                    "sha256:0000000000000000000000000000000000000000000000000000000000000001"
                        .to_owned(),
                process_uid: "uid".to_owned(),
                outcome: "ok".to_owned(),
                exit_class: None,
            }),
        )
        .unwrap();
        let path = directory.join("audit-20240101000000000000.jsonl");
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string(&record).unwrap()),
        )
        .unwrap();
        let mut tampered = serde_json::to_value(&record).unwrap();
        tampered["zone"] = serde_json::json!("tampered");
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(format!("{}\n", tampered).as_bytes())
            .unwrap();
        let lines = export_segments(&directory).unwrap();
        assert!(lines.iter().any(|line| matches!(
            line,
            ExportLine::Error {
                error_code: "hash-break",
                ..
            }
        )));
        assert!(
            !lines
                .iter()
                .any(|line| line.to_json().contains("\"realm\""))
        );
        let _ = fs::remove_dir_all(directory);
    }
}
