//! Broker ops: `OpenKvm`, `OpenVhostNet`, `OpenFuse`, `OpenDevice`.
//!
//! Each variant validates a declared role × device-class pair against
//! the trusted bundle device-node matrix
//! (`d2b_host::devices::DeviceNodeEntry`) and, on success, returns
//! an `OwnedFd` that the runtime hands back to `d2bd` via
//! `SCM_RIGHTS`. The dispatcher is split here so the L1c canary
//! `tests/device-node-matrix.sh` can drive the typed pre-open
//! decision path without `/dev` access (`PreOpenDecision` is a pure
//! function).

use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};

use nix::sys::stat::{SFlag, fstat, major, minor};
use serde::{Deserialize, Serialize};

use d2b_core::bundle_resolver::BundleResolver;
use d2b_host::devices::{
    DeviceClass, DeviceNodeEntry, DeviceNodeKind, DeviceNodeReadback, DeviceValidation,
    read_device_metadata, validate_entry,
};

use crate::ops::exec_reconcile::SystemLiveExec;

/// Pre-open decision: whether the broker should attempt the open at
/// all, and which audit disposition to record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PreOpenDecision {
    Allow,
    DeniedNotInMatrix,
    DeniedRoleClassMismatch,
    DeniedValidation(DeviceValidation),
}

/// Audit fields for an `Open*` dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAuditRecord {
    pub device_class: DeviceClass,
    pub role_id: String,
    pub decision: PreOpenDecision,
}

/// Per-role declared device classes. The dispatcher refuses any
/// `Open*` request whose role does not declare the matching class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleDeviceClaim {
    pub role_id: String,
    pub allowed_classes: Vec<DeviceClass>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeviceMatrixEntry {
    entry: DeviceNodeEntry,
    expected_major_minor: Option<(u64, u64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeviceOpenValidationError {
    DeviceMajorMinorMismatch {
        expected: (u64, u64),
        actual: (u64, u64),
    },
    DeviceKindMismatch {
        expected: DeviceNodeKind,
        actual: String,
    },
}

impl std::fmt::Display for DeviceOpenValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeviceMajorMinorMismatch { expected, actual } => write!(
                f,
                "device major/minor mismatch: expected {}:{}, got {}:{}",
                expected.0, expected.1, actual.0, actual.1,
            ),
            Self::DeviceKindMismatch { expected, actual } => write!(
                f,
                "device type mismatch: expected {}, got {}",
                device_kind_name(expected),
                actual,
            ),
        }
    }
}

/// Pre-open decision used by both the real-host path and the L1c fake
/// canary. Pure: caller supplies the readback.
pub fn pre_open_decision(
    requested_class: DeviceClass,
    role: &RoleDeviceClaim,
    matrix: &[DeviceNodeEntry],
    readback: &DeviceNodeReadback,
) -> PreOpenDecision {
    if !role.allowed_classes.contains(&requested_class) {
        return PreOpenDecision::DeniedRoleClassMismatch;
    }
    let Some(entry) = matrix
        .iter()
        .find(|e| e.class == requested_class && e.path == readback.path)
    else {
        return PreOpenDecision::DeniedNotInMatrix;
    };
    match validate_entry(entry, readback) {
        DeviceValidation::Ok => PreOpenDecision::Allow,
        other => PreOpenDecision::DeniedValidation(other),
    }
}

/// Helper for `tests/device-node-matrix.sh`: an `Open*`-style record
/// the L1c canary asserts against.
pub fn audit_for(
    requested_class: DeviceClass,
    role: &RoleDeviceClaim,
    decision: PreOpenDecision,
) -> OpenAuditRecord {
    OpenAuditRecord {
        device_class: requested_class,
        role_id: role.role_id.clone(),
        decision,
    }
}

/// Open the underlying device fd. Caller is responsible for the
/// pre-open decision (see [`pre_open_decision`]). The returned
/// `OwnedFd` is `O_CLOEXEC` (`std::fs::OpenOptions` sets that by
/// default on Linux); the broker's `SCM_RIGHTS` send path clears
/// CLOEXEC on the recipient side only.
pub fn open_device_fd(path: &Path, read_write: bool) -> Result<OwnedFd, std::io::Error> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;
    let mut opts = OpenOptions::new();
    if read_write {
        opts.read(true).write(true);
    } else {
        opts.read(true);
    }
    opts.custom_flags(nix::libc::O_NONBLOCK | nix::libc::O_NOFOLLOW);
    let file = opts.open(path)?;
    Ok(OwnedFd::from(file))
}

#[derive(Debug)]
pub struct LiveOpenDeviceOutcome {
    pub device_path: PathBuf,
    pub fd: OwnedFd,
    pub device_class: String,
    pub matrix_entry_id: String,
}

pub fn live_open_kvm(
    _exec: &SystemLiveExec,
    resolver: &BundleResolver,
    req: &d2b_contracts::broker_wire::OpenKvmRequest,
    _audit_log: &crate::audit::AuditLog,
) -> Result<LiveOpenDeviceOutcome, super::OpError> {
    live_open_device_common(resolver, req.role_id.as_str(), DeviceClass::Kvm)
}

pub fn live_open_vhost_net(
    _exec: &SystemLiveExec,
    resolver: &BundleResolver,
    req: &d2b_contracts::broker_wire::OpenVhostNetRequest,
    _audit_log: &crate::audit::AuditLog,
) -> Result<LiveOpenDeviceOutcome, super::OpError> {
    live_open_device_common(resolver, req.role_id.as_str(), DeviceClass::VhostNet)
}

pub fn live_open_fuse(
    _exec: &SystemLiveExec,
    resolver: &BundleResolver,
    req: &d2b_contracts::broker_wire::OpenFuseRequest,
    _audit_log: &crate::audit::AuditLog,
) -> Result<LiveOpenDeviceOutcome, super::OpError> {
    live_open_device_common(resolver, req.role_id.as_str(), DeviceClass::Fuse)
}

pub fn live_open_device(
    _exec: &SystemLiveExec,
    resolver: &BundleResolver,
    req: &d2b_contracts::broker_wire::OpenDeviceRequest,
    _audit_log: &crate::audit::AuditLog,
) -> Result<LiveOpenDeviceOutcome, super::OpError> {
    let requested_class: DeviceClass = serde_json::from_value(serde_json::Value::String(
        req.device_class.clone(),
    ))
    .map_err(|e| super::OpError::InvalidInput {
        detail: format!("unknown device class `{}`: {e}", req.device_class),
    })?;
    live_open_device_common(resolver, req.role_id.as_str(), requested_class)
}

fn live_open_device_common(
    resolver: &BundleResolver,
    role_id: &str,
    requested_class: DeviceClass,
) -> Result<LiveOpenDeviceOutcome, super::OpError> {
    let claim = resolver.resolve_role_device_claim(role_id).ok_or_else(|| {
        super::OpError::UnknownSubject {
            operation: "OpenDevice",
            subject: role_id.to_owned(),
        }
    })?;
    let requested_name = serde_json::to_value(requested_class)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| {
            requested_class
                .default_path()
                .trim_start_matches("/dev/")
                .to_owned()
        });
    if !claim
        .allowed_device_classes
        .iter()
        .any(|c| c == &requested_name)
    {
        return Err(super::OpError::Refused {
            operation: "OpenDevice",
            reason: format!(
                "role `{role_id}` may not receive device class `{}`",
                requested_name,
            ),
        });
    }

    let matrix_entry = default_matrix_entry(requested_class);
    let readback = read_device_metadata(&matrix_entry.entry.path);
    match validate_entry(&matrix_entry.entry, &readback) {
        DeviceValidation::Ok => {}
        other => {
            return Err(super::OpError::Refused {
                operation: "OpenDevice",
                reason: format!(
                    "device matrix validation failed for {}: {:?}",
                    matrix_entry.entry.path.display(),
                    other
                ),
            });
        }
    }
    let fd = open_device_fd(
        &matrix_entry.entry.path,
        read_write_for_class(requested_class),
    )
    .map_err(|e| super::OpError::Io {
        path: matrix_entry.entry.path.clone(),
        detail: e.to_string(),
    })?;
    let stat = fstat(fd.as_raw_fd()).map_err(|e| super::OpError::Io {
        path: matrix_entry.entry.path.clone(),
        detail: e.to_string(),
    })?;
    let actual_kind =
        device_kind_from_mode(stat.st_mode).ok_or_else(|| super::OpError::Refused {
            operation: "OpenDevice",
            reason: format!(
                "{}: opened path has unsupported file type mode {:#o}",
                matrix_entry.entry.path.display(),
                stat.st_mode,
            ),
        })?;
    let actual_major_minor = match &actual_kind {
        DeviceNodeKind::CharacterDevice | DeviceNodeKind::BlockDevice => {
            Some((major(stat.st_rdev), minor(stat.st_rdev)))
        }
        _ => None,
    };
    if let Err(validation_err) =
        validate_opened_device(&matrix_entry, actual_kind, actual_major_minor)
    {
        return Err(super::OpError::Refused {
            operation: "OpenDevice",
            reason: format!("{}: {validation_err}", matrix_entry.entry.path.display()),
        });
    }
    Ok(LiveOpenDeviceOutcome {
        device_path: matrix_entry.entry.path.clone(),
        fd,
        device_class: requested_name,
        matrix_entry_id: format!("device:{}", matrix_entry.entry.path.display()),
    })
}

fn read_write_for_class(class: DeviceClass) -> bool {
    matches!(
        class,
        DeviceClass::NetTun | DeviceClass::VhostNet | DeviceClass::Fuse
    )
}

fn device_kind_name(kind: &DeviceNodeKind) -> &'static str {
    match kind {
        DeviceNodeKind::CharacterDevice => "char-device",
        DeviceNodeKind::BlockDevice => "block-device",
        DeviceNodeKind::Directory => "directory",
        DeviceNodeKind::UnixSocket => "unix-socket",
    }
}

fn device_kind_from_mode(mode: nix::libc::mode_t) -> Option<DeviceNodeKind> {
    match SFlag::from_bits_truncate(mode) {
        SFlag::S_IFCHR => Some(DeviceNodeKind::CharacterDevice),
        SFlag::S_IFBLK => Some(DeviceNodeKind::BlockDevice),
        SFlag::S_IFDIR => Some(DeviceNodeKind::Directory),
        SFlag::S_IFSOCK => Some(DeviceNodeKind::UnixSocket),
        _ => None,
    }
}

fn expected_major_minor_for_class(class: DeviceClass) -> Option<(u64, u64)> {
    match class {
        DeviceClass::Kvm => Some((10, 232)),
        DeviceClass::NetTun => Some((10, 200)),
        DeviceClass::VhostNet => Some((10, 238)),
        DeviceClass::Fuse => Some((10, 229)),
        DeviceClass::NvidiaCtl => Some((195, 255)),
        DeviceClass::NvidiaUvm => Some((236, 0)),
        DeviceClass::NvidiaRender => None,
        DeviceClass::PipewireSocket | DeviceClass::Dri => None,
        DeviceClass::UsbipHost => Some((251, 0)),
        DeviceClass::Tpm => Some((10, 224)),
        DeviceClass::Vfio => Some((10, 196)),
        // udmabuf is a misc device with dynamic minor; skip the
        // major:minor check (matrix-loader still validates the path +
        // kind + group).
        DeviceClass::Udmabuf => None,
    }
}

fn validate_opened_device(
    matrix_entry: &DeviceMatrixEntry,
    actual_kind: DeviceNodeKind,
    actual_major_minor: Option<(u64, u64)>,
) -> Result<(), DeviceOpenValidationError> {
    if matrix_entry.entry.kind != actual_kind {
        return Err(DeviceOpenValidationError::DeviceKindMismatch {
            expected: matrix_entry.entry.kind,
            actual: device_kind_name(&actual_kind).to_owned(),
        });
    }
    if let Some(expected) = matrix_entry.expected_major_minor {
        match actual_major_minor {
            Some(actual) if actual == expected => {}
            Some(actual) => {
                return Err(DeviceOpenValidationError::DeviceMajorMinorMismatch {
                    expected,
                    actual,
                });
            }
            None => {
                return Err(DeviceOpenValidationError::DeviceKindMismatch {
                    expected: matrix_entry.entry.kind,
                    actual: "non-device".to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn default_matrix_entry(class: DeviceClass) -> DeviceMatrixEntry {
    DeviceMatrixEntry {
        expected_major_minor: expected_major_minor_for_class(class),
        entry: DeviceNodeEntry {
            class,
            path: PathBuf::from(class.default_path()),
            kind: match class {
                DeviceClass::PipewireSocket => DeviceNodeKind::UnixSocket,
                DeviceClass::Dri => DeviceNodeKind::Directory,
                _ => DeviceNodeKind::CharacterDevice,
            },
            mode_required: match class {
                DeviceClass::PipewireSocket => 0o660,
                _ => 0o660,
            },
            group_required: match class {
                DeviceClass::Kvm => "kvm",
                DeviceClass::NetTun => "d2bd",
                DeviceClass::VhostNet => "kvm",
                DeviceClass::Fuse => "d2bd",
                DeviceClass::Dri => "render",
                DeviceClass::NvidiaCtl | DeviceClass::NvidiaUvm | DeviceClass::NvidiaRender => {
                    "video"
                }
                DeviceClass::PipewireSocket => "pipewire",
                DeviceClass::UsbipHost => "root",
                DeviceClass::Tpm => "tss",
                DeviceClass::Vfio => "vfio",
                // Cross-domain Wayland's /dev/udmabuf is rendered
                // through the render group on stock distros; matches
                // the kernel module ownership.
                DeviceClass::Udmabuf => "render",
            }
            .to_owned(),
            required: true,
            rationale: format!("live open for {}", class.default_path()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_host::devices::DeviceNodeKind;
    use std::path::PathBuf;

    fn matrix_kvm() -> DeviceMatrixEntry {
        default_matrix_entry(DeviceClass::Kvm)
    }

    fn good_readback() -> DeviceNodeReadback {
        DeviceNodeReadback {
            path: PathBuf::from("/dev/kvm"),
            exists: true,
            kind: Some(DeviceNodeKind::CharacterDevice),
            mode: 0o660,
            uid: 0,
            gid: 36,
            group_name: Some("kvm".to_owned()),
        }
    }

    #[test]
    fn allowed_when_role_declares_class_and_matrix_validates() {
        let role = RoleDeviceClaim {
            role_id: "kvm-runner".into(),
            allowed_classes: vec![DeviceClass::Kvm],
        };
        let decision = pre_open_decision(
            DeviceClass::Kvm,
            &role,
            &[matrix_kvm().entry.clone()],
            &good_readback(),
        );
        assert_eq!(decision, PreOpenDecision::Allow);
    }

    #[test]
    fn denied_role_class_mismatch() {
        let role = RoleDeviceClaim {
            role_id: "audit-only".into(),
            allowed_classes: vec![],
        };
        let decision = pre_open_decision(
            DeviceClass::Kvm,
            &role,
            &[matrix_kvm().entry.clone()],
            &good_readback(),
        );
        assert_eq!(decision, PreOpenDecision::DeniedRoleClassMismatch);
    }

    #[test]
    fn denied_not_in_matrix_for_undeclared_path() {
        let role = RoleDeviceClaim {
            role_id: "kvm-runner".into(),
            allowed_classes: vec![DeviceClass::Kvm],
        };
        let mut readback = good_readback();
        readback.path = PathBuf::from("/dev/sg0");
        let decision = pre_open_decision(
            DeviceClass::Kvm,
            &role,
            &[matrix_kvm().entry.clone()],
            &readback,
        );
        assert_eq!(decision, PreOpenDecision::DeniedNotInMatrix);
    }

    #[test]
    fn denied_validation_propagates() {
        let role = RoleDeviceClaim {
            role_id: "kvm-runner".into(),
            allowed_classes: vec![DeviceClass::Kvm],
        };
        let mut readback = good_readback();
        readback.mode = 0o666;
        let decision = pre_open_decision(
            DeviceClass::Kvm,
            &role,
            &[matrix_kvm().entry.clone()],
            &readback,
        );
        assert_eq!(
            decision,
            PreOpenDecision::DeniedValidation(DeviceValidation::LooseMode)
        );
    }

    #[test]
    fn post_open_validation_rejects_major_minor_mismatch() {
        let err =
            validate_opened_device(&matrix_kvm(), DeviceNodeKind::CharacterDevice, Some((1, 3)))
                .unwrap_err();
        assert_eq!(
            err,
            DeviceOpenValidationError::DeviceMajorMinorMismatch {
                expected: (10, 232),
                actual: (1, 3),
            }
        );
    }

    #[test]
    fn post_open_validation_rejects_kind_mismatch() {
        let err =
            validate_opened_device(&matrix_kvm(), DeviceNodeKind::BlockDevice, Some((10, 232)))
                .unwrap_err();
        assert_eq!(
            err,
            DeviceOpenValidationError::DeviceKindMismatch {
                expected: DeviceNodeKind::CharacterDevice,
                actual: "block-device".to_owned(),
            }
        );
    }

    #[test]
    fn open_device_fd_against_dev_null_succeeds() {
        let fd = open_device_fd(Path::new("/dev/null"), true).expect("open /dev/null");
        // Ownership transferred to fd; drop closes it.
        drop(fd);
    }
}
