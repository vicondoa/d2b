//! `[pending restart]` machinery for the daemon binary.
//!
//! On startup the daemon writes a [`DaemonVersionFile`] to
//! `/run/d2b/version` recording the running binary's version,
//! resolved store path (when launched from a Nix-built `d2bd`),
//! and the wall-clock start time. The CLI reads the file and
//! compares it against the on-disk binary path under
//! `/run/current-system/sw/bin/d2bd` (or the Tier-0 install path)
//! to compute the daemon-level `pending-restart` signal.
//!
//! This mirrors the per-VM `is_pending_restart` semantics already
//! shipping in the d2b CLI (`current` vs `booted` symlink), but
//! at the daemon-binary level: a newer daemon binary on disk means
//! the running daemon will be replaced by the next `systemctl
//! restart d2bd`.
//!
//! Pure module — system-call surface lives in the production daemon
//! callers; this module is data shuffling + filesystem helpers behind
//! a [`FilesystemReader`] trait so tests do not need /run.

use serde::{Deserialize, Serialize};

/// Persistent payload of `/run/d2b/version`. Written exactly
/// once per daemon process at startup; updated only by a restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DaemonVersionFile {
    /// Semver of the running daemon (mirrors `DEFAULT_SERVER_VERSION`
    /// at build time).
    pub server_version: String,
    /// Absolute path the daemon was launched from. For Nix-built
    /// installs this is the Nix store path of `d2bd`. The CLI
    /// computes `[pending restart]` from this path's identity, so
    /// it must NOT be a symlink to the canonical install path —
    /// the daemon callers resolve via `std::fs::canonicalize` before
    /// writing.
    pub binary_path: String,
    /// RFC 3339 wall-clock at startup. Logged for operator forensics;
    /// the reconciliation logic does not parse it.
    pub started_at: String,
    /// Server-side `PROTOCOL_VERSION` constant (`u32`). Surfaces
    /// post-restart wire compat (a daemon upgrade with a different
    /// `PROTOCOL_VERSION` is also a pending-restart).
    pub protocol_version: u32,
}

/// Outcome of comparing the running daemon's version file against
/// the on-disk binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "status")]
pub enum DaemonRestartStatus {
    /// Running binary matches the on-disk install path; no restart
    /// needed.
    UpToDate,
    /// On-disk install path resolves to a different store path than
    /// the running daemon. A `systemctl restart d2bd` will pick
    /// up the new binary.
    PendingRestart {
        running_path: String,
        on_disk_path: String,
    },
    /// `/run/d2b/version` is missing. Either the daemon is not
    /// running, or it has not yet written the file (a brief window
    /// during startup). The CLI surfaces this as `daemon-down` rather
    /// than `pending-restart`.
    DaemonNotRunning,
    /// `/run/d2b/version` is present but unparseable JSON. The
    /// daemon may be a future incompatible version; the CLI logs
    /// this as `version-file-unreadable` and refuses to compute the
    /// pending-restart signal.
    VersionFileUnreadable { detail: String },
}

/// File-system reads the [`compute_restart_status`] function needs.
/// Production CLI: [`SystemFilesystemReader`]. Tests: an in-memory
/// fake that maps the two paths to canned outcomes.
pub trait FilesystemReader: Send + Sync {
    /// Returns the parsed [`DaemonVersionFile`], `None` if absent,
    /// or `Err(detail)` if present but unparseable.
    fn read_version_file(&self) -> Result<Option<DaemonVersionFile>, String>;
    /// Returns the canonicalized path the install-path symlink
    /// resolves to (`/run/current-system/sw/bin/d2bd` on
    /// NixOS, the package install path on Tier-0). `None` if the
    /// path does not exist.
    fn read_on_disk_binary_path(&self) -> Result<Option<String>, String>;
}

/// Production [`FilesystemReader`] backed by `/run/d2b/version`
/// + the resolved install path.
pub struct SystemFilesystemReader {
    pub version_file_path: String,
    pub install_path: String,
}

impl FilesystemReader for SystemFilesystemReader {
    fn read_version_file(&self) -> Result<Option<DaemonVersionFile>, String> {
        match std::fs::read_to_string(&self.version_file_path) {
            Ok(content) => {
                let parsed: DaemonVersionFile = serde_json::from_str(&content)
                    .map_err(|e| format!("parsing {}: {}", self.version_file_path, e))?;
                Ok(Some(parsed))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.to_string()),
        }
    }

    fn read_on_disk_binary_path(&self) -> Result<Option<String>, String> {
        match std::fs::canonicalize(&self.install_path) {
            Ok(p) => Ok(Some(p.to_string_lossy().into_owned())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.to_string()),
        }
    }
}

/// Compare the running daemon's version file against the on-disk
/// install path. Pure: takes a [`FilesystemReader`], returns the
/// classified [`DaemonRestartStatus`].
pub fn compute_restart_status(reader: &dyn FilesystemReader) -> DaemonRestartStatus {
    let version = match reader.read_version_file() {
        Ok(Some(v)) => v,
        Ok(None) => return DaemonRestartStatus::DaemonNotRunning,
        Err(detail) => return DaemonRestartStatus::VersionFileUnreadable { detail },
    };
    let on_disk = match reader.read_on_disk_binary_path() {
        Ok(Some(p)) => p,
        // Install path missing OR an unrelated error → treat as
        // "no on-disk newer binary" rather than spurious pending-
        // restart; the daemon process is still authoritative.
        _ => return DaemonRestartStatus::UpToDate,
    };
    if version.binary_path == on_disk {
        DaemonRestartStatus::UpToDate
    } else {
        DaemonRestartStatus::PendingRestart {
            running_path: version.binary_path,
            on_disk_path: on_disk,
        }
    }
}

/// Render a human-readable banner for the CLI surface. The bash
/// status command renders this as the daemon-level row alongside
/// the per-VM `[pending restart]` annotations.
pub fn restart_status_banner(status: &DaemonRestartStatus) -> String {
    match status {
        DaemonRestartStatus::UpToDate => "daemon: up-to-date".to_owned(),
        DaemonRestartStatus::PendingRestart {
            running_path,
            on_disk_path,
        } => {
            format!("daemon: [pending restart] (running {running_path} vs on-disk {on_disk_path})")
        }
        DaemonRestartStatus::DaemonNotRunning => "daemon: not running".to_owned(),
        DaemonRestartStatus::VersionFileUnreadable { detail } => {
            format!("daemon: version-file-unreadable ({detail})")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeFs {
        version: Mutex<Option<Result<Option<DaemonVersionFile>, String>>>,
        on_disk: Mutex<Option<Result<Option<String>, String>>>,
    }

    impl FakeFs {
        fn with_version_running(path: &str) -> Self {
            let me = Self::default();
            *me.version.lock().unwrap() = Some(Ok(Some(DaemonVersionFile {
                server_version: "0.4.0".to_owned(),
                binary_path: path.to_owned(),
                started_at: "2026-05-29T03:00:00Z".to_owned(),
                protocol_version: 3,
            })));
            me
        }
        fn with_on_disk(self, path: &str) -> Self {
            *self.on_disk.lock().unwrap() = Some(Ok(Some(path.to_owned())));
            self
        }
        fn with_no_version_file(self) -> Self {
            *self.version.lock().unwrap() = Some(Ok(None));
            *self.on_disk.lock().unwrap() = Some(Ok(Some("ignored".to_owned())));
            self
        }
        fn with_unparseable_version(self) -> Self {
            *self.version.lock().unwrap() =
                Some(Err("parsing /run/d2b/version: expected JSON".to_owned()));
            self
        }
    }

    impl FilesystemReader for FakeFs {
        fn read_version_file(&self) -> Result<Option<DaemonVersionFile>, String> {
            self.version
                .lock()
                .unwrap()
                .clone()
                .expect("fake fs configured for version")
        }
        fn read_on_disk_binary_path(&self) -> Result<Option<String>, String> {
            self.on_disk.lock().unwrap().clone().unwrap_or(Ok(None))
        }
    }

    #[test]
    fn up_to_date_when_paths_match() {
        let fs = FakeFs::with_version_running("/nix/store/abc-d2bd-0.4.0/bin/d2bd")
            .with_on_disk("/nix/store/abc-d2bd-0.4.0/bin/d2bd");
        assert_eq!(compute_restart_status(&fs), DaemonRestartStatus::UpToDate);
    }

    #[test]
    fn pending_restart_when_paths_differ() {
        let fs = FakeFs::with_version_running("/nix/store/old-d2bd-0.4.0/bin/d2bd")
            .with_on_disk("/nix/store/new-d2bd-0.4.1/bin/d2bd");
        let status = compute_restart_status(&fs);
        match status {
            DaemonRestartStatus::PendingRestart {
                running_path,
                on_disk_path,
            } => {
                assert_eq!(running_path, "/nix/store/old-d2bd-0.4.0/bin/d2bd");
                assert_eq!(on_disk_path, "/nix/store/new-d2bd-0.4.1/bin/d2bd");
            }
            other => panic!("expected PendingRestart, got {other:?}"),
        }
    }

    #[test]
    fn daemon_not_running_when_version_file_missing() {
        let fs = FakeFs::default().with_no_version_file();
        assert_eq!(
            compute_restart_status(&fs),
            DaemonRestartStatus::DaemonNotRunning
        );
    }

    #[test]
    fn version_file_unreadable_surfaces_detail() {
        let fs = FakeFs::default().with_unparseable_version();
        match compute_restart_status(&fs) {
            DaemonRestartStatus::VersionFileUnreadable { detail } => {
                assert!(detail.contains("expected JSON"));
            }
            other => panic!("expected VersionFileUnreadable, got {other:?}"),
        }
    }

    #[test]
    fn missing_install_path_treats_as_up_to_date() {
        // If /run/current-system/sw/bin/d2bd is gone (Tier-0
        // weirdness or a deliberate uninstall mid-run), the daemon
        // process is still authoritative. We do NOT surface a
        // spurious pending-restart for an absent install path.
        let fs = FakeFs::with_version_running("/nix/store/abc-d2bd/bin/d2bd");
        // on_disk left as default (returns Ok(None))
        assert_eq!(compute_restart_status(&fs), DaemonRestartStatus::UpToDate);
    }

    #[test]
    fn banner_up_to_date() {
        let banner = restart_status_banner(&DaemonRestartStatus::UpToDate);
        assert_eq!(banner, "daemon: up-to-date");
    }

    #[test]
    fn banner_pending_includes_both_paths() {
        let banner = restart_status_banner(&DaemonRestartStatus::PendingRestart {
            running_path: "/a".to_owned(),
            on_disk_path: "/b".to_owned(),
        });
        assert!(banner.contains("[pending restart]"));
        assert!(banner.contains("/a"));
        assert!(banner.contains("/b"));
    }

    #[test]
    fn banner_not_running() {
        let banner = restart_status_banner(&DaemonRestartStatus::DaemonNotRunning);
        assert_eq!(banner, "daemon: not running");
    }

    #[test]
    fn banner_unreadable_includes_detail() {
        let banner = restart_status_banner(&DaemonRestartStatus::VersionFileUnreadable {
            detail: "bad JSON".to_owned(),
        });
        assert!(banner.contains("version-file-unreadable"));
        assert!(banner.contains("bad JSON"));
    }

    #[test]
    fn version_file_round_trip_serializable() {
        let original = DaemonVersionFile {
            server_version: "0.4.0".to_owned(),
            binary_path: "/nix/store/abc-d2bd-0.4.0/bin/d2bd".to_owned(),
            started_at: "2026-05-29T03:00:00Z".to_owned(),
            protocol_version: 3,
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: DaemonVersionFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn version_file_rejects_unknown_fields() {
        // deny_unknown_fields catches a future field added by a
        // newer daemon and consumed by an older CLI — surfaces
        // VersionFileUnreadable so the CLI does not silently miss
        // the new field.
        let json = serde_json::json!({
            "serverVersion": "0.4.0",
            "binaryPath": "/nix/store/abc/bin/d2bd",
            "startedAt": "2026-05-29T03:00:00Z",
            "protocolVersion": 3,
            "extraNewField": "abc"
        });
        let res: Result<DaemonVersionFile, _> = serde_json::from_value(json);
        assert!(res.is_err());
    }

    #[test]
    fn restart_status_round_trip_serializable() {
        let s = DaemonRestartStatus::PendingRestart {
            running_path: "/a".to_owned(),
            on_disk_path: "/b".to_owned(),
        };
        let json = serde_json::to_string(&s).unwrap();
        let parsed: DaemonRestartStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, s);
    }
}
