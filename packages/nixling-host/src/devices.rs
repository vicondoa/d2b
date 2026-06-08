//! Device-node matrix readback + validators.
//!
//! Host-check does **read-only preflight** on `/dev/kvm`, `/dev/net/tun`,
//! `/dev/vhost-net`, `/dev/fuse`, plus the optional accelerator/USBIP/
//! TPM/vfio matrix. Mutation (ACL fixup, fd handoff) lives in the
//! broker (`nixling_priv_broker::ops::device`); this module exposes
//! only the typed matrix + readback + per-row validators so the L1c
//! `tests/device-node-matrix.sh` canary can drive it deterministically.

use std::collections::BTreeSet;
use std::fs::{self, Metadata};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Device class taxonomy. The ioctl-allowlist derivation
/// (`crate::ioctl_policy`) consumes the same enum so role → ioctl
/// resolution is single-sourced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceClass {
    Kvm,
    NetTun,
    VhostNet,
    Fuse,
    Dri,
    NvidiaCtl,
    NvidiaUvm,
    NvidiaRender,
    PipewireSocket,
    UsbipHost,
    Tpm,
    Vfio,
    /// Cross-domain Wayland needs /dev/udmabuf to wrap imported guest
    /// dmabufs for the host compositor. The Gpu role
    /// claims this class via the per-role device matrix.
    Udmabuf,
}

impl DeviceClass {
    /// Default device-node path for the class. Operators can override
    /// per entry; this is the "happy-path" path used when no override
    /// is present.
    pub const fn default_path(self) -> &'static str {
        match self {
            Self::Kvm => "/dev/kvm",
            Self::NetTun => "/dev/net/tun",
            Self::VhostNet => "/dev/vhost-net",
            Self::Fuse => "/dev/fuse",
            Self::Dri => "/dev/dri",
            Self::NvidiaCtl => "/dev/nvidiactl",
            Self::NvidiaUvm => "/dev/nvidia-uvm",
            // NVIDIA primary device node is /dev/nvidia<N> (per-card),
            // not /dev/nvidia-render. The
            // matrix-loader is responsible for instantiating per-card
            // paths (/dev/nvidia0, /dev/nvidia1, …); default is the
            // single-GPU happy path /dev/nvidia0.
            Self::NvidiaRender => "/dev/nvidia0",
            Self::PipewireSocket => "/run/user/pipewire-0",
            Self::UsbipHost => "/dev/usbip-host",
            Self::Tpm => "/dev/tpm0",
            Self::Vfio => "/dev/vfio/vfio",
            Self::Udmabuf => "/dev/udmabuf",
        }
    }
}

/// What kind of inode the validator expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceNodeKind {
    CharacterDevice,
    BlockDevice,
    UnixSocket,
    Directory,
}

/// A single device-node matrix row. The broker reads its trusted
/// bundle copy of this matrix and refuses to open any path/class
/// combination absent from the row set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceNodeEntry {
    pub class: DeviceClass,
    pub path: PathBuf,
    pub kind: DeviceNodeKind,
    /// Required POSIX mode bits (e.g. `0o660`). Validation requires an
    /// exact `0o7777` match (permission plus special bits).
    pub mode_required: u32,
    /// Required POSIX group name (UNIX group ownership), matched via
    /// the supplied resolver in [`read_device_metadata`]. Empty string
    /// means "no group requirement".
    pub group_required: String,
    /// Whether the role considers this entry mandatory. Optional rows
    /// surface as `MissingOptional` rather than fail-closed.
    pub required: bool,
    /// Operator-facing rationale; surfaced in `nixling host check`
    /// output.
    pub rationale: String,
}

/// Result of reading a device-node path. Pure data so the validator
/// stays deterministic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceNodeReadback {
    pub path: PathBuf,
    pub exists: bool,
    pub kind: Option<DeviceNodeKind>,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub group_name: Option<String>,
}

impl DeviceNodeReadback {
    /// Builds a `DeviceNodeReadback` from a `Metadata` and an optional
    /// gid → name resolver. Pure: used by both the real host probe
    /// and the L1c fake backend.
    pub fn from_metadata(
        path: &Path,
        metadata: &Metadata,
        group_resolver: impl Fn(u32) -> Option<String>,
    ) -> Self {
        let ft = metadata.file_type();
        let kind = if ft.is_char_device() {
            Some(DeviceNodeKind::CharacterDevice)
        } else if ft.is_block_device() {
            Some(DeviceNodeKind::BlockDevice)
        } else if ft.is_socket() {
            Some(DeviceNodeKind::UnixSocket)
        } else if ft.is_dir() {
            Some(DeviceNodeKind::Directory)
        } else {
            None
        };
        Self {
            path: path.to_path_buf(),
            exists: true,
            kind,
            mode: metadata.permissions().mode() & 0o7777,
            uid: metadata.uid(),
            gid: metadata.gid(),
            group_name: group_resolver(metadata.gid()),
        }
    }

    /// Marker for "path not present on disk".
    pub fn absent(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            exists: false,
            kind: None,
            mode: 0,
            uid: 0,
            gid: 0,
            group_name: None,
        }
    }
}

/// Reads metadata for one declared entry. Real-host wrapper; the L1c
/// canary uses [`DeviceNodeReadback::from_metadata`] directly with a
/// fake group resolver.
pub fn read_device_metadata(path: &Path) -> DeviceNodeReadback {
    match fs::symlink_metadata(path) {
        Ok(metadata) => DeviceNodeReadback::from_metadata(path, &metadata, real_group_name),
        Err(_) => DeviceNodeReadback::absent(path),
    }
}

fn real_group_name(gid: u32) -> Option<String> {
    // gid → name resolution via parsing `/etc/group` directly. We
    // intentionally avoid `libc::getgrgid_r` (which would trigger NSS
    // module loading) so the host-prepare path keeps working inside
    // sandboxed activation contexts that whitelist filesystem reads
    // but block dlopen of NSS plugins.
    //
    // Format: one record per line, fields colon-separated:
    //   `<name>:<password>:<gid>:<comma-separated user list>`
    parse_etc_group_for_gid(&std::fs::read_to_string("/etc/group").ok()?, gid)
}

/// Pure parser used by [`real_group_name`] and by the unit tests.
/// Returns the first `name` whose third field equals `gid` (decimal).
pub(crate) fn parse_etc_group_for_gid(body: &str, gid: u32) -> Option<String> {
    let target = gid.to_string();
    for line in body.lines() {
        if line.starts_with('#') {
            continue;
        }
        let mut fields = line.splitn(4, ':');
        let name = fields.next()?;
        let _passwd = fields.next()?;
        let parsed = fields.next()?;
        if parsed == target {
            return Some(name.to_owned());
        }
    }
    None
}

/// Per-entry validation outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceValidation {
    Ok,
    MissingOptional,
    /// Required path is absent; broker cannot open the fd.
    MissingRequired,
    /// Path exists but is the wrong kind (e.g. file instead of char
    /// device).
    WrongKind,
    /// POSIX mode bits do not exactly match the required mask.
    LooseMode,
    /// Group ownership does not match the matrix requirement.
    WrongGroup,
    /// gid → name resolution failed (no `/etc/group` entry and no
    /// trusted bundle gid match). Reported as a warning rather than
    /// a hard fail so dev environments without NSS still surface a
    /// useful host-check finding. Real-host validators downgrade
    /// `WrongGroup` to `GroupUnverifiable` when the name resolver
    /// could not resolve the observed gid AND the bundle's trusted
    /// gid-database has no row for that gid.
    GroupUnverifiable,
}

impl DeviceValidation {
    /// `true` when this validation result should fail-close the host
    /// check for a `required: true` entry. `GroupUnverifiable` is a
    /// warning and does NOT fail-close: it surfaces the finding without
    /// blocking dev-environment workflows.
    pub fn is_fail_closed(&self) -> bool {
        matches!(
            self,
            DeviceValidation::MissingRequired
                | DeviceValidation::WrongKind
                | DeviceValidation::LooseMode
                | DeviceValidation::WrongGroup
        )
    }
}

/// Per-row validation report consumed by `host check` + audit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceValidationRow {
    pub class: DeviceClass,
    pub path: PathBuf,
    pub validation: DeviceValidation,
    pub readback: DeviceNodeReadback,
}

/// Aggregate report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceMatrixReport {
    pub rows: Vec<DeviceValidationRow>,
    pub fail_closed: Vec<DeviceClass>,
}

impl DeviceMatrixReport {
    pub fn ok(&self) -> bool {
        self.fail_closed.is_empty()
    }
}

/// Validates a single declared entry against a readback. The function
/// is pure so the L1c fake-backend matrix can exercise every failure
/// mode without `/dev`.
pub fn validate_entry(entry: &DeviceNodeEntry, readback: &DeviceNodeReadback) -> DeviceValidation {
    validate_entry_with_gid_db(entry, readback, &[])
}

/// Variant of [`validate_entry`] that accepts a bundle-supplied
/// "trusted gid database" — a list of `(gid, name)` rows pinned in
/// the bundle. When the host-side gid → name resolver returns `None`
/// (NSS unavailable, sandboxed activation context), the trusted DB
/// is consulted before falling back to a `GroupUnverifiable` warning.
pub fn validate_entry_with_gid_db(
    entry: &DeviceNodeEntry,
    readback: &DeviceNodeReadback,
    trusted_gid_db: &[(u32, String)],
) -> DeviceValidation {
    if !readback.exists {
        return if entry.required {
            DeviceValidation::MissingRequired
        } else {
            DeviceValidation::MissingOptional
        };
    }
    if let Some(kind) = readback.kind {
        if kind != entry.kind {
            return DeviceValidation::WrongKind;
        }
    } else {
        return DeviceValidation::WrongKind;
    }
    let required_special_bits = entry.mode_required & 0o7000;
    let actual_special_bits = readback.mode & 0o7000;
    if actual_special_bits != required_special_bits {
        return DeviceValidation::LooseMode;
    }
    let required_bits = entry.mode_required & 0o777;
    let actual_bits = readback.mode & 0o777;
    if actual_bits != required_bits {
        return DeviceValidation::LooseMode;
    }
    if !entry.group_required.is_empty() {
        match &readback.group_name {
            Some(name) if name == &entry.group_required => return DeviceValidation::Ok,
            Some(_) => return DeviceValidation::WrongGroup,
            None => {
                // gid → name lookup failed. Try the bundle's pinned
                // trusted DB before downgrading to a warning.
                let resolved = trusted_gid_db
                    .iter()
                    .find_map(|(gid, name)| (*gid == readback.gid).then_some(name.as_str()));
                match resolved {
                    Some(name) if name == entry.group_required => return DeviceValidation::Ok,
                    Some(_) => return DeviceValidation::WrongGroup,
                    None => return DeviceValidation::GroupUnverifiable,
                }
            }
        }
    }
    DeviceValidation::Ok
}

/// Validates an entire entry list. Real-host wrapper.
pub fn validate(entries: &[DeviceNodeEntry]) -> DeviceMatrixReport {
    let mut rows = Vec::with_capacity(entries.len());
    let mut failed = BTreeSet::new();
    for entry in entries {
        let readback = read_device_metadata(&entry.path);
        let validation = validate_entry(entry, &readback);
        if validation.is_fail_closed() && entry.required {
            failed.insert(entry.class);
        }
        rows.push(DeviceValidationRow {
            class: entry.class,
            path: entry.path.clone(),
            validation,
            readback,
        });
    }
    DeviceMatrixReport {
        rows,
        fail_closed: failed.into_iter().collect(),
    }
}

/// Pure-validator entry point used by the L1c fake-backend tests. The
/// caller supplies the readback explicitly so no `/dev` access is
/// required.
pub fn validate_with(entries: &[(DeviceNodeEntry, DeviceNodeReadback)]) -> DeviceMatrixReport {
    let mut rows = Vec::with_capacity(entries.len());
    let mut failed = BTreeSet::new();
    for (entry, readback) in entries {
        let validation = validate_entry(entry, readback);
        if validation.is_fail_closed() && entry.required {
            failed.insert(entry.class);
        }
        rows.push(DeviceValidationRow {
            class: entry.class,
            path: entry.path.clone(),
            validation,
            readback: readback.clone(),
        });
    }
    DeviceMatrixReport {
        rows,
        fail_closed: failed.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(class: DeviceClass, mode: u32, group: &str, required: bool) -> DeviceNodeEntry {
        DeviceNodeEntry {
            class,
            path: PathBuf::from(class.default_path()),
            kind: DeviceNodeKind::CharacterDevice,
            mode_required: mode,
            group_required: group.to_owned(),
            required,
            rationale: "test".to_owned(),
        }
    }

    fn readback(kind: DeviceNodeKind, mode: u32, group: Option<&str>) -> DeviceNodeReadback {
        DeviceNodeReadback {
            path: PathBuf::from("/dev/kvm"),
            exists: true,
            kind: Some(kind),
            mode,
            uid: 0,
            gid: 36,
            group_name: group.map(|s| s.to_owned()),
        }
    }

    #[test]
    fn ok_when_mode_and_group_match() {
        let e = entry(DeviceClass::Kvm, 0o660, "kvm", true);
        let r = readback(DeviceNodeKind::CharacterDevice, 0o660, Some("kvm"));
        assert_eq!(validate_entry(&e, &r), DeviceValidation::Ok);
    }

    #[test]
    fn missing_required_when_absent() {
        let e = entry(DeviceClass::Kvm, 0o660, "kvm", true);
        let r = DeviceNodeReadback::absent(&e.path);
        assert_eq!(validate_entry(&e, &r), DeviceValidation::MissingRequired);
    }

    #[test]
    fn missing_optional_when_absent_and_not_required() {
        let e = entry(DeviceClass::Vfio, 0o660, "vfio", false);
        let r = DeviceNodeReadback::absent(&e.path);
        assert_eq!(validate_entry(&e, &r), DeviceValidation::MissingOptional);
    }

    #[test]
    fn wrong_kind_when_directory_seen_for_char_device() {
        let e = entry(DeviceClass::Kvm, 0o660, "kvm", true);
        let r = readback(DeviceNodeKind::Directory, 0o660, Some("kvm"));
        assert_eq!(validate_entry(&e, &r), DeviceValidation::WrongKind);
    }

    #[test]
    fn loose_mode_when_world_bit_set() {
        let e = entry(DeviceClass::Kvm, 0o660, "kvm", true);
        let r = readback(DeviceNodeKind::CharacterDevice, 0o666, Some("kvm"));
        assert_eq!(validate_entry(&e, &r), DeviceValidation::LooseMode);
    }

    #[test]
    fn loose_mode_when_special_bit_set() {
        let e = entry(DeviceClass::Kvm, 0o660, "kvm", true);
        let r = readback(DeviceNodeKind::CharacterDevice, 0o4660, Some("kvm"));
        assert_eq!(validate_entry(&e, &r), DeviceValidation::LooseMode);
    }

    #[test]
    fn loose_mode_when_group_execute_set() {
        let e = entry(DeviceClass::Kvm, 0o660, "kvm", true);
        let r = readback(DeviceNodeKind::CharacterDevice, 0o670, Some("kvm"));
        assert_eq!(validate_entry(&e, &r), DeviceValidation::LooseMode);
    }

    #[test]
    fn wrong_group_when_group_name_differs() {
        let e = entry(DeviceClass::Kvm, 0o660, "kvm", true);
        let r = readback(DeviceNodeKind::CharacterDevice, 0o660, Some("foo"));
        assert_eq!(validate_entry(&e, &r), DeviceValidation::WrongGroup);
    }

    #[test]
    fn validate_with_collects_fail_closed_classes() {
        let entries = vec![
            (
                entry(DeviceClass::Kvm, 0o660, "kvm", true),
                DeviceNodeReadback::absent(Path::new("/dev/kvm")),
            ),
            (
                entry(DeviceClass::NetTun, 0o660, "kvm", true),
                readback(DeviceNodeKind::CharacterDevice, 0o660, Some("kvm")),
            ),
        ];
        let report = validate_with(&entries);
        assert!(!report.ok());
        assert_eq!(report.fail_closed, vec![DeviceClass::Kvm]);
    }

    #[test]
    fn parse_etc_group_for_gid_matches_numeric_gid() {
        let body = "\
root:x:0:
kvm:x:36:alice,bob
fuse:x:128:
";
        assert_eq!(parse_etc_group_for_gid(body, 36), Some("kvm".to_owned()));
        assert_eq!(parse_etc_group_for_gid(body, 0), Some("root".to_owned()));
        assert_eq!(parse_etc_group_for_gid(body, 9999), None);
    }

    #[test]
    fn unresolved_gid_with_trusted_db_match_yields_ok() {
        let e = entry(DeviceClass::Kvm, 0o660, "kvm", true);
        let mut r = readback(DeviceNodeKind::CharacterDevice, 0o660, None);
        r.gid = 36;
        let validation = validate_entry_with_gid_db(&e, &r, &[(36, "kvm".to_owned())]);
        assert_eq!(validation, DeviceValidation::Ok);
    }

    #[test]
    fn unresolved_gid_without_trusted_db_yields_group_unverifiable() {
        let e = entry(DeviceClass::Kvm, 0o660, "kvm", true);
        let mut r = readback(DeviceNodeKind::CharacterDevice, 0o660, None);
        r.gid = 36;
        let validation = validate_entry_with_gid_db(&e, &r, &[]);
        assert_eq!(validation, DeviceValidation::GroupUnverifiable);
        // GroupUnverifiable is a warning, not fail-closed.
        assert!(!validation.is_fail_closed());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn real_host_dev_kvm_validates_when_kvm_group_exists() {
        // Skip when /etc/group has no `kvm` row (CI containers).
        let body = match std::fs::read_to_string("/etc/group") {
            Ok(b) => b,
            Err(_) => return,
        };
        let kvm_gid = body.lines().find_map(|line| {
            let mut fields = line.splitn(4, ':');
            let name = fields.next()?;
            let _pw = fields.next()?;
            let gid: u32 = fields.next()?.parse().ok()?;
            (name == "kvm").then_some(gid)
        });
        let Some(kvm_gid) = kvm_gid else {
            return;
        };
        // Skip when /dev/kvm is absent (no virt support in CI).
        let Ok(meta) = std::fs::symlink_metadata("/dev/kvm") else {
            return;
        };
        if meta.gid() != kvm_gid {
            return;
        }
        let readback =
            DeviceNodeReadback::from_metadata(Path::new("/dev/kvm"), &meta, real_group_name);
        let e = DeviceNodeEntry {
            class: DeviceClass::Kvm,
            path: PathBuf::from("/dev/kvm"),
            kind: DeviceNodeKind::CharacterDevice,
            mode_required: meta.permissions().mode() & 0o777,
            group_required: "kvm".to_owned(),
            required: true,
            rationale: "real-host integration test".to_owned(),
        };
        let validation = validate_entry(&e, &readback);
        assert_eq!(validation, DeviceValidation::Ok);
    }
}
