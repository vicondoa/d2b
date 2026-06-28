//! Audio policy dispatch for `d2b audio status`, `set-volume`, and `mute`.
//!
//! Resolves the per-VM provider capability row before touching local state:
//!
//! * **Cloud Hypervisor NixOS** – OFD-locked local state I/O, host PipeWire
//!   enforcement via a `pw-cli`/`wpctl` subprocess (credential-aware; see
//!   [`audio_host_controller::PipeWireHostController`]), guest enforcement via
//!   guestd audio RPCs over the authenticated guest-control transport.
//! * **qemu-media** – OFD-locked local state I/O, offline state-file policy.
//!   Guest enforcement always reported `Unsupported`. No guestd calls.
//!
//! All provider-internal resource IDs and credentials are redacted from
//! public responses. Volume/gain values never appear in audit records,
//! metric labels, or log messages.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt as _;
use std::os::unix::io::AsRawFd as _;
use std::path::Path;

use nix::fcntl::{FcntlArg, fcntl};

use d2b_contracts::guest_proto as pb;
use d2b_contracts::public_wire::{
    AudioChannel, AudioChannelState, AudioEnforcementPosture, AudioErrorKind, AudioMuteArgs,
    AudioOp, AudioOpResponse, AudioProviderKind, AudioSetApplied, AudioSetResult,
    AudioSetVolumeArgs, AudioStatusArgs, AudioStatusResult, AudioVmError, AudioVmState,
};
use d2b_core::audio_policy::{
    AudioGrant, AudioPolicyError, AudioPolicyState, LevelPercent, parse_audio_state,
};
use d2b_core::manifest_v04::{ManifestV04, VmEntry as ManifestVmEntry};
use d2b_core::processes::ProcessesJson;
use d2b_core::provider_capabilities::{
    AudioGuestEnforcementKind, AudioHostEnforcementKind, AudioProviderCapability,
};
use d2b_core::runtime::{RuntimeKind, RuntimeProviderDriver};
use serde_json::Value;

use crate::ServerState;
use crate::TypedError;
use crate::audio_host_controller::{
    HostAudioController, PipeWireHostController, QemuAudioController,
};
use crate::guest_control_bridge::{
    GUEST_CONTROL_AUDIO_SET_TIMEOUT, run_audio_set_on_dedicated_thread,
    run_audio_status_on_dedicated_thread,
};
use crate::guest_control_health::{GuestAudioSetError, GuestAudioStatus};

// ── Lock path ────────────────────────────────────────────────────────────────

/// Path of the per-VM OFD lock file.
fn audio_lock_path(locks_dir: &Path, vm: &str) -> std::path::PathBuf {
    locks_dir.join(format!("audio-{vm}.lock"))
}

/// Path of the per-VM audio-state file.
fn audio_state_path(state_dir: &Path) -> std::path::PathBuf {
    state_dir.join("state/audio-state.json")
}

// ── OFD lock helpers ─────────────────────────────────────────────────────────

/// Acquire a Linux OFD lock on `fd`.
///
/// `exclusive = true`  → F_OFD_SETLKW write-lock (blocking).
/// `exclusive = false` → F_OFD_SETLKW read-lock  (blocking).
///
/// The file descriptor must have been opened with `O_CLOEXEC` so exec'd
/// children do not inherit the lock.
fn ofd_lock(fd: std::os::unix::io::RawFd, exclusive: bool) -> io::Result<()> {
    let ltype = if exclusive {
        libc::F_WRLCK as libc::c_short
    } else {
        libc::F_RDLCK as libc::c_short
    };
    let fl = libc::flock {
        l_type: ltype,
        l_whence: libc::SEEK_SET as libc::c_short,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    fcntl(fd, FcntlArg::F_OFD_SETLKW(&fl))
        .map(|_| ())
        .map_err(|e| io::Error::from_raw_os_error(e as i32))
}

/// Unlock an OFD lock held on `fd`.
///
/// Uses `F_OFD_SETLK` (non-blocking) rather than `F_OFD_SETLKW`: unlocking
/// never needs to wait and using the blocking variant is incorrect for the
/// release path.
fn ofd_unlock(fd: std::os::unix::io::RawFd) -> io::Result<()> {
    let fl = libc::flock {
        l_type: libc::F_UNLCK as libc::c_short,
        l_whence: libc::SEEK_SET as libc::c_short,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    fcntl(fd, FcntlArg::F_OFD_SETLK(&fl))
        .map(|_| ())
        .map_err(|e| io::Error::from_raw_os_error(e as i32))
}

/// RAII guard that releases an OFD lock when dropped.
struct OfdLockGuard {
    fd: std::os::unix::io::RawFd,
}

impl Drop for OfdLockGuard {
    fn drop(&mut self) {
        let _ = ofd_unlock(self.fd);
    }
}

struct AudioStateLock {
    _file: File,
    _guard: OfdLockGuard,
}

fn acquire_audio_state_lock(
    lock_path: &Path,
    exclusive: bool,
) -> Result<AudioStateLock, AudioStateIoError> {
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(lock_path)
        .map_err(AudioStateIoError::LockOpen)?;
    let fd = lock_file.as_raw_fd();
    ofd_lock(fd, exclusive).map_err(AudioStateIoError::LockAcquire)?;
    Ok(AudioStateLock {
        _file: lock_file,
        _guard: OfdLockGuard { fd },
    })
}

// ── Audio state I/O ──────────────────────────────────────────────────────────

/// Error from audio state file I/O.
#[derive(Debug)]
pub enum AudioStateIoError {
    LockOpen(io::Error),
    LockAcquire(io::Error),
    StateRead(io::Error),
    StateParse(AudioPolicyError),
    TempFile(io::Error),
    TempWrite(io::Error),
    TempSync(io::Error),
    AtomicRename(io::Error),
}

impl std::fmt::Display for AudioStateIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LockOpen(e) => write!(f, "open audio lock file: {e}"),
            Self::LockAcquire(e) => write!(f, "acquire audio OFD lock: {e}"),
            Self::StateRead(e) => write!(f, "read audio state file: {e}"),
            Self::StateParse(e) => write!(f, "parse audio state: {e}"),
            Self::TempFile(e) => write!(f, "create audio state temp file: {e}"),
            Self::TempWrite(e) => write!(f, "write audio state temp file: {e}"),
            Self::TempSync(e) => write!(f, "sync audio state temp file: {e}"),
            Self::AtomicRename(e) => write!(f, "atomic rename audio state: {e}"),
        }
    }
}

/// Read the current audio state under a shared OFD lock.
///
/// Opens `lock_path` with `O_RDONLY|O_CLOEXEC|O_CREAT` (the lock file is
/// pre-created by systemd-tmpfiles, but we tolerate it being absent during
/// tests). Acquires a shared lock, reads and parses the state file, then
/// releases the lock.
///
/// Returns `AudioPolicyState::default_v2()` when the state file is absent.
pub fn read_audio_state_locked(
    lock_path: &Path,
    state_path: &Path,
) -> Result<AudioPolicyState, AudioStateIoError> {
    let _lock = acquire_audio_state_lock(lock_path, false)?;
    read_audio_state_unlocked(state_path)
}

fn read_audio_state_unlocked(state_path: &Path) -> Result<AudioPolicyState, AudioStateIoError> {
    let bytes = match std::fs::read(state_path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(AudioPolicyState::default_v2());
        }
        Err(e) => return Err(AudioStateIoError::StateRead(e)),
    };

    parse_audio_state(&bytes).map_err(AudioStateIoError::StateParse)
}

/// Write a new audio state atomically under an exclusive OFD lock.
///
/// The write path:
/// 1. Open the lock file and acquire an exclusive OFD lock.
/// 2. Serialize the new state to v2 JSON.
/// 3. Write to a `.tmp` file in the same directory (ensuring same-fs rename).
/// 4. `fsync` the temp file.
/// 5. `rename` temp → state file (atomic on the same fs).
/// 6. Release the lock via the RAII guard.
#[cfg(test)]
fn write_audio_state_locked(
    lock_path: &Path,
    state_path: &Path,
    state: &AudioPolicyState,
) -> Result<(), AudioStateIoError> {
    let _lock = acquire_audio_state_lock(lock_path, true)?;
    write_audio_state_unlocked(state_path, state)
}

fn write_audio_state_unlocked(
    state_path: &Path,
    state: &AudioPolicyState,
) -> Result<(), AudioStateIoError> {
    use std::io::Write as _;
    let bytes = state.to_v2_bytes().map_err(AudioStateIoError::StateParse)?;

    // Place the temp file in the same directory to guarantee same-filesystem
    // rename (hardlinks cannot cross mount points).
    let parent = state_path.parent().unwrap_or(Path::new("."));
    let tmp_path = parent.join("audio-state.json.tmp");

    let mut tmp_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(&tmp_path)
        .map_err(AudioStateIoError::TempFile)?;

    tmp_file
        .write_all(&bytes)
        .map_err(AudioStateIoError::TempWrite)?;

    // Ensure the data reaches stable storage before rename.
    tmp_file.sync_data().map_err(AudioStateIoError::TempSync)?;
    drop(tmp_file);

    std::fs::rename(&tmp_path, state_path).map_err(AudioStateIoError::AtomicRename)?;

    Ok(())
}

// ── Provider capability resolution ───────────────────────────────────────────

/// Resolve the audio capability row for a VM manifest entry.
///
/// Returns `None` when the VM does not have `audio = true`.
pub fn audio_capability_for_vm(vm: &ManifestVmEntry) -> Option<AudioProviderCapability> {
    if !vm.audio {
        return None;
    }
    let cap = match vm.runtime.kind {
        RuntimeKind::Nixos => match vm.runtime.provider.driver {
            RuntimeProviderDriver::CloudHypervisor | RuntimeProviderDriver::Crosvm => {
                AudioProviderCapability::cloud_hypervisor_nixos()
            }
            RuntimeProviderDriver::Qemu => AudioProviderCapability::qemu_media(),
        },
        RuntimeKind::QemuMedia => AudioProviderCapability::qemu_media(),
    };
    Some(cap)
}

/// Map provider capability host enforcement to the public `AudioProviderKind`.
fn public_provider_kind(cap: &AudioProviderCapability) -> AudioProviderKind {
    match cap.host_enforcement {
        AudioHostEnforcementKind::None => AudioProviderKind::AcaSandbox,
        AudioHostEnforcementKind::PipeWireVhostUserSound => AudioProviderKind::LocalHypervisor,
        AudioHostEnforcementKind::QemuAudioBackend => AudioProviderKind::QemuMedia,
    }
}

/// Map provider capability to the public enforcement posture.
fn public_enforcement_posture(cap: &AudioProviderCapability) -> AudioEnforcementPosture {
    match (cap.host_enforcement, cap.guest_enforcement) {
        (AudioHostEnforcementKind::None, AudioGuestEnforcementKind::GuestdCapable) => {
            AudioEnforcementPosture::GuestOnly
        }
        (_, AudioGuestEnforcementKind::Unsupported) => AudioEnforcementPosture::HostOnly,
        _ => AudioEnforcementPosture::HostAndGuest,
    }
}

// ── Host enforcement ─────────────────────────────────────────────────────────

/// Result of a host-side audio enforcement call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum HostEnforcementResult {
    Applied,
    Unsupported,
    Failed,
}

/// Build the host controller for a VM based on its audio capability row.
///
/// * For `PipeWireVhostUserSound` providers (Cloud Hypervisor NixOS), reads
///   the audio ProcessNode from `processes.json` to extract `WPCTL_PATH` and
///   `PIPEWIRE_RUNTIME_DIR` and returns a [`PipeWireHostController`]. Falls
///   back to returning `Unsupported` if the node or required env vars are
///   absent — this is a configuration error, not a runtime failure.
///
/// * For `QemuAudioBackend` providers, returns a [`QemuAudioController`]
///   which commits offline policy and returns `Applied` immediately.
///
/// * For `None` (ACA sandboxes), no host enforcement is performed; callers
///   should skip the controller entirely.
fn build_host_controller(
    state: &ServerState,
    vm_name: &str,
    cap: &AudioProviderCapability,
) -> Option<Box<dyn HostAudioController>> {
    match cap.host_enforcement {
        AudioHostEnforcementKind::PipeWireVhostUserSound => {
            // Load processes.json and find the audio runner node for this VM.
            let processes: ProcessesJson =
                match crate::load_json(&state.config.artifacts.processes_path) {
                    Ok(p) => p,
                    Err(_) => {
                        tracing::warn!(
                            vm = vm_name,
                            "failed to load processes.json; PipeWire host enforcement unavailable"
                        );
                        return None;
                    }
                };
            let audio_node = PipeWireHostController::find_audio_node(&processes, vm_name)?;
            PipeWireHostController::from_audio_node(audio_node)
                .map(|ctrl| -> Box<dyn HostAudioController> { Box::new(ctrl) })
        }
        AudioHostEnforcementKind::QemuAudioBackend => Some(Box::new(QemuAudioController)),
        AudioHostEnforcementKind::None => {
            // ACA sandboxes: no host enforcement; caller skips the controller.
            None
        }
    }
}

/// Apply host-side audio grant (mute/unmute) using the appropriate controller.
///
/// Returns `Unsupported` when no controller is available (ACA or configuration
/// gap). Returns `Failed` when the controller is present but enforcement failed
/// (subprocess error, credential failure, etc.) so callers know the host
/// boundary was NOT sealed for `off` requests.
pub fn enforce_host_grant(
    state: &ServerState,
    vm_name: &str,
    cap: &AudioProviderCapability,
    grant: AudioGrant,
    channel: AudioChannel,
) -> HostEnforcementResult {
    match build_host_controller(state, vm_name, cap) {
        Some(ctrl) => ctrl.enforce_grant(vm_name, grant, channel),
        None => HostEnforcementResult::Unsupported,
    }
}

/// Apply host-side audio level change using the appropriate controller.
///
/// Returns `Unsupported` when no controller is available.
pub fn enforce_host_level(
    state: &ServerState,
    vm_name: &str,
    cap: &AudioProviderCapability,
    level: LevelPercent,
    channel: AudioChannel,
) -> HostEnforcementResult {
    match build_host_controller(state, vm_name, cap) {
        Some(ctrl) => ctrl.enforce_level(vm_name, level, channel),
        None => HostEnforcementResult::Unsupported,
    }
}

// ── State → public wire mapping ───────────────────────────────────────────────

fn state_to_channel(grant: AudioGrant, level: Option<LevelPercent>) -> AudioChannelState {
    AudioChannelState {
        muted: !grant.is_on(),
        level,
    }
}

fn state_to_vm_state(
    vm: &str,
    state: &AudioPolicyState,
    cap: &AudioProviderCapability,
) -> AudioVmState {
    AudioVmState {
        vm: vm.to_owned(),
        speaker: state_to_channel(state.speaker, state.speaker_level),
        microphone: state_to_channel(state.mic, state.mic_gain),
        provider_kind: public_provider_kind(cap),
        enforcement: public_enforcement_posture(cap),
    }
}

// ── Guest enforcement result ──────────────────────────────────────────────────

/// Result of a guest-side (guestd AudioSet RPC) enforcement call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestEnforcementResult {
    /// guestd applied the change successfully.
    Applied,
    /// guestd was not reachable or the capability was not advertised.
    Unavailable,
    /// guestd was reachable but returned an error (PipeWire unavailable, etc.).
    Failed,
}

// ── Enforcement result → AudioSetApplied mapping ──────────────────────────────

/// Combine host and guest enforcement results into the public
/// [`AudioSetApplied`] outcome.
///
/// This function is `pub(crate)` so the test suite can lock the mapping
/// without needing a full [`crate::ServerState`].
pub(crate) fn combined_audio_applied(
    host_result: HostEnforcementResult,
    guest_result: GuestEnforcementResult,
    cap: &AudioProviderCapability,
) -> AudioSetApplied {
    match (cap.host_enforcement, cap.guest_enforcement) {
        // ACA sandbox: guest-only enforcement path.
        (AudioHostEnforcementKind::None, AudioGuestEnforcementKind::GuestdCapable) => {
            match guest_result {
                GuestEnforcementResult::Applied => AudioSetApplied::GuestOnly,
                _ => AudioSetApplied::Unsupported,
            }
        }
        // qemu-media: host-only enforcement (no guestd).
        (_, AudioGuestEnforcementKind::Unsupported) => match host_result {
            HostEnforcementResult::Applied => AudioSetApplied::HostOnly,
            _ => AudioSetApplied::Unsupported,
        },
        // Cloud Hypervisor NixOS: both host and guest paths.
        _ => match (host_result, guest_result) {
            (HostEnforcementResult::Applied, GuestEnforcementResult::Applied) => {
                AudioSetApplied::HostAndGuest
            }
            (HostEnforcementResult::Applied, _) => AudioSetApplied::HostOnly,
            (_, GuestEnforcementResult::Applied) => AudioSetApplied::GuestOnly,
            _ => AudioSetApplied::Unsupported,
        },
    }
}

/// Issue a guestd AudioSet (mute/unmute) call for a VM.
///
/// Returns `Unavailable` when the bundle cannot be resolved, the VM has no
/// vsock socket, or the capability is not advertised. Returns `Failed` when
/// the guestd call is reachable but fails (e.g. PipeWire unavailable in the
/// guest).
fn enforce_guest_grant(
    state: &ServerState,
    vm_name: &str,
    grant: AudioGrant,
    channel: AudioChannel,
) -> GuestEnforcementResult {
    let wire_channel = match channel {
        AudioChannel::Speaker => pb::AudioChannel::AUDIO_CHANNEL_SPEAKER,
        AudioChannel::Microphone => pb::AudioChannel::AUDIO_CHANNEL_MICROPHONE,
    };
    run_guestd_audio_set(
        state,
        vm_name,
        wire_channel,
        pb::AudioSetKind::AUDIO_SET_KIND_GRANT,
        grant.is_on(),
        0,
    )
}

/// Issue a guestd AudioSet (volume) call for a VM.
fn enforce_guest_level(
    state: &ServerState,
    vm_name: &str,
    level: LevelPercent,
    channel: AudioChannel,
) -> GuestEnforcementResult {
    let wire_channel = match channel {
        AudioChannel::Speaker => pb::AudioChannel::AUDIO_CHANNEL_SPEAKER,
        AudioChannel::Microphone => pb::AudioChannel::AUDIO_CHANNEL_MICROPHONE,
    };
    run_guestd_audio_set(
        state,
        vm_name,
        wire_channel,
        pb::AudioSetKind::AUDIO_SET_KIND_LEVEL,
        false,
        level.get().into(),
    )
}

fn run_guestd_audio_set(
    state: &ServerState,
    vm_name: &str,
    channel: pb::AudioChannel,
    kind: pb::AudioSetKind,
    grant_on: bool,
    level: u32,
) -> GuestEnforcementResult {
    let resolver = match crate::load_bundle_resolver(state) {
        Ok(r) => r,
        Err(_) => {
            tracing::debug!(vm = vm_name, "audio guestd: bundle resolver unavailable");
            return GuestEnforcementResult::Unavailable;
        }
    };
    let params = match crate::resolve_guest_control_probe_params(state, &resolver, vm_name) {
        Ok(p) => p,
        Err(reason) => {
            tracing::debug!(vm = vm_name, %reason, "audio guestd: probe params unresolved");
            return GuestEnforcementResult::Unavailable;
        }
    };
    let broker_path = crate::broker_socket_path(state);

    match run_audio_set_on_dedicated_thread(
        params,
        broker_path,
        channel,
        kind,
        grant_on,
        level,
        GUEST_CONTROL_AUDIO_SET_TIMEOUT,
    ) {
        Ok(_) => GuestEnforcementResult::Applied,
        Err(GuestAudioSetError::CapabilityUnavailable) => {
            tracing::debug!(
                vm = vm_name,
                "audio guestd: AudioSet capability not advertised"
            );
            GuestEnforcementResult::Unavailable
        }
        Err(e) => {
            tracing::warn!(
                vm = vm_name,
                error = ?e,
                "audio guestd: AudioSet RPC failed"
            );
            GuestEnforcementResult::Failed
        }
    }
}

// ── dispatch_audio ────────────────────────────────────────────────────────────

pub fn dispatch_audio(state: &ServerState, op: AudioOp) -> Result<Value, TypedError> {
    match op {
        AudioOp::Status(args) => dispatch_audio_status(state, args),
        AudioOp::SetVolume(args) => dispatch_audio_set_volume(state, args),
        AudioOp::Mute(args) => dispatch_audio_mute(state, args),
    }
}

// ── Status ─────────────────────────────────────────────────────────────────

fn dispatch_audio_status(state: &ServerState, args: AudioStatusArgs) -> Result<Value, TypedError> {
    let manifest: ManifestV04 = crate::load_json(&state.config.artifacts.public_manifest_path)?;
    let mut entries: Vec<AudioVmState> = Vec::new();
    let mut errors: Vec<AudioVmError> = Vec::new();

    // Collect the set of VMs to query.
    let vm_names: Vec<String> = if args.vms.is_empty() {
        manifest
            .vms
            .iter()
            .filter(|(_, v)| v.audio)
            .map(|(k, _)| k.clone())
            .collect()
    } else {
        args.vms.clone()
    };

    for vm_name in &vm_names {
        match resolve_vm_audio_status(state, vm_name, &manifest) {
            Ok(vm_state) => entries.push(vm_state),
            Err(vm_error) => errors.push(vm_error),
        }
    }

    let result = AudioStatusResult { entries, errors };
    Ok(crate::wire::audio_response(&AudioOpResponse::Status(
        result,
    )))
}

fn resolve_vm_audio_status(
    state: &ServerState,
    vm_name: &str,
    manifest: &ManifestV04,
) -> Result<AudioVmState, AudioVmError> {
    let vm = manifest.vms.get(vm_name).ok_or_else(|| AudioVmError {
        vm: vm_name.to_owned(),
        kind: AudioErrorKind::VmNotFound,
        remediation: None,
    })?;

    let cap = audio_capability_for_vm(vm).ok_or_else(|| AudioVmError {
        vm: vm_name.to_owned(),
        kind: AudioErrorKind::AudioNotEnabled,
        remediation: Some(
            "enable audio for this VM with `d2b.vms.<name>.audio.enable = true`".to_owned(),
        ),
    })?;

    // Read local state under OFD lock.
    let state_dir = std::path::PathBuf::from(&vm.state_dir);
    let lock_path = audio_lock_path(&state.config.locks_dir, vm_name);
    let state_path = audio_state_path(&state_dir);

    let audio_state = read_audio_state_locked(&lock_path, &state_path).map_err(|e| {
        tracing::warn!(vm = vm_name, error = %e, "failed to read audio state");
        AudioVmError {
            vm: vm_name.to_owned(),
            kind: AudioErrorKind::InternalError,
            remediation: None,
        }
    })?;

    let mut vm_state = state_to_vm_state(vm_name, &audio_state, &cap);
    if matches!(
        cap.guest_enforcement,
        AudioGuestEnforcementKind::GuestdCapable
    ) {
        if let Some(guest_status) = query_guest_audio_status(state, vm_name) {
            apply_guest_status(&mut vm_state, guest_status);
        } else {
            vm_state.enforcement = match cap.host_enforcement {
                AudioHostEnforcementKind::None => AudioEnforcementPosture::Unsupported,
                _ => AudioEnforcementPosture::HostOnly,
            };
        }
    }
    Ok(vm_state)
}

fn query_guest_audio_status(state: &ServerState, vm_name: &str) -> Option<GuestAudioStatus> {
    let resolver = crate::load_bundle_resolver(state).ok()?;
    let params = crate::resolve_guest_control_probe_params(state, &resolver, vm_name).ok()?;
    run_audio_status_on_dedicated_thread(
        params,
        crate::broker_socket_path(state),
        GUEST_CONTROL_AUDIO_SET_TIMEOUT,
    )
    .ok()
}

fn apply_guest_status(vm_state: &mut AudioVmState, guest_status: GuestAudioStatus) {
    vm_state.microphone.muted = guest_status.microphone.muted;
    if guest_status.microphone.level_known {
        vm_state.microphone.level = u8::try_from(guest_status.microphone.level)
            .ok()
            .and_then(|level| LevelPercent::new(level).ok());
    }
    vm_state.speaker.muted = guest_status.speaker.muted;
    if guest_status.speaker.level_known {
        vm_state.speaker.level = u8::try_from(guest_status.speaker.level)
            .ok()
            .and_then(|level| LevelPercent::new(level).ok());
    }
}

// ── SetVolume ─────────────────────────────────────────────────────────────────

fn dispatch_audio_set_volume(
    state: &ServerState,
    args: AudioSetVolumeArgs,
) -> Result<Value, TypedError> {
    let vm_name = &args.vm;
    let channel = args.channel;
    let level = args.level;

    let manifest: ManifestV04 = crate::load_json(&state.config.artifacts.public_manifest_path)?;

    let vm = manifest
        .vms
        .get(vm_name)
        .ok_or_else(|| TypedError::InternalIo {
            context: format!("audio set-volume {vm_name}"),
            detail: "VM not present in public manifest".to_owned(),
        })?;

    let cap = audio_capability_for_vm(vm).ok_or_else(|| TypedError::InternalIo {
        context: format!("audio set-volume {vm_name}"),
        detail: "audio not enabled for this VM".to_owned(),
    })?;

    let state_dir = std::path::PathBuf::from(&vm.state_dir);
    let lock_path = audio_lock_path(&state.config.locks_dir, vm_name);
    let state_path = audio_state_path(&state_dir);

    let _state_lock =
        acquire_audio_state_lock(&lock_path, true).map_err(|e| TypedError::InternalIo {
            context: "acquire audio state lock".to_owned(),
            detail: e.to_string(),
        })?;
    let current = read_audio_state_unlocked(&state_path).map_err(|e| TypedError::InternalIo {
        context: "read audio state".to_owned(),
        detail: e.to_string(),
    })?;

    let old_level = match channel {
        AudioChannel::Speaker => current.speaker_level,
        AudioChannel::Microphone => current.mic_gain,
    };
    let new_state = match channel {
        AudioChannel::Speaker => current.with_speaker_level(level),
        AudioChannel::Microphone => current.with_mic_gain(level),
    };
    let level_increase = old_level.map(|old| level.get() > old.get()).unwrap_or(true);

    // For live PipeWire enforcement, prove the host boundary update before
    // persisting an increased level as applied. Missing live nodes report
    // Unsupported and still allow the offline boot policy to be staged.
    if !level_increase {
        write_audio_state_unlocked(&state_path, &new_state).map_err(|e| {
            TypedError::InternalIo {
                context: "write audio state".to_owned(),
                detail: e.to_string(),
            }
        })?;
    }

    let host_result = if cap.host_enforcement == AudioHostEnforcementKind::PipeWireVhostUserSound {
        let result = enforce_host_level(state, vm_name, &cap, level, channel);
        if level_increase && matches!(result, HostEnforcementResult::Failed) {
            return Err(TypedError::InternalIo {
                context: "audio host enforcement".to_owned(),
                detail: "host level enforcement failed; state not updated".to_owned(),
            });
        }
        result
    } else {
        HostEnforcementResult::Unsupported
    };

    if level_increase {
        write_audio_state_unlocked(&state_path, &new_state).map_err(|e| {
            TypedError::InternalIo {
                context: "write audio state".to_owned(),
                detail: e.to_string(),
            }
        })?;
    }

    let host_result = if cap.host_enforcement == AudioHostEnforcementKind::QemuAudioBackend {
        enforce_host_level(state, vm_name, &cap, level, channel)
    } else {
        host_result
    };

    // Guest enforcement for guestd-capable VMs (CH NixOS). qemu never calls
    // guestd. ACA has no local state file and calls guestd only.
    let guest_result = if cap.guest_enforcement == AudioGuestEnforcementKind::GuestdCapable {
        enforce_guest_level(state, vm_name, level, channel)
    } else {
        GuestEnforcementResult::Unavailable
    };

    let applied = combined_audio_applied(host_result, guest_result, &cap);

    let channel_state = match channel {
        AudioChannel::Speaker => state_to_channel(new_state.speaker, new_state.speaker_level),
        AudioChannel::Microphone => state_to_channel(new_state.mic, new_state.mic_gain),
    };

    Ok(crate::wire::audio_response(&AudioOpResponse::SetVolume(
        AudioSetResult {
            vm: vm_name.clone(),
            channel,
            applied,
            state: channel_state,
        },
    )))
}

// ── Mute ──────────────────────────────────────────────────────────────────────

fn dispatch_audio_mute(state: &ServerState, args: AudioMuteArgs) -> Result<Value, TypedError> {
    let vm_name = &args.vm;
    let channel = args.channel;
    let mute = args.mute;

    let manifest: ManifestV04 = crate::load_json(&state.config.artifacts.public_manifest_path)?;

    let vm = manifest
        .vms
        .get(vm_name)
        .ok_or_else(|| TypedError::InternalIo {
            context: format!("audio mute {vm_name}"),
            detail: "VM not present in public manifest".to_owned(),
        })?;

    let cap = audio_capability_for_vm(vm).ok_or_else(|| TypedError::InternalIo {
        context: format!("audio mute {vm_name}"),
        detail: "audio not enabled for this VM".to_owned(),
    })?;

    let state_dir = std::path::PathBuf::from(&vm.state_dir);
    let lock_path = audio_lock_path(&state.config.locks_dir, vm_name);
    let state_path = audio_state_path(&state_dir);

    let _state_lock =
        acquire_audio_state_lock(&lock_path, true).map_err(|e| TypedError::InternalIo {
            context: "acquire audio state lock".to_owned(),
            detail: e.to_string(),
        })?;
    let current = read_audio_state_unlocked(&state_path).map_err(|e| TypedError::InternalIo {
        context: "read audio state".to_owned(),
        detail: e.to_string(),
    })?;

    let grant = if mute {
        AudioGrant::Off
    } else {
        AudioGrant::On
    };
    let new_state = match channel {
        AudioChannel::Speaker => current.with_speaker(grant),
        AudioChannel::Microphone => current.with_mic(grant),
    };

    // Persist revocations before live enforcement so a failed live update still
    // boots with the restrictive policy. Enabling access still proves live host
    // enforcement before persisting the less-restrictive state.
    if grant == AudioGrant::Off {
        write_audio_state_unlocked(&state_path, &new_state).map_err(|e| {
            TypedError::InternalIo {
                context: "write audio state".to_owned(),
                detail: e.to_string(),
            }
        })?;
    }

    let host_result = if cap.host_enforcement == AudioHostEnforcementKind::PipeWireVhostUserSound {
        let result = enforce_host_grant(state, vm_name, &cap, grant, channel);
        if grant == AudioGrant::On && matches!(result, HostEnforcementResult::Failed) {
            return Err(TypedError::InternalIo {
                context: "audio host enforcement".to_owned(),
                detail: "host grant enforcement failed; state not updated".to_owned(),
            });
        }
        result
    } else {
        HostEnforcementResult::Unsupported
    };

    if grant == AudioGrant::On {
        write_audio_state_unlocked(&state_path, &new_state).map_err(|e| {
            TypedError::InternalIo {
                context: "write audio state".to_owned(),
                detail: e.to_string(),
            }
        })?;
    }

    let host_result = if cap.host_enforcement == AudioHostEnforcementKind::QemuAudioBackend {
        enforce_host_grant(state, vm_name, &cap, grant, channel)
    } else {
        host_result
    };

    // Guest enforcement for guestd-capable VMs. qemu never calls guestd.
    let guest_result = if cap.guest_enforcement == AudioGuestEnforcementKind::GuestdCapable {
        enforce_guest_grant(state, vm_name, grant, channel)
    } else {
        GuestEnforcementResult::Unavailable
    };

    let applied = combined_audio_applied(host_result, guest_result, &cap);

    let channel_state = match channel {
        AudioChannel::Speaker => state_to_channel(new_state.speaker, new_state.speaker_level),
        AudioChannel::Microphone => state_to_channel(new_state.mic, new_state.mic_gain),
    };

    Ok(crate::wire::audio_response(&AudioOpResponse::Mute(
        AudioSetResult {
            vm: vm_name.clone(),
            channel,
            applied,
            state: channel_state,
        },
    )))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_core::audio_policy::{AudioGrant, AudioPolicyState};

    // ── OFD lock tests ──────────────────────────────────────────────────────

    #[test]
    fn ofd_read_write_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lock_path = dir.path().join("audio-test.lock");
        let state_path = dir.path().join("state").join("audio-state.json");
        std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();

        // Write a state.
        let state = AudioPolicyState::default_v2()
            .with_mic(AudioGrant::On)
            .with_speaker(AudioGrant::Off)
            .with_speaker_level(LevelPercent::new(75).unwrap());
        write_audio_state_locked(&lock_path, &state_path, &state).expect("write state");

        // Read it back.
        let read_back = read_audio_state_locked(&lock_path, &state_path).expect("read state");
        assert_eq!(read_back.mic, AudioGrant::On);
        assert_eq!(read_back.speaker, AudioGrant::Off);
        assert_eq!(read_back.speaker_level.map(|l| l.get()), Some(75));
    }

    #[test]
    fn ofd_missing_state_returns_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lock_path = dir.path().join("audio-test.lock");
        let state_path = dir.path().join("audio-state.json");

        let state = read_audio_state_locked(&lock_path, &state_path).expect("read missing");
        assert_eq!(state, AudioPolicyState::default_v2());
    }

    #[test]
    fn write_is_atomic_rename() {
        // Verify the temp file is never left behind after a successful write.
        let dir = tempfile::tempdir().expect("tempdir");
        let lock_path = dir.path().join("audio.lock");
        let state_path = dir.path().join("audio-state.json");

        let state = AudioPolicyState::default_v2();
        write_audio_state_locked(&lock_path, &state_path, &state).unwrap();

        // .tmp file must not exist after the write.
        let tmp_path = dir.path().join("audio-state.json.tmp");
        assert!(
            !tmp_path.exists(),
            ".tmp file must not exist after atomic rename"
        );
        assert!(state_path.exists(), "state file must exist after write");
    }

    // ── Provider capability tests ───────────────────────────────────────────

    #[test]
    fn ch_nixos_cap_is_pipewire_guestd() {
        let cap = AudioProviderCapability::cloud_hypervisor_nixos();
        assert_eq!(
            cap.host_enforcement,
            AudioHostEnforcementKind::PipeWireVhostUserSound
        );
        assert_eq!(
            cap.guest_enforcement,
            AudioGuestEnforcementKind::GuestdCapable
        );
        assert!(cap.needs_local_state_file);
    }

    #[test]
    fn qemu_media_cap_is_host_only() {
        let cap = AudioProviderCapability::qemu_media();
        assert_eq!(
            cap.host_enforcement,
            AudioHostEnforcementKind::QemuAudioBackend
        );
        assert_eq!(
            cap.guest_enforcement,
            AudioGuestEnforcementKind::Unsupported
        );
        assert!(cap.needs_local_state_file);
    }

    #[test]
    fn aca_cap_is_guest_only_no_local_state() {
        let cap = AudioProviderCapability::aca_sandbox();
        assert_eq!(cap.host_enforcement, AudioHostEnforcementKind::None);
        assert_eq!(
            cap.guest_enforcement,
            AudioGuestEnforcementKind::GuestdCapable
        );
        assert!(!cap.needs_local_state_file);
    }

    #[test]
    fn enforcement_posture_mapping() {
        let ch_cap = AudioProviderCapability::cloud_hypervisor_nixos();
        assert_eq!(
            public_enforcement_posture(&ch_cap),
            AudioEnforcementPosture::HostAndGuest
        );

        let qemu_cap = AudioProviderCapability::qemu_media();
        assert_eq!(
            public_enforcement_posture(&qemu_cap),
            AudioEnforcementPosture::HostOnly
        );

        let aca_cap = AudioProviderCapability::aca_sandbox();
        assert_eq!(
            public_enforcement_posture(&aca_cap),
            AudioEnforcementPosture::GuestOnly
        );
    }

    // ── combined_audio_applied tests ────────────────────────────────────────
    //
    // These tests lock the combined host+guest result mapping. The key
    // invariants are:
    //   - CH NixOS: host+guest both succeed → HostAndGuest
    //   - CH NixOS: host succeeds, guest unavailable → HostOnly
    //   - CH NixOS: host fails → Unsupported (no success-shaped fallback)
    //   - qemu-media: host applied, guest unsupported → HostOnly
    //   - ACA sandbox: guest applied → GuestOnly; guest fails → Unsupported

    #[test]
    fn ch_nixos_host_and_guest_applied_returns_host_and_guest() {
        let cap = AudioProviderCapability::cloud_hypervisor_nixos();
        let result = combined_audio_applied(
            HostEnforcementResult::Applied,
            GuestEnforcementResult::Applied,
            &cap,
        );
        assert_eq!(result, AudioSetApplied::HostAndGuest);
    }

    #[test]
    fn ch_nixos_host_applied_guest_unavailable_returns_host_only() {
        let cap = AudioProviderCapability::cloud_hypervisor_nixos();
        let result = combined_audio_applied(
            HostEnforcementResult::Applied,
            GuestEnforcementResult::Unavailable,
            &cap,
        );
        assert_eq!(result, AudioSetApplied::HostOnly);
    }

    #[test]
    fn ch_nixos_host_applied_guest_failed_returns_host_only() {
        let cap = AudioProviderCapability::cloud_hypervisor_nixos();
        let result = combined_audio_applied(
            HostEnforcementResult::Applied,
            GuestEnforcementResult::Failed,
            &cap,
        );
        assert_eq!(result, AudioSetApplied::HostOnly);
    }

    #[test]
    fn ch_nixos_host_failed_guest_applied_returns_guest_only() {
        let cap = AudioProviderCapability::cloud_hypervisor_nixos();
        let result = combined_audio_applied(
            HostEnforcementResult::Failed,
            GuestEnforcementResult::Applied,
            &cap,
        );
        assert_eq!(result, AudioSetApplied::GuestOnly);
    }

    #[test]
    fn ch_nixos_both_failed_returns_unsupported_not_success() {
        let cap = AudioProviderCapability::cloud_hypervisor_nixos();
        let result = combined_audio_applied(
            HostEnforcementResult::Failed,
            GuestEnforcementResult::Failed,
            &cap,
        );
        assert_eq!(
            result,
            AudioSetApplied::Unsupported,
            "both failed must not produce success-shaped result"
        );
    }

    #[test]
    fn qemu_host_applied_returns_host_only() {
        let cap = AudioProviderCapability::qemu_media();
        let result = combined_audio_applied(
            HostEnforcementResult::Applied,
            GuestEnforcementResult::Unavailable, // qemu never sets guest result
            &cap,
        );
        assert_eq!(result, AudioSetApplied::HostOnly);
    }

    #[test]
    fn qemu_host_failed_returns_unsupported() {
        let cap = AudioProviderCapability::qemu_media();
        let result = combined_audio_applied(
            HostEnforcementResult::Failed,
            GuestEnforcementResult::Unavailable,
            &cap,
        );
        assert_eq!(result, AudioSetApplied::Unsupported);
    }

    #[test]
    fn aca_guest_applied_returns_guest_only() {
        let cap = AudioProviderCapability::aca_sandbox();
        let result = combined_audio_applied(
            HostEnforcementResult::Unsupported, // ACA has no host enforcement
            GuestEnforcementResult::Applied,
            &cap,
        );
        assert_eq!(result, AudioSetApplied::GuestOnly);
    }

    #[test]
    fn aca_guest_unavailable_returns_unsupported() {
        let cap = AudioProviderCapability::aca_sandbox();
        let result = combined_audio_applied(
            HostEnforcementResult::Unsupported,
            GuestEnforcementResult::Unavailable,
            &cap,
        );
        assert_eq!(result, AudioSetApplied::Unsupported);
    }

    #[test]
    fn no_success_shaped_fallback_for_both_unavailable() {
        // CH NixOS: both host and guest unavailable must not produce any success.
        let cap = AudioProviderCapability::cloud_hypervisor_nixos();
        let result = combined_audio_applied(
            HostEnforcementResult::Unsupported,
            GuestEnforcementResult::Unavailable,
            &cap,
        );
        assert_ne!(result, AudioSetApplied::HostOnly);
        assert_ne!(result, AudioSetApplied::GuestOnly);
        assert_ne!(result, AudioSetApplied::HostAndGuest);
        assert_eq!(result, AudioSetApplied::Unsupported);
    }

    // ── legacy host-only integration tests (FakeHostController) ─────────────

    #[test]
    fn fake_controller_success_guest_unavailable_maps_to_host_only() {
        use crate::audio_host_controller::FakeHostController;
        let cap = AudioProviderCapability::cloud_hypervisor_nixos();
        let ctrl = FakeHostController::success();
        let host_result = ctrl.enforce_grant("corp-vm", AudioGrant::Off, AudioChannel::Speaker);
        assert_eq!(host_result, HostEnforcementResult::Applied);
        let applied =
            combined_audio_applied(host_result, GuestEnforcementResult::Unavailable, &cap);
        assert_eq!(
            applied,
            AudioSetApplied::HostOnly,
            "host applied, guestd unavailable → HostOnly"
        );
    }

    #[test]
    fn fake_controller_failure_on_off_maps_to_unsupported_not_success() {
        use crate::audio_host_controller::FakeHostController;
        // When enforcement fails, the host boundary is NOT sealed; we must
        // report Unsupported, never HostOnly.
        let cap = AudioProviderCapability::cloud_hypervisor_nixos();
        let ctrl = FakeHostController::failed();
        let host_result = ctrl.enforce_grant("corp-vm", AudioGrant::Off, AudioChannel::Speaker);
        assert_eq!(host_result, HostEnforcementResult::Failed);
        let applied =
            combined_audio_applied(host_result, GuestEnforcementResult::Unavailable, &cap);
        assert_eq!(
            applied,
            AudioSetApplied::Unsupported,
            "failed enforcement on Off must be Unsupported — host boundary NOT sealed"
        );
        assert_ne!(applied, AudioSetApplied::HostOnly);
    }

    #[test]
    fn fake_controller_failure_on_level_maps_to_unsupported() {
        use crate::audio_host_controller::FakeHostController;
        let cap = AudioProviderCapability::qemu_media();
        let ctrl = FakeHostController::failed();
        let level = LevelPercent::new(80).unwrap();
        let host_result = ctrl.enforce_level("corp-vm", level, AudioChannel::Microphone);
        assert_eq!(host_result, HostEnforcementResult::Failed);
        let applied =
            combined_audio_applied(host_result, GuestEnforcementResult::Unavailable, &cap);
        assert_eq!(applied, AudioSetApplied::Unsupported);
    }

    #[test]
    fn qemu_controller_applied_maps_to_host_only() {
        use crate::audio_host_controller::QemuAudioController;
        let cap = AudioProviderCapability::qemu_media();
        let ctrl = QemuAudioController;
        let host_result = ctrl.enforce_grant("qemu-vm", AudioGrant::Off, AudioChannel::Speaker);
        assert_eq!(host_result, HostEnforcementResult::Applied);
        let applied =
            combined_audio_applied(host_result, GuestEnforcementResult::Unavailable, &cap);
        assert_eq!(applied, AudioSetApplied::HostOnly);
    }

    #[test]
    fn qemu_controller_never_calls_guestd_capable_path() {
        use crate::audio_host_controller::QemuAudioController;
        // qemu-media VMs have guest_enforcement = Unsupported. Verify the
        // applied result with Unsupported guest kind, not GuestdCapable.
        let cap = AudioProviderCapability::qemu_media();
        let ctrl = QemuAudioController;
        let host_result = ctrl.enforce_level(
            "qemu-vm",
            LevelPercent::new(50).unwrap(),
            AudioChannel::Microphone,
        );
        assert_eq!(host_result, HostEnforcementResult::Applied);
        let applied =
            combined_audio_applied(host_result, GuestEnforcementResult::Unavailable, &cap);
        assert_eq!(
            applied,
            AudioSetApplied::HostOnly,
            "qemu-media: offline policy applied → HostOnly; no guest enforcement"
        );
    }

    #[test]
    fn level_increase_classifier_treats_missing_old_level_as_increase() {
        let current = AudioPolicyState::default_v2();
        let old = current.speaker_level;
        let next = LevelPercent::new(1).unwrap();
        assert!(old.map(|old| next.get() > old.get()).unwrap_or(true));
    }

    #[test]
    fn level_increase_classifier_distinguishes_decrease() {
        let current =
            AudioPolicyState::default_v2().with_speaker_level(LevelPercent::new(80).unwrap());
        let old = current.speaker_level;
        let next = LevelPercent::new(40).unwrap();
        assert!(!old.map(|old| next.get() > old.get()).unwrap_or(true));
    }
}
