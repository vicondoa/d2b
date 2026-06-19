//! Pure helpers for qemu-media physical USB handling.
//!
//! This module has no syscall side effects. The privileged broker owns live
//! sysfs/procfs reads, registry writes, udev rule reloads, and fd opening.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

const MEDIA_REF_MAX: usize = 63;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaRefError {
    Empty,
    TooLong,
    BadStart,
    BadCharacter,
}

impl fmt::Display for MediaRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "media ref must not be empty"),
            Self::TooLong => write!(f, "media ref must be at most {MEDIA_REF_MAX} bytes"),
            Self::BadStart => write!(f, "media ref must start with a lowercase ASCII letter"),
            Self::BadCharacter => {
                write!(
                    f,
                    "media ref may contain only lowercase ASCII, digits, and '-'"
                )
            }
        }
    }
}

impl std::error::Error for MediaRefError {}

pub fn validate_media_ref(value: &str) -> Result<(), MediaRefError> {
    if value.is_empty() {
        return Err(MediaRefError::Empty);
    }
    if value.len() > MEDIA_REF_MAX {
        return Err(MediaRefError::TooLong);
    }
    let mut bytes = value.bytes();
    let first = bytes.next().ok_or(MediaRefError::Empty)?;
    if !first.is_ascii_lowercase() {
        return Err(MediaRefError::BadStart);
    }
    if !bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        return Err(MediaRefError::BadCharacter);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusIdError {
    Empty,
    TooLong,
    BadCharacter,
}

impl fmt::Display for BusIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "USB busid must not be empty"),
            Self::TooLong => write!(f, "USB busid is too long"),
            Self::BadCharacter => write!(f, "USB busid contains an invalid character"),
        }
    }
}

impl std::error::Error for BusIdError {}

/// Validate the Linux USB busid shape used under `/sys/bus/usb/devices/`.
pub fn validate_usb_busid(value: &str) -> Result<(), BusIdError> {
    if value.is_empty() {
        return Err(BusIdError::Empty);
    }
    if value.len() > 64 {
        return Err(BusIdError::TooLong);
    }
    if !value
        .bytes()
        .all(|b| b.is_ascii_digit() || b == b'-' || b == b'.')
    {
        return Err(BusIdError::BadCharacter);
    }
    if value.starts_with('-')
        || value.ends_with('-')
        || value.ends_with('.')
        || !value.contains('-')
    {
        return Err(BusIdError::BadCharacter);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevnumError {
    Empty,
    Invalid,
    Zero,
}

pub fn parse_devnum(value: &str) -> Result<u16, DevnumError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(DevnumError::Empty);
    }
    let parsed = trimmed.parse::<u16>().map_err(|_| DevnumError::Invalid)?;
    if parsed == 0 {
        return Err(DevnumError::Zero);
    }
    Ok(parsed)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbPhysicalIdentity {
    pub bus_id: String,
    pub devnum: u16,
    pub vendor_id: String,
    pub product_id: String,
    pub by_id_names: Vec<String>,
    pub block_device: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByIdNameError {
    Empty,
    ContainsSlash,
    ContainsNul,
}

pub fn by_id_name_from_path(path: &Path) -> Result<String, ByIdNameError> {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Err(ByIdNameError::Empty);
    };
    if name.is_empty() {
        return Err(ByIdNameError::Empty);
    }
    if name.contains('/') {
        return Err(ByIdNameError::ContainsSlash);
    }
    if name.contains('\0') {
        return Err(ByIdNameError::ContainsNul);
    }
    Ok(name.to_owned())
}

pub fn sysfs_usb_device_dir(sysfs_root: &Path, bus_id: &str) -> PathBuf {
    sysfs_root.join("bus/usb/devices").join(bus_id)
}

pub fn sysfs_block_device_dir(sysfs_root: &Path, block_device: &str) -> PathBuf {
    sysfs_root.join("block").join(block_device)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceUse {
    Mounted { source: String },
    Swap { source: String },
    Holder { holder: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevicePreflight {
    pub uses: Vec<DeviceUse>,
}

impl DevicePreflight {
    pub fn is_clear(&self) -> bool {
        self.uses.is_empty()
    }
}

pub fn preflight_device_not_in_use(
    block_device: &str,
    mounts: &str,
    swaps: &str,
    holders: &[String],
) -> DevicePreflight {
    let mut uses = Vec::new();
    for source in mounted_sources_for_device(block_device, mounts) {
        uses.push(DeviceUse::Mounted { source });
    }
    for source in swap_sources_for_device(block_device, swaps) {
        uses.push(DeviceUse::Swap { source });
    }
    for holder in holders {
        if !holder.is_empty() {
            uses.push(DeviceUse::Holder {
                holder: holder.clone(),
            });
        }
    }
    DevicePreflight { uses }
}

pub fn mounted_sources_for_device(block_device: &str, mounts: &str) -> Vec<String> {
    mounts
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|source| dev_source_matches(block_device, source))
        .map(str::to_owned)
        .collect()
}

pub fn swap_sources_for_device(block_device: &str, swaps: &str) -> Vec<String> {
    swaps
        .lines()
        .skip(1)
        .filter_map(|line| line.split_whitespace().next())
        .filter(|source| dev_source_matches(block_device, source))
        .map(str::to_owned)
        .collect()
}

fn dev_source_matches(block_device: &str, source: &str) -> bool {
    let Some(dev_name) = source.strip_prefix("/dev/") else {
        return false;
    };
    if dev_name == block_device {
        return true;
    }
    let Some(rest) = dev_name.strip_prefix(block_device) else {
        return false;
    };
    rest.as_bytes()
        .first()
        .is_some_and(|b| b.is_ascii_digit() || *b == b'p')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAccessMode {
    ReadOnly,
    ReadWrite,
}

/// Initial open flags required before a block device can be handed to QEMU.
pub fn initial_open_flags(access: MediaAccessMode) -> i32 {
    match access {
        MediaAccessMode::ReadOnly => libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        MediaAccessMode::ReadWrite => {
            libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_EXCL
        }
    }
}

/// QEMU must still receive a read-only blockdev flag when policy is read-only.
pub fn qemu_read_only_required(access: MediaAccessMode) -> bool {
    matches!(access, MediaAccessMode::ReadOnly)
}

pub fn redacted_enrollment_summary(vm: &str, media_ref: &str, read_only: bool) -> String {
    format!(
        "nixling usb enroll --apply: enrolled media ref '{}' for vm '{}' (access={})",
        media_ref,
        vm,
        if read_only { "read-only" } else { "read-write" }
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeUsbCandidate {
    pub bus_id: String,
    pub block_device: String,
    pub by_id_names: Vec<String>,
}

/// Best-effort non-privileged scan for USB block devices that are safe to offer
/// as enrollment candidates. Raw by-id names stay inside the return value for
/// tests and broker-side matching; callers that render operator output should
/// expose only `bus_id`.
pub fn safe_usb_block_candidates(sysfs_root: &Path, by_id_root: &Path) -> Vec<SafeUsbCandidate> {
    let devices_root = sysfs_root.join("bus/usb/devices");
    let Ok(entries) = std::fs::read_dir(&devices_root) else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let Some(bus_id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if validate_usb_busid(&bus_id).is_err() {
            continue;
        }
        let usb_dir = entry.path();
        if !usb_dir.join("idVendor").is_file() || !usb_dir.join("idProduct").is_file() {
            continue;
        }
        let mut block_devices = BTreeSet::new();
        collect_block_devices_under(&usb_dir, 0, &mut block_devices);
        if let Some(parent) = usb_dir.parent()
            && let Ok(siblings) = std::fs::read_dir(parent)
        {
            let prefix = format!("{bus_id}:");
            for sibling in siblings.flatten() {
                if sibling
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with(&prefix))
                {
                    collect_block_devices_under(&sibling.path(), 0, &mut block_devices);
                }
            }
        }
        let mut block_devices: Vec<_> = block_devices.into_iter().collect();
        if block_devices.len() != 1 {
            continue;
        }
        let block_device = block_devices.pop().expect("len checked");
        let by_id_names = by_id_names_for_block(by_id_root, &block_device);
        if by_id_names.is_empty() {
            continue;
        }
        candidates.push(SafeUsbCandidate {
            bus_id,
            block_device,
            by_id_names,
        });
    }
    candidates.sort_by(|left, right| left.bus_id.cmp(&right.bus_id));
    candidates
}

fn collect_block_devices_under(path: &Path, depth: u8, out: &mut BTreeSet<String>) {
    if depth > 12 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        if entry.file_name() == "block" {
            if let Ok(blocks) = std::fs::read_dir(entry.path()) {
                for block in blocks.flatten() {
                    if let Some(name) = block.file_name().to_str()
                        && !name.is_empty()
                    {
                        out.insert(name.to_owned());
                    }
                }
            }
            continue;
        }
        collect_block_devices_under(&entry.path(), depth + 1, out);
    }
}

fn by_id_names_for_block(by_id_root: &Path, block_device: &str) -> Vec<String> {
    let expected = PathBuf::from("/dev").join(block_device);
    let mut names = Vec::new();
    let Ok(entries) = std::fs::read_dir(by_id_root) else {
        return names;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(target) = std::fs::canonicalize(&path) else {
            continue;
        };
        if target == expected
            && let Ok(name) = by_id_name_from_path(&path)
        {
            names.push(name);
        }
    }
    names.sort();
    names.dedup();
    names
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QemuMediaHotplugAction {
    Attach,
    Detach,
}

impl QemuMediaHotplugAction {
    pub fn qmp_commands(self) -> &'static [&'static str] {
        match self {
            Self::Attach => &["blockdev-add", "device_add"],
            Self::Detach => &["device_del", "blockdev-del"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QemuMediaHotplugScaffold {
    pub media_ref: String,
    pub slot: String,
    pub blockdev_id: String,
    pub device_id: String,
    pub qmp_commands: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QemuMediaHotplugScaffoldError {
    InvalidMediaRef,
    EmptySlot,
    InvalidSlot,
}

pub fn qemu_media_hotplug_scaffold(
    media_ref: &str,
    slot: &str,
    action: QemuMediaHotplugAction,
) -> Result<QemuMediaHotplugScaffold, QemuMediaHotplugScaffoldError> {
    validate_media_ref(media_ref).map_err(|_| QemuMediaHotplugScaffoldError::InvalidMediaRef)?;
    if slot.is_empty() {
        return Err(QemuMediaHotplugScaffoldError::EmptySlot);
    }
    if slot.len() > 63
        || !slot
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(QemuMediaHotplugScaffoldError::InvalidSlot);
    }
    Ok(QemuMediaHotplugScaffold {
        media_ref: media_ref.to_owned(),
        slot: slot.to_owned(),
        blockdev_id: format!("nl-media-{media_ref}"),
        device_id: format!("nl-usb-{media_ref}"),
        qmp_commands: action
            .qmp_commands()
            .iter()
            .map(|command| (*command).to_owned())
            .collect(),
    })
}

#[cfg(test)]
mod hotplug_tests {
    use super::*;

    #[test]
    fn qmp_scaffold_uses_only_opaque_ref_derived_ids() {
        let plan =
            qemu_media_hotplug_scaffold("installer-usb", "cdrom", QemuMediaHotplugAction::Attach)
                .expect("scaffold");

        assert_eq!(plan.blockdev_id, "nl-media-installer-usb");
        assert_eq!(plan.device_id, "nl-usb-installer-usb");
        assert_eq!(plan.qmp_commands, ["blockdev-add", "device_add"]);
    }

    #[test]
    fn qmp_scaffold_rejects_path_like_refs_and_slots() {
        assert!(matches!(
            qemu_media_hotplug_scaffold(
                "/dev/disk/by-id/secret",
                "cdrom",
                QemuMediaHotplugAction::Attach
            ),
            Err(QemuMediaHotplugScaffoldError::InvalidMediaRef)
        ));
        assert!(matches!(
            qemu_media_hotplug_scaffold(
                "installer-usb",
                "../cdrom",
                QemuMediaHotplugAction::Attach
            ),
            Err(QemuMediaHotplugScaffoldError::InvalidSlot)
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_refs_are_opaque_single_components() {
        assert!(validate_media_ref("installer-usb").is_ok());
        assert_eq!(validate_media_ref(""), Err(MediaRefError::Empty));
        assert_eq!(
            validate_media_ref("Installer"),
            Err(MediaRefError::BadStart)
        );
        assert_eq!(
            validate_media_ref("installer_usb"),
            Err(MediaRefError::BadCharacter)
        );
        assert_eq!(
            validate_media_ref("a".repeat(64).as_str()),
            Err(MediaRefError::TooLong)
        );
    }

    #[test]
    fn busids_accept_sysfs_shape_only() {
        assert!(validate_usb_busid("1-2.3").is_ok());
        assert_eq!(validate_usb_busid("../1-2"), Err(BusIdError::BadCharacter));
        assert_eq!(validate_usb_busid("1-2:1.0"), Err(BusIdError::BadCharacter));
    }

    #[test]
    fn devnum_readback_is_positive_decimal() {
        assert_eq!(parse_devnum("007\n"), Ok(7));
        assert_eq!(parse_devnum("0"), Err(DevnumError::Zero));
        assert_eq!(parse_devnum("abc"), Err(DevnumError::Invalid));
    }

    #[test]
    fn by_id_readback_uses_basename_only() {
        assert_eq!(
            by_id_name_from_path(Path::new("/dev/disk/by-id/usb-test-serial")).unwrap(),
            "usb-test-serial"
        );
        assert_eq!(
            by_id_name_from_path(Path::new("/")),
            Err(ByIdNameError::Empty)
        );
    }

    #[test]
    fn preflight_catches_whole_disk_partitions_swaps_and_holders() {
        let mounts = "/dev/sdb1 /media/usb ext4 rw 0 0\n/dev/nvme0n1p2 /mnt xfs rw 0 0\n";
        let swaps = "Filename\tType\tSize\tUsed\tPriority\n/dev/sdb2 partition 1 0 -2\n";
        let holders = vec!["dm-0".to_owned()];
        let report = preflight_device_not_in_use("sdb", mounts, swaps, &holders);
        assert_eq!(report.uses.len(), 3);
        assert!(!report.is_clear());
        assert!(preflight_device_not_in_use("sdc", mounts, swaps, &[]).is_clear());
    }

    #[test]
    fn initial_open_flags_enforce_exclusive_writes_and_read_only_scaffold() {
        let ro = initial_open_flags(MediaAccessMode::ReadOnly);
        assert_ne!(ro & libc::O_RDONLY, libc::O_RDWR);
        assert_ne!(ro & libc::O_NOFOLLOW, 0);
        assert_eq!(ro & libc::O_EXCL, 0);
        assert!(qemu_read_only_required(MediaAccessMode::ReadOnly));

        let rw = initial_open_flags(MediaAccessMode::ReadWrite);
        assert_ne!(rw & libc::O_RDWR, 0);
        assert_ne!(rw & libc::O_EXCL, 0);
        assert!(!qemu_read_only_required(MediaAccessMode::ReadWrite));
    }

    #[test]
    fn enrollment_summary_does_not_echo_raw_busid_or_by_id() {
        let summary = redacted_enrollment_summary("media-vm", "installer-usb", true);
        assert!(summary.contains("installer-usb"));
        assert!(!summary.contains("1-2.3"));
        assert!(!summary.contains("usb-Vendor_Serial"));
        assert!(!summary.contains("/dev/"));
    }
}
