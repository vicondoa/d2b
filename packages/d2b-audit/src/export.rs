//! Bounded audit export with inline chain-break records.

use std::{
    fs,
    io::{self, BufRead},
    path::{Path, PathBuf},
};

use crate::{hash_chain::genesis_hash, record_types::AuditRecord};

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
    let mut paths = fs::read_dir(directory.as_ref())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                return false;
            };
            is_segment_name(name)
                && after.is_none_or(|boundary| name > boundary)
                && before.is_none_or(|boundary| name < boundary)
        })
        .collect::<Vec<PathBuf>>();
    paths.sort();
    let mut lines = Vec::new();
    let mut previous = genesis_hash();
    let mut sequence = 0_u64;
    for path in paths {
        let file = fs::File::open(path)?;
        for line in io::BufReader::new(file).lines() {
            let Ok(line) = line else {
                lines.push(ExportLine::Error {
                    sequence,
                    error_code: "read-failed",
                });
                sequence = sequence.saturating_add(1);
                continue;
            };
            match serde_json::from_str::<AuditRecord>(&line) {
                Ok(record) => {
                    if record.verify(&previous).is_err() {
                        lines.push(ExportLine::Error {
                            sequence,
                            error_code: "hash-break",
                        });
                    } else {
                        previous = record.record_hash().clone();
                        lines.push(ExportLine::Record(line));
                    }
                }
                Err(error) => lines.push(ExportLine::Error {
                    sequence,
                    error_code: if error.to_string().contains("audit-record-hash-mismatch") {
                        "hash-break"
                    } else {
                        "record-invalid"
                    },
                }),
            }
            sequence = sequence.saturating_add(1);
        }
    }
    Ok(lines)
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

    #[test]
    fn export_reports_hash_breaks_inline_without_old_fields() {
        let directory =
            std::env::temp_dir().join(format!("d2b-audit-export-{}", std::process::id()));
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
                execution_ref_digest: "sha256:exec".to_owned(),
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
