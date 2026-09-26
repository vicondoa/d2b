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
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
      "schemaVersion": 1,
      "system": "x86_64-linux",
      "nodeShape": "daemon",
      "image": {"disk": "disk.qcow2", "diskFormat": "qcow2", "diskSizeMib": 8192, "systemImage": null},
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
