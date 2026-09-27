//! The guest image's declared invocation.
//!
//! The image action evaluates the re-homed guest node and writes down what it
//! found: the machine size, the drive layout, the boot method with its kernel
//! command line already resolved, the per-check device options, and the
//! activation contract. That manifest is the only record of the invocation.
//! The launcher renders it and never restates a number the node declared, so
//! a check that asks for four vCPUs, eight gigabytes of memory, and a vsock
//! device gets exactly that, and a check that asks for something else gets
//! something else.

use std::{fs, path::{Path, PathBuf}};

use serde::Deserialize;

use crate::error::{HarnessError, Result};

/// The manifest schema this launcher reads. A manifest that declares a
/// different one is refused rather than guessed at.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// The guest image's declared invocation, as the image action wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestManifest {
    /// The manifest schema version.
    pub schema_version: u32,
    /// The system the guest closure was realized for.
    pub system: String,
    /// Which of the re-homed node's two guest shapes this image evaluates.
    pub node_shape: String,
    /// The files the launcher boots, relative to the image root.
    pub image: ImageFiles,
    /// The check this guest was built for, and whether that check boots a
    /// nested guest inside it. `None` on a shape-only image, which carries
    /// no check.
    pub check: Option<CheckRecord>,
    /// What one pool member costs the host.
    pub footprint: Footprint,
    /// The bound the lane's pool is sized against, declared next to the
    /// guests rather than supplied by a caller.
    pub pool: PoolBudget,
    /// The machine size the node declared.
    pub machine: Machine,
    /// How the guest is booted, and with what.
    pub boot: Boot,
    /// The block devices the launcher attaches, in the node's order.
    pub drives: Vec<Drive>,
    /// The per-check device options, in the node's order, with the VM
    /// module's own direct-boot directives already resolved into [`Boot`].
    pub extra_options: Vec<String>,
    /// The networking options the node declared, with the VM module's own
    /// shell substitution resolved out.
    ///
    /// A separate field from [`Self::extra_options`] because the VM module
    /// renders them in a separate place in its own run script, and a guest
    /// booted without them has no network device at all - which the daemon
    /// refuses to start over, by name.
    pub networking_options: Vec<String>,
    /// The host directories the guest mounts over 9p.
    #[serde(default)]
    pub shared_directories: Vec<SharedDirectory>,
    /// The activation contract the launcher waits for.
    pub activation: Activation,
    /// The guest's init, as the closure names it.
    pub init: String,
    /// The guest's system closure, as the closure names it.
    pub toplevel: String,
    /// The host-tool package the guest closure was built against.
    pub host_tool_bundle: String,
    /// The binaries that package carries.
    pub host_tool_inventory: Vec<String>,
    /// The Cloud Hypervisor controller, when the image carried one.
    pub cloud_hypervisor_controller: Option<String>,
}

/// The files the launcher boots, relative to the image root.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageFiles {
    /// The guest's root disk.
    pub disk: String,
    /// The format that disk is in.
    pub disk_format: String,
    /// How large the node asked the disk to be.
    pub disk_size_mib: u64,
    /// The installed system image the root disk is a writable overlay on, for
    /// the shape that boots through a bootloader.
    pub system_image: Option<String>,
}

/// The check a guest image was built for.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckRecord {
    /// The check's name, as the lane reports it and as a contributor filters
    /// it.
    pub name: String,
    /// The name the check's own fixture gave it, which is what appears in a
    /// driver log line.
    pub test_name: String,
    /// Whether this guest runs a guest of its own. A member that has is
    /// retired rather than restored, because restoring a guest with a live
    /// guest inside it is not defined behaviour.
    pub nested_guest: bool,
}

/// What one pool member costs the host, in the three currencies the pool is
/// bounded in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Footprint {
    /// The memory the member's guest reserves, in MiB.
    pub memory_size_mib: u64,
    /// The vCPUs the member's guest holds.
    pub cores: u32,
    /// The lane working directory the member needs for its own root disk, in
    /// MiB. The launcher copies that disk into the directory the member
    /// owns, so this is the disk the node declared rather than a number the
    /// lane chose.
    pub working_directory_mib: u64,
}

/// The share of the host the pool may take, as the guest configurations
/// declared it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PoolBudget {
    /// The numerator of the memory share.
    pub memory_share_numerator: u64,
    /// The denominator of the memory share.
    pub memory_share_denominator: u64,
    /// The numerator of the vCPU share.
    pub core_share_numerator: u64,
    /// The denominator of the vCPU share.
    pub core_share_denominator: u64,
    /// Whether a member's working directory follows the disk its node
    /// declared. Declared rather than assumed, because a node that grows its
    /// root disk grows the lane's working directory with it.
    pub working_directory_follows_disk: bool,
}

/// The machine size the node declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Machine {
    /// The vCPU count.
    pub cores: u32,
    /// The memory size, in MiB.
    pub memory_size_mib: u32,
}

/// How the guest is booted.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Boot {
    /// `direct` boots the kernel and initrd the image carries; `bootloader`
    /// boots the root disk through the bootloader the image was built with.
    pub method: String,
    /// The kernel to direct-boot, relative to the image root.
    pub kernel: Option<String>,
    /// The initrd to direct-boot, relative to the image root.
    pub initrd: Option<String>,
    /// The kernel command line, already resolved: the guest's own kernel
    /// parameters, its init, the store registration the activation reads,
    /// and the console list the node declared.
    pub append: Option<String>,
}

/// One attached block device, in the shape the node declared it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Drive {
    /// The node's name for the drive, when it gave one.
    pub name: Option<String>,
    /// The backing file: an image-relative name, or an absolute path into
    /// the store for a device the node attaches itself.
    pub file: String,
    /// The image format, when the node pinned one. `None` leaves the
    /// emulator to read it from the file, exactly as the node asked.
    pub format: Option<String>,
    /// The cache mode.
    pub cache: String,
    /// Whether the emulator reports write errors.
    pub werror: String,
    /// The boot order index, when the node pinned one.
    pub boot_index: Option<String>,
    /// The serial the guest identifies the device by.
    pub serial: Option<String>,
    /// The bus the device is attached to.
    pub interface: String,
}

/// A drive the guest node attached through its option list and asked the
/// emulator to shadow with a throwaway overlay.
///
/// The node declares one of these as a rendered `-drive` with `snapshot=on`
/// rather than as a drive of its own, so it arrives as text rather than as
/// structure. What the lane needs from it is the store image the overlay
/// sits on, because that image is what a member's snapshot restores the
/// device to: the overlay the emulator created is named after the emulator's
/// own temporary directory and is gone on the next run, while the store
/// image behind it is the same for every member and every check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EphemeralDrive {
    /// The store image the overlay sits on.
    pub file: String,
    /// The format that image is in.
    pub format: String,
    /// The cache mode the node declared for the drive.
    pub cache: String,
    /// The bus the device is attached to.
    pub interface: String,
}

/// One host directory the guest mounts over 9p.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SharedDirectory {
    /// The mount tag the guest sees.
    pub mount_tag: String,
    /// The 9p security model.
    pub security_model: String,
    /// Where the guest mounts it.
    pub target: String,
    /// The host path, exactly as the node declared it. A `$TMPDIR`-relative
    /// source stays relative; the launcher resolves it against the working
    /// directory it owns.
    pub source: String,
}

/// The activation contract, in the repository's own field names.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Activation {
    /// The readiness signal the guest reports on.
    pub readiness: String,
    /// What that signal means.
    pub contract: String,
    /// The line the guest writes once the contract is met.
    pub marker: String,
    /// The serial device that line arrives on.
    pub serial_device: String,
    /// Where inside the guest the units the contract names are declared.
    pub acceptance_units_file: String,
    /// The shape the guest reports, so a launcher that booted the wrong
    /// image finds out from the guest rather than from a later failure.
    pub shape: String,
}

impl GuestManifest {
    /// Read the manifest out of a built guest image.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    pub fn load(image_dir: &Path) -> Result<Self> {
        let path = image_dir.join("manifest.json");
        let text = fs::read_to_string(&path)
            .map_err(|error| HarnessError::io(format!("reading {}", path.display()), error))?;
        let manifest: Self = serde_json::from_str(&text).map_err(|error| {
            HarnessError::Manifest {
                path: PathBuf::from(&path),
                detail: error.to_string(),
            }
        })?;
        if manifest.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(HarnessError::Manifest {
                path,
                detail: format!(
                    "manifest schema {} is not the version this launcher reads ({SUPPORTED_SCHEMA_VERSION})",
                    manifest.schema_version
                ),
            });
        }
        Ok(manifest)
    }

    /// The path of a file the manifest names relative to the image root.
    pub fn resolve(&self, image_dir: &Path, file: &str) -> PathBuf {
        let declared = Path::new(file);
        if declared.is_absolute() {
            declared.to_path_buf()
        } else {
            image_dir.join(declared)
        }
    }

    /// Whether the guest boots through a bootloader rather than being handed
    /// a kernel.
    pub fn uses_bootloader(&self) -> bool {
        self.boot.method == "bootloader"
    }

    /// The identity of the emulator invocation this guest is booted with.
    ///
    /// Two checks share a pool member when their guests are booted the same
    /// way, and that is a statement about the invocation alone: the machine
    /// size, the root disk, the boot method, the drives, the per-check device
    /// options, and the networking. The closure deliberately does not appear
    /// in it. The system closure and the host-tool package are what make one
    /// guest a different guest from another, and a group of checks that
    /// shared a closure would be a group that shared an image - which is the
    /// thing the lane stopped doing, because it is why one guest could not
    /// serve a check that wanted nftables, a vsock device, or a nested guest.
    pub fn invocation_key(&self) -> String {
        let drives: Vec<serde_json::Value> = self
            .drives
            .iter()
            .map(|drive| {
                serde_json::json!({
                    "interface": drive.interface,
                    "cache": drive.cache,
                    "werror": drive.werror,
                    "bootIndex": drive.boot_index,
                    "serial": drive.serial,
                    "format": drive.format,
                    // An image-relative file is the guest's own root disk and
                    // is the same file for every guest; a store path is a
                    // device the node attached itself, and the path is what
                    // says which one.
                    "ownDisk": !Path::new(&drive.file).is_absolute(),
                })
            })
            .collect();
        let key = serde_json::json!({
            "method": self.boot.method,
            "memorySizeMib": self.machine.memory_size_mib,
            "cores": self.machine.cores,
            "diskSizeMib": self.image.disk_size_mib,
            "bootloader": self.image.system_image.is_some(),
            "drives": drives,
            "extraOptions": self.extra_options,
            "networkingOptions": self.networking_options,
            "shares": self
                .shared_directories
                .iter()
                .map(|share| (share.mount_tag.clone(), share.security_model.clone()))
                .collect::<Vec<_>>(),
        });
        serde_json::to_string(&key).unwrap_or_default()
    }

    /// Refuse a guest whose attached writable devices cannot carry an
    /// internal snapshot, before a pool is built around it.
    ///
    /// The emulator's own verdict is the authority - it is asked again over
    /// the monitor once the guest is running - but it can only be asked of a
    /// running guest, and a pool that boots five members and then discovers
    /// the sixth cannot be snapshotted has already spent the run it was
    /// supposed to explain. This is the pass that reads it off the
    /// declaration instead: in the current emulator qcow2 is the only
    /// format with the snapshot vtable, a single writable node without it
    /// fails the save for the whole guest rather than for that node, and a
    /// node that asks for an ephemeral overlay is one whose writes live in a
    /// qcow2 the emulator creates for it.
    pub fn unsnapshottable_drives(&self) -> Vec<String> {
        let mut refused: Vec<String> = Vec::new();
        for drive in &self.drives {
            let own_disk = !Path::new(&drive.file).is_absolute();
            let image_format = own_disk.then_some(self.image.disk_format.as_str());
            if !matches!(drive.format.as_deref().or(image_format), Some("qcow2")) {
                refused.push(format!(
                    "{} ({})",
                    drive.file,
                    drive.format.as_deref().unwrap_or("no format")
                ));
            }
        }
        // A drive the node attached through the option list rather than
        // through the drive list: the state disk the re-homed node mounts at
        // `/var/lib/d2b` is declared exactly this way.
        for (index, option) in self.extra_options.iter().enumerate() {
            if !option.starts_with("file=") {
                continue;
            }
            let option_is_drive = self
                .extra_options
                .get(index.wrapping_sub(1))
                .is_some_and(|previous| previous == "-drive");
            if !option_is_drive {
                continue;
            }
            if !option.contains("snapshot=on")
                && !option.contains("format=qcow2")
                && !option.contains("readonly=on")
            {
                refused.push(option.clone());
            }
        }
        refused
    }

    /// The drives the node attached through its option list rather than
    /// through its drive list, as drives the lane can reason about.
    ///
    /// The state disk the re-homed node mounts at `/var/lib/d2b` is declared
    /// exactly this way: a `-drive` rendered by the VM module with
    /// `snapshot=on`, which asks the emulator for a throwaway overlay over
    /// the store image. That overlay is a real device with a real node, and a
    /// member's snapshot has to cover it: `/var/lib/d2b` holds the daemon's
    /// store, so a snapshot that left it out would hand the second check on a
    /// member the first check's rows.
    pub fn ephemeral_drives(&self) -> Vec<EphemeralDrive> {
        let mut drives = Vec::new();
        for (index, option) in self.extra_options.iter().enumerate() {
            if !option.starts_with("file=")
                || !self
                    .extra_options
                    .get(index.wrapping_sub(1))
                    .is_some_and(|previous| previous == "-drive")
            {
                continue;
            }
            if !option.contains("snapshot=on") {
                continue;
            }
            let read = |key: &str| {
                option
                    .split(',')
                    .find_map(|field| field.strip_prefix(&format!("{key}=")))
                    .map(str::to_owned)
            };
            let (Some(file), Some(format)) = (read("file"), read("format")) else {
                continue;
            };
            drives.push(EphemeralDrive {
                file,
                format,
                cache: read("cache").unwrap_or_else(|| "writeback".to_owned()),
                interface: match read("if").as_deref() {
                    Some("scsi") => "scsi".to_owned(),
                    Some("ide") => "ide".to_owned(),
                    _ => "virtio".to_owned(),
                },
            });
        }
        drives
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
      "schemaVersion": 1,
      "system": "x86_64-linux",
      "nodeShape": "daemon",
      "image": {"disk": "disk.qcow2", "diskFormat": "qcow2", "diskSizeMib": 8192, "systemImage": null},
      "check": {"name": "daemon-smoke", "testName": "d2b-daemon-smoke", "nestedGuest": false},
      "footprint": {"memorySizeMib": 3072, "cores": 3, "workingDirectoryMib": 8192},
      "pool": {
        "memoryShareNumerator": 2, "memoryShareDenominator": 3,
        "coreShareNumerator": 3, "coreShareDenominator": 4,
        "workingDirectoryFollowsDisk": true
      },
      "machine": {"cores": 3, "memorySizeMib": 3072},
      "boot": {"method": "direct", "kernel": "kernel", "initrd": "initrd", "append": "console=ttyS0"},
      "drives": [
        {"name": "root", "file": "disk.qcow2", "format": null, "cache": "writeback",
         "werror": "report", "bootIndex": "1", "serial": "root", "interface": "virtio"}
      ],
      "extraOptions": ["-device", "virtio-keyboard", "-usb"],
      "networkingOptions": ["-net nic,netdev=user.0,model=virtio", "-netdev user,id=user.0,"],
      "sharedDirectories": [
        {"mountTag": "nix-store", "securityModel": "none", "target": "/nix/.ro-store", "source": "/nix/store"}
      ],
      "activation": {
        "readiness": "console-marker",
        "contract": "d2b-daemon-acceptance",
        "marker": "D2B_LANE_READY",
        "serialDevice": "ttyS0",
        "acceptanceUnitsFile": "/etc/d2b/daemon-acceptance-units",
        "shape": "daemon"
      },
      "init": "/nix/store/x-nixos-system/init",
      "toplevel": "/nix/store/x-nixos-system",
      "hostToolBundle": "/nix/store/y-d2b-bazel-host-tools-0.0.0",
      "hostToolInventory": ["d2b", "d2bd"],
      "cloudHypervisorController": null
    }"#;

    #[test]
    fn the_manifest_round_trips_into_the_declared_invocation() {
        let manifest: GuestManifest = serde_json::from_str(MANIFEST).expect("the manifest parses");
        assert_eq!(manifest.machine.cores, 3);
        assert_eq!(manifest.machine.memory_size_mib, 3072);
        assert_eq!(manifest.image.disk_size_mib, 8192);
        assert_eq!(manifest.drives[0].file, "disk.qcow2");
        assert!(!manifest.uses_bootloader());
        assert_eq!(manifest.activation.marker, "D2B_LANE_READY");
        assert_eq!(
            manifest.networking_options,
            vec![
                "-net nic,netdev=user.0,model=virtio".to_owned(),
                "-netdev user,id=user.0,".to_owned()
            ],
            "the node's networking declaration survives into the manifest whole"
        );
    }

    #[test]
    fn a_manifest_field_the_launcher_does_not_know_is_refused() {
        // A manifest that grew a field is a manifest whose meaning the
        // launcher cannot honour; guessing is how a lane boots a guest
        // shaped like the wrong check.
        let drifted = MANIFEST.replace(
            "\"cores\": 3",
            "\"cores\": 3, \"machineIdentity\": \"whatever\"",
        );
        let error = serde_json::from_str::<GuestManifest>(&drifted)
            .expect_err("an unknown field is refused");
        assert!(error.to_string().contains("machineIdentity"), "{error}");
    }

    #[test]
    fn an_image_relative_file_resolves_against_the_image_and_a_store_path_does_not() {
        let manifest: GuestManifest = serde_json::from_str(MANIFEST).expect("the manifest parses");
        assert_eq!(
            manifest.resolve(Path::new("/run/lane/image"), "disk.qcow2"),
            Path::new("/run/lane/image/disk.qcow2")
        );
        assert_eq!(
            manifest.resolve(Path::new("/run/lane/image"), "/nix/store/z-state.img"),
            Path::new("/nix/store/z-state.img")
        );
    }
}
