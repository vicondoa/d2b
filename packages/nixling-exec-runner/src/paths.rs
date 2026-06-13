//! Slot-keyed on-disk layout for a detached exec.
//!
//! Every live/retained detached exec owns a bounded slot index in
//! `[0, DETACHED_RETAINED_PER_VM)`. The slot directory and its files are
//! derived purely from the slot, never from the opaque exec id, so guest
//! journald `_SYSTEMD_UNIT`/`ExecStart` cardinality stays bounded to <= 32
//! stable values that carry no exec id.

use std::path::{Path, PathBuf};

/// Canonical parent for all detached-exec slot directories. Root-owned 0700,
/// boot-scoped (see the nixos `systemd.tmpfiles` rule).
pub const RUN_DIR: &str = "/run/nixling-exec";

/// One captured output stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

impl Stream {
    fn data_name(self) -> &'static str {
        match self {
            Stream::Stdout => "stdout",
            Stream::Stderr => "stderr",
        }
    }

    fn meta_name(self) -> &'static str {
        match self {
            Stream::Stdout => "stdout.meta",
            Stream::Stderr => "stderr.meta",
        }
    }
}

/// Resolves the file paths for a single slot directory under a configurable
/// base (the production base is [`RUN_DIR`]; tests inject a temp dir).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerPaths {
    base: PathBuf,
    slot: u32,
}

impl RunnerPaths {
    pub fn new(base: impl Into<PathBuf>, slot: u32) -> Self {
        Self {
            base: base.into(),
            slot,
        }
    }

    /// Production layout rooted at [`RUN_DIR`].
    pub fn for_slot(slot: u32) -> Self {
        Self::new(RUN_DIR, slot)
    }

    pub fn base(&self) -> &Path {
        &self.base
    }

    pub fn slot(&self) -> u32 {
        self.slot
    }

    /// `slot-<NN>` directory name (zero-padded width 2 for the <= 32 range).
    pub fn slot_dir_name(&self) -> String {
        format!("slot-{:02}", self.slot)
    }

    pub fn slot_dir(&self) -> PathBuf {
        self.base.join(self.slot_dir_name())
    }

    pub fn record(&self) -> PathBuf {
        self.slot_dir().join("record")
    }

    pub fn spec(&self) -> PathBuf {
        self.slot_dir().join("spec")
    }

    pub fn status(&self) -> PathBuf {
        self.slot_dir().join("status")
    }

    pub fn cancel(&self) -> PathBuf {
        self.slot_dir().join("cancel")
    }

    pub fn data(&self, stream: Stream) -> PathBuf {
        self.slot_dir().join(stream.data_name())
    }

    pub fn sidecar(&self, stream: Stream) -> PathBuf {
        self.slot_dir().join(stream.meta_name())
    }

    /// Static basenames of every per-slot file the framework may create. Used
    /// by the re-adoption authenticity gate (per-file `openat`/`O_NOFOLLOW`
    /// `fstat`) and by stale-file scrubbing on slot reuse.
    pub fn slot_file_names() -> [&'static str; 8] {
        [
            "record",
            "spec",
            "status",
            "cancel",
            "stdout",
            "stderr",
            "stdout.meta",
            "stderr.meta",
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_dir_is_zero_padded_and_id_free() {
        let paths = RunnerPaths::new("/run/nixling-exec", 7);
        assert_eq!(paths.slot_dir_name(), "slot-07");
        assert_eq!(
            paths.slot_dir(),
            Path::new("/run/nixling-exec/slot-07").to_path_buf()
        );
        assert!(paths.record().ends_with("slot-07/record"));
        assert!(paths.spec().ends_with("slot-07/spec"));
        assert!(paths.status().ends_with("slot-07/status"));
        assert!(paths.cancel().ends_with("slot-07/cancel"));
        assert!(paths.data(Stream::Stdout).ends_with("slot-07/stdout"));
        assert!(paths
            .sidecar(Stream::Stderr)
            .ends_with("slot-07/stderr.meta"));
    }
}
