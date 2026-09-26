//! The lane's guest self-check.
//!
//! One invocation proves the three things the lane's own harness owns,
//! against a real guest built by the guest-image action from the re-homed
//! guest node:
//!
//!   1. the host satisfies every virtualization precondition, or the lane
//!      stops before a guest starts;
//!   2. the guest boots with exactly the invocation its configuration
//!      declares, reaches its activation contract, and attaches only
//!      devices that can carry a snapshot;
//!   3. tearing it down leaves no emulator process and no working directory
//!      behind, and doing that repeatedly changes nothing.
//!
//! Everything it needs arrives as runfile paths in the environment, because
//! that is how the Bazel test target hands it the emulator from the pinned
//! nix package set and the guest image from the guest-image action.

use std::{env, fs, path::{Path, PathBuf}, process::ExitCode, time::Duration};

use d2b_vm_harness::{
    GuestSpec, HarnessError, boot, host, manifest::GuestManifest, report,
};
use serde_json::json;

/// The image the lane was pointed at. Supplied by the Bazel test target as a
/// runfile path.
const IMAGE: &str = "D2B_VM_HARNESS_IMAGE";
/// The emulator binary, from the pinned nix package set the guest closure was
/// realized from.
const EMULATOR: &str = "D2B_VM_HARNESS_EMULATOR";
/// The lane's working directory, which outlives each individual guest.
const WORK_ROOT: &str = "D2B_VM_HARNESS_WORK_ROOT";
/// How long the guest has to activate.
const ACTIVATION_TIMEOUT: &str = "D2B_VM_HARNESS_ACTIVATION_TIMEOUT_SECS";
/// How many boot and teardown cycles to run.
const CYCLES: &str = "D2B_VM_HARNESS_CYCLES";

/// The block-graph node name the boot-time snapshot-capability proof attaches
/// its unsnapshottable device under.
const REFUSAL_NODE: &str = "lane_refusal";

/// The device id the proof attaches. The emulator reports a hot-attached
/// device by its qdev path rather than by this id, so the id exists to be
/// legible in the emulator's own diagnostics.
const REFUSAL_DEVICE: &str = "lane-refusal";

/// How large the proof's raw device is. The size is irrelevant to the
/// judgement - the format is what has no snapshot support - and only has to
/// be big enough for the guest to accept the disk.
const REFUSAL_DEVICE_BYTES: u64 = 8 * 1024 * 1024;

fn main() -> ExitCode {
    match run() {
        Ok(report) => {
            for line in report {
                report_line(&line);
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            report_line(&format!("FAIL {error}"));
            ExitCode::FAILURE
        }
    }
}

/// The lane's working directory, resolved against the caller's own directory
/// when the caller named a relative one. The lane's directory has to outlive
/// each individual guest, so it cannot be a per-action temporary directory
/// the current Bazel release does not expose to a sandboxed action.
fn work_root(caller_dir: std::path::PathBuf) -> Result<PathBuf, HarnessError> {
    let declared = required_path(WORK_ROOT)?;
    Ok(if declared.is_absolute() {
        declared
    } else {
        caller_dir.join(declared)
    })
}

fn report_line(line: &str) {
    report(line);
}

fn run() -> Result<Vec<String>, HarnessError> {
    let image_dir = required_path(IMAGE)?;
    let emulator = required_path(EMULATOR)?;
    let work_root = work_root(env::current_dir().map_err(|error| {
        HarnessError::io("reading the lane's working directory", error)
    })?)?;
    let activation_timeout = Duration::from_secs(optional_u64(ACTIVATION_TIMEOUT, 1200)?);
    let cycles = optional_u64(CYCLES, 1)?;

    // The host preconditions come first and gate everything after them: a
    // host that cannot run the lane must stop before a guest is built or
    // booted, not degrade into a slower run.
    let facts = host::require_this_host()?;
    let mut lines = vec![format!(
        "host: /dev/kvm usable, nested virtualization and nested-state save present ({})",
        facts
            .module_parameters
            .iter()
            .map(|(name, value)| format!("{name}={}", value.as_deref().unwrap_or("absent")))
            .collect::<Vec<_>>()
            .join(" ")
    )];

    let manifest = GuestManifest::load(&image_dir)?;
    let contract = manifest.activation.contract.clone();
    lines.push(format!(
        "image: shape {} booted {} with {} vCPU and {} MiB of memory on a {} MiB {} disk",
        manifest.node_shape,
        manifest.boot.method,
        manifest.machine.cores,
        manifest.machine.memory_size_mib,
        manifest.image.disk_size_mib,
        manifest.image.disk_format,
    ));

    let mut spec = GuestSpec::new(
        manifest,
        image_dir,
        emulator,
        work_root,
        "lane-harness",
    );
    spec.activation_timeout = activation_timeout;
    let work_dir = spec.work_dir();

    for cycle in 0..cycles {
        lines.push(format!("cycle {}: booting", cycle + 1));
        let mut guest = boot(&spec)?;
        guest.require_snapshottable()?;
        lines.push(format!(
            "cycle {}: activated ({contract}), every attached writable device carries a snapshot",
            cycle + 1,
        ));
        guest.shutdown()?;
        require_removed(&work_dir)?;
        lines.push(format!(
            "cycle {}: torn down, no emulator process and no working directory left",
            cycle + 1
        ));
    }

    lines.push(refuse_unsnapshottable_device(&spec)?);
    Ok(lines)
}

/// A guest that left its working directory behind has not been torn down, and
/// a second cycle would inherit whatever the first one left.
fn require_removed(work_dir: &Path) -> Result<(), HarnessError> {
    if work_dir.exists() {
        return Err(HarnessError::Configuration(format!(
            "{} survived teardown",
            work_dir.display()
        )));
    }
    Ok(())
}

/// Prove, against a real booted guest, that a writable device the lane
/// cannot snapshot stops the lane at boot.
///
/// The device is a raw image inside the guest's own working directory,
/// attached through the monitor the way any other device is attached, and it
/// is the case the refusal exists for: a writable node whose format has no
/// internal snapshot support, which a later `snapshot-save` would refuse for
/// the whole guest rather than for that one node. The guest is judged once
/// before the device is attached and once after, so the refusal is
/// attributable to the device rather than to the guest.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn refuse_unsnapshottable_device(spec: &GuestSpec) -> Result<String, HarnessError> {
    let work_dir = spec.work_dir();
    let mut guest = boot(spec)?;
    guest.require_snapshottable()?;

    let image = work_dir.join("refusal.img");
    let backing = fs::File::create(&image)
        .map_err(|error| HarnessError::io(format!("creating {}", image.display()), error))?;
    backing
        .set_len(REFUSAL_DEVICE_BYTES)
        .map_err(|error| HarnessError::io(format!("sizing {}", image.display()), error))?;
    drop(backing);

    let monitor = guest
        .monitor()
        .ok_or_else(|| HarnessError::Configuration("the guest has no monitor".to_owned()))?;
    monitor.execute_with(
        "blockdev-add",
        json!({
            "node-name": REFUSAL_NODE,
            "driver": "raw",
            "file": { "driver": "file", "filename": image.to_string_lossy() },
        }),
    )?;
    monitor.execute_with(
        "device_add",
        json!({
            "driver": "virtio-blk-pci",
            "drive": REFUSAL_NODE,
            "id": REFUSAL_DEVICE,
        }),
    )?;

    let error = guest
        .require_snapshottable()
        .expect_err("a writable raw device stops the lane at boot");
    let HarnessError::NotSnapshottable { devices } = error else {
        return Err(HarnessError::Configuration(format!(
            "a writable raw device was refused for the wrong reason: {error}"
        )));
    };
    let named = devices
        .iter()
        .map(|device| device.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let refused = devices
        .iter()
        .find(|device| device.file == image.to_string_lossy())
        .ok_or_else(|| {
            HarnessError::Configuration(format!(
                "the refusal did not name the device the lane attached, but named: {named}"
            ))
        })?
        .clone();
    guest.shutdown()?;
    require_removed(&work_dir)?;
    Ok(format!(
        "boot-time snapshot capability: a writable raw device stops the lane before any check runs \
         ({} bytes at {}, refused as {} on {})",
        REFUSAL_DEVICE_BYTES,
        refused.file,
        refused.format,
        refused.device,
    ))
}

fn required_path(name: &str) -> Result<PathBuf, HarnessError> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| HarnessError::Configuration(format!("{name} is not set")))
}

fn optional_u64(name: &str, default: u64) -> Result<u64, HarnessError> {
    match env::var(name) {
        Err(_) => Ok(default),
        Ok(value) if value.is_empty() => Ok(default),
        Ok(value) => value.parse().map_err(|error| {
            HarnessError::Configuration(format!("{name} is not a number: {value} ({error})"))
        }),
    }
}
