//! Audio policy dispatch for `d2b audio status`, `set-volume`, and `mute`.
//!
//! Resolves the per-VM provider capability row before touching local state:
//!
//! * **Cloud Hypervisor NixOS** - OFD-locked local state I/O and host PipeWire
//!   enforcement via the broker-backed
//!   `audio_host_controller::PipeWireHostController`.
//! * **qemu-media** - OFD-locked local state I/O, offline state-file policy.
//!   Target-side enforcement is reported as unavailable.
//!
//! All provider-internal resource IDs and credentials are redacted from
//! public responses. Volume/gain values never appear in audit records,
//! metric labels, or log messages.

use std::sync::Arc;

use d2b_contracts_broker::broker_wire::BrokerCallerRole;
use d2b_contracts_control::public_wire::{
    AudioChannel, AudioChannelState, AudioEnforcementPosture, AudioErrorKind, AudioMuteArgs,
    AudioOp, AudioOpResponse, AudioProviderKind, AudioSetApplied, AudioSetResult,
    AudioSetVolumeArgs, AudioStatusArgs, AudioStatusResult, AudioVmError, AudioVmState,
};
use d2b_core::manifest_v04::{ManifestV04, VmEntry as ManifestVmEntry};
use d2b_core::processes::ProcessesJson;
use d2b_core::provider_capabilities::{
    AudioGuestEnforcementKind, AudioHostEnforcementKind, AudioProviderCapability,
};
use d2b_core::runtime::{RuntimeKind, RuntimeProviderDriver};
use d2b_provider_audio_pipewire::{
    AudioChannel as ProviderAudioChannel, AudioGrant as ProviderAudioGrant, AudioMediator,
    AudioMediatorError, AudioReadiness, GuestAudioReadiness, HostAudioReadiness,
    LevelPercent as ProviderLevelPercent,
};
use d2b_provider_audio_pipewire::{
    AudioGrant, AudioPolicyState, LevelPercent, acquire_audio_state_lock, audio_lock_path,
    audio_state_path, read_audio_state_locked, read_audio_state_unlocked,
    write_audio_state_unlocked,
};
use serde_json::Value;

use crate::ServerState;
use crate::TypedError;
use crate::audio_host_controller::{
    HostAudioController, PipeWireHostController, QemuAudioController,
};
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
                d2b_provider_guest_cloud_hypervisor::audio_capability()
            }
            RuntimeProviderDriver::Qemu => d2b_provider_guest_qemu_media::audio_capability(),
        },
        RuntimeKind::QemuMedia => d2b_provider_guest_qemu_media::audio_capability(),
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
    match cap.host_enforcement {
        AudioHostEnforcementKind::None => AudioEnforcementPosture::Unsupported,
        AudioHostEnforcementKind::PipeWireVhostUserSound
        | AudioHostEnforcementKind::QemuAudioBackend => AudioEnforcementPosture::HostOnly,
    }
}

// ── Host enforcement ─────────────────────────────────────────────────────────

/// Result of a host-side audio enforcement call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostEnforcementResult {
    Applied,
    Unsupported,
    Failed,
}

/// Build the retained legacy host controller for a VM based on its audio
/// capability row. v3 Guest lifecycle does not use this process-DAG connector.
///
/// * For `PipeWireVhostUserSound` providers (Cloud Hypervisor NixOS), reads
///   the audio ProcessNode from `processes.json` to extract `WPCTL_PATH` and
///   `PIPEWIRE_RUNTIME_DIR` and returns a [`PipeWireHostController`]. Falls
///   back to returning `Unsupported` if the node or required env vars are
///   absent - this is a configuration error, not a runtime failure.
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
    caller_role: BrokerCallerRole,
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
            Some(Box::new(PipeWireHostController::from_audio_node(
                audio_node,
                vm_name,
                crate::broker_socket_path(state),
                caller_role,
            )) as Box<dyn HostAudioController>)
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
    caller_role: BrokerCallerRole,
    grant: AudioGrant,
    channel: AudioChannel,
) -> HostEnforcementResult {
    match build_host_controller(state, vm_name, cap, caller_role) {
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
    caller_role: BrokerCallerRole,
    level: LevelPercent,
    channel: AudioChannel,
) -> HostEnforcementResult {
    match build_host_controller(state, vm_name, cap, caller_role) {
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

/// Production AudioMediator backed by the daemon's broker and
/// target-local audio Provider.
///
/// The mediator owns no host handles or paths. It translates the Provider's
/// channel-neutral effect port into the existing capability-resolved daemon
/// controllers, which keep PipeWire mutations broker-owned. Target-side
/// effects are represented by the signed Process child resources.
pub(crate) struct DaemonAudioMediator {
    state: Arc<ServerState>,
    vm_name: String,
    capability: AudioProviderCapability,
    caller_role: BrokerCallerRole,
}

impl std::fmt::Debug for DaemonAudioMediator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DaemonAudioMediator")
            .field("vm_name", &"<opaque>")
            .field("capability", &self.capability)
            .finish()
    }
}

impl DaemonAudioMediator {
    pub(crate) fn new(
        state: &ServerState,
        vm_name: impl Into<String>,
        capability: AudioProviderCapability,
        caller_role: BrokerCallerRole,
    ) -> Self {
        Self {
            state: Arc::new(state.clone()),
            vm_name: vm_name.into(),
            capability,
            caller_role,
        }
    }

    fn wire_channel(channel: ProviderAudioChannel) -> AudioChannel {
        match channel {
            ProviderAudioChannel::Microphone => AudioChannel::Microphone,
            ProviderAudioChannel::Speaker => AudioChannel::Speaker,
        }
    }

    fn host_error(&self, result: HostEnforcementResult) -> Result<(), AudioMediatorError> {
        match (self.capability.host_enforcement, result) {
            (AudioHostEnforcementKind::None, _) | (_, HostEnforcementResult::Applied) => Ok(()),
            (_, HostEnforcementResult::Unsupported | HostEnforcementResult::Failed) => {
                Err(AudioMediatorError::ProviderSessionUnavailable)
            }
        }
    }
}

impl AudioMediator for DaemonAudioMediator {
    fn set_grant(&mut self, grant: ProviderAudioGrant) -> Result<(), AudioMediatorError> {
        self.set_channel_grant(ProviderAudioChannel::Speaker, grant)
    }

    fn set_channel_grant(
        &mut self,
        channel: ProviderAudioChannel,
        grant: ProviderAudioGrant,
    ) -> Result<(), AudioMediatorError> {
        let wire_channel = Self::wire_channel(channel);
        let host = enforce_host_grant(
            &self.state,
            &self.vm_name,
            &self.capability,
            self.caller_role.clone(),
            core_audio_grant(grant),
            wire_channel,
        );
        self.host_error(host)
    }

    fn set_level(&mut self, level: ProviderLevelPercent) -> Result<(), AudioMediatorError> {
        self.set_channel_level(ProviderAudioChannel::Speaker, level)
    }

    fn set_channel_level(
        &mut self,
        channel: ProviderAudioChannel,
        level: ProviderLevelPercent,
    ) -> Result<(), AudioMediatorError> {
        let wire_channel = Self::wire_channel(channel);
        let level = core_audio_level(level)?;
        let host = enforce_host_level(
            &self.state,
            &self.vm_name,
            &self.capability,
            self.caller_role.clone(),
            level,
            wire_channel,
        );
        self.host_error(host)
    }

    fn readiness(&self) -> AudioReadiness {
        match (self.host_readiness(), self.guest_readiness()) {
            (HostAudioReadiness::Ready, GuestAudioReadiness::Ready) => AudioReadiness::Ready,
            _ => AudioReadiness::Unavailable,
        }
    }

    fn host_readiness(&self) -> HostAudioReadiness {
        if self.capability.host_enforcement == AudioHostEnforcementKind::None
            || build_host_controller(
                &self.state,
                &self.vm_name,
                &self.capability,
                self.caller_role.clone(),
            )
            .is_some()
        {
            HostAudioReadiness::Ready
        } else {
            HostAudioReadiness::Unavailable
        }
    }

    fn guest_readiness(&self) -> GuestAudioReadiness {
        if self.capability.guest_enforcement == AudioGuestEnforcementKind::Unsupported {
            return GuestAudioReadiness::Ready;
        }
        GuestAudioReadiness::Unavailable
    }
}

fn core_audio_grant(grant: ProviderAudioGrant) -> AudioGrant {
    match grant {
        ProviderAudioGrant::On => AudioGrant::On,
        ProviderAudioGrant::Off => AudioGrant::Off,
    }
}

fn core_audio_level(level: ProviderLevelPercent) -> Result<LevelPercent, AudioMediatorError> {
    LevelPercent::new(level.get()).map_err(|_| AudioMediatorError::LevelOutOfRange)
}

// ── Enforcement result → AudioSetApplied mapping ──────────────────────────────

/// Combine host enforcement results into the public
/// [`AudioSetApplied`] outcome.
///
/// This function is `pub(crate)` so the test suite can lock the mapping
/// without needing a full [`crate::ServerState`].
pub(crate) fn combined_audio_applied(
    host_result: HostEnforcementResult,
    cap: &AudioProviderCapability,
) -> AudioSetApplied {
    if cap.host_enforcement == AudioHostEnforcementKind::None {
        AudioSetApplied::Unsupported
    } else {
        match host_result {
            HostEnforcementResult::Applied => AudioSetApplied::HostOnly,
            HostEnforcementResult::Unsupported | HostEnforcementResult::Failed => {
                AudioSetApplied::Unsupported
            }
        }
    }
}

// ── dispatch_audio ────────────────────────────────────────────────────────────

pub fn dispatch_audio(
    state: &ServerState,
    caller_role: BrokerCallerRole,
    op: AudioOp,
) -> Result<Value, TypedError> {
    match op {
        AudioOp::Status(args) => dispatch_audio_status(state, caller_role, args),
        AudioOp::SetVolume(args) => dispatch_audio_set_volume(state, caller_role, args),
        AudioOp::Mute(args) => dispatch_audio_mute(state, caller_role, args),
    }
}

// ── Status ─────────────────────────────────────────────────────────────────

fn dispatch_audio_status(
    state: &ServerState,
    caller_role: BrokerCallerRole,
    args: AudioStatusArgs,
) -> Result<Value, TypedError> {
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
        match resolve_vm_audio_status(state, vm_name, &manifest, caller_role.clone()) {
            Ok(vm_state) => entries.push(vm_state),
            Err(vm_error) => errors.push(vm_error),
        }
    }

    let result = AudioStatusResult { entries, errors };
    Ok(d2bd_runtime::wire::audio_response(
        &AudioOpResponse::Status(result),
    ))
}

fn resolve_vm_audio_status(
    state: &ServerState,
    vm_name: &str,
    manifest: &ManifestV04,
    caller_role: BrokerCallerRole,
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
    let mediator = DaemonAudioMediator::new(state, vm_name, cap.clone(), caller_role.clone());
    vm_state.enforcement = match (mediator.host_readiness(), mediator.guest_readiness()) {
        (HostAudioReadiness::Ready, GuestAudioReadiness::Ready) => public_enforcement_posture(&cap),
        (HostAudioReadiness::Ready, GuestAudioReadiness::Unavailable) => {
            AudioEnforcementPosture::HostOnly
        }
        (HostAudioReadiness::Unavailable, GuestAudioReadiness::Ready) => {
            AudioEnforcementPosture::GuestOnly
        }
        (HostAudioReadiness::Unavailable, GuestAudioReadiness::Unavailable) => {
            AudioEnforcementPosture::Unsupported
        }
    };
    Ok(vm_state)
}

// ── SetVolume ─────────────────────────────────────────────────────────────────

fn dispatch_audio_set_volume(
    state: &ServerState,
    caller_role: BrokerCallerRole,
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
        let result = enforce_host_level(state, vm_name, &cap, caller_role.clone(), level, channel);
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
        enforce_host_level(state, vm_name, &cap, caller_role.clone(), level, channel)
    } else {
        host_result
    };

    let applied = combined_audio_applied(host_result, &cap);

    let channel_state = match channel {
        AudioChannel::Speaker => state_to_channel(new_state.speaker, new_state.speaker_level),
        AudioChannel::Microphone => state_to_channel(new_state.mic, new_state.mic_gain),
    };

    Ok(d2bd_runtime::wire::audio_response(
        &AudioOpResponse::SetVolume(AudioSetResult {
            vm: vm_name.clone(),
            channel,
            applied,
            state: channel_state,
        }),
    ))
}

// ── Mute ──────────────────────────────────────────────────────────────────────

fn dispatch_audio_mute(
    state: &ServerState,
    caller_role: BrokerCallerRole,
    args: AudioMuteArgs,
) -> Result<Value, TypedError> {
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
        let result = enforce_host_grant(state, vm_name, &cap, caller_role.clone(), grant, channel);
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
        enforce_host_grant(state, vm_name, &cap, caller_role.clone(), grant, channel)
    } else {
        host_result
    };

    let applied = combined_audio_applied(host_result, &cap);

    let channel_state = match channel {
        AudioChannel::Speaker => state_to_channel(new_state.speaker, new_state.speaker_level),
        AudioChannel::Microphone => state_to_channel(new_state.mic, new_state.mic_gain),
    };

    Ok(d2bd_runtime::wire::audio_response(&AudioOpResponse::Mute(
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
    use d2b_provider_audio_pipewire::{AudioGrant, AudioPolicyState};

    // ── Provider capability tests ───────────────────────────────────────────

    #[test]
    fn ch_nixos_cap_is_pipewire_target_process() {
        let cap = d2b_provider_guest_cloud_hypervisor::audio_capability();
        assert_eq!(
            cap.host_enforcement,
            AudioHostEnforcementKind::PipeWireVhostUserSound
        );
        assert_eq!(
            cap.guest_enforcement,
            AudioGuestEnforcementKind::ProcessCapable
        );
        assert!(cap.needs_local_state_file);
    }

    #[test]
    fn qemu_media_cap_is_host_only() {
        let cap = d2b_provider_guest_qemu_media::audio_capability();
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
    fn aca_cap_uses_target_process_without_local_state() {
        let cap = AudioProviderCapability::aca_sandbox();
        assert_eq!(cap.host_enforcement, AudioHostEnforcementKind::None);
        assert_eq!(
            cap.guest_enforcement,
            AudioGuestEnforcementKind::ProcessCapable
        );
        assert!(!cap.needs_local_state_file);
    }

    #[test]
    fn enforcement_posture_mapping() {
        let ch_cap = d2b_provider_guest_cloud_hypervisor::audio_capability();
        assert_eq!(
            public_enforcement_posture(&ch_cap),
            AudioEnforcementPosture::HostOnly
        );

        let qemu_cap = d2b_provider_guest_qemu_media::audio_capability();
        assert_eq!(
            public_enforcement_posture(&qemu_cap),
            AudioEnforcementPosture::HostOnly
        );

        let aca_cap = AudioProviderCapability::aca_sandbox();
        assert_eq!(
            public_enforcement_posture(&aca_cap),
            AudioEnforcementPosture::Unsupported
        );
    }

    // ── legacy host-only integration tests (FakeHostController) ─────────────

    #[test]
    fn fake_controller_success_guest_unavailable_maps_to_host_only() {
        use crate::audio_host_controller::FakeHostController;
        let cap = d2b_provider_guest_cloud_hypervisor::audio_capability();
        let ctrl = FakeHostController::success();
        let host_result = ctrl.enforce_grant("corp-vm", AudioGrant::Off, AudioChannel::Speaker);
        assert_eq!(host_result, HostEnforcementResult::Applied);
        let applied = combined_audio_applied(host_result, &cap);
        assert_eq!(
            applied,
            AudioSetApplied::HostOnly,
            "host applied, target process unavailable -> HostOnly"
        );
    }

    #[test]
    fn fake_controller_failure_on_off_maps_to_unsupported_not_success() {
        use crate::audio_host_controller::FakeHostController;
        // When enforcement fails, the host boundary is NOT sealed; we must
        // report Unsupported, never HostOnly.
        let cap = d2b_provider_guest_cloud_hypervisor::audio_capability();
        let ctrl = FakeHostController::failed();
        let host_result = ctrl.enforce_grant("corp-vm", AudioGrant::Off, AudioChannel::Speaker);
        assert_eq!(host_result, HostEnforcementResult::Failed);
        let applied = combined_audio_applied(host_result, &cap);
        assert_eq!(
            applied,
            AudioSetApplied::Unsupported,
            "failed enforcement on Off must be Unsupported - host boundary NOT sealed"
        );
        assert_ne!(applied, AudioSetApplied::HostOnly);
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

    // ── dispatch error paths ────────────────────────────────────────────────

    fn audio_manifest(vms: serde_json::Value) -> serde_json::Value {
        // ManifestV04 flattens the per-VM entries to the top level.
        let mut manifest = serde_json::json!({
            "_manifest": { "manifestVersion": 6 },
            "_observability": {
                "enabled": false,
                "signozUrl": "http://127.0.0.1:8080",
                "signozOtlpGrpcPort": 4317,
                "signozOtlpHttpPort": 4318,
                "obsVsockCid": 1000,
                "obsVsockHostSocket": "/run/d2b/obs.sock",
                "vmName": "sys-obs"
            }
        });
        let object = manifest.as_object_mut().expect("manifest object");
        for (name, entry) in vms.as_object().expect("vms object") {
            object.insert(name.clone(), entry.clone());
        }
        manifest
    }

    fn audio_vm_json(name: &str, state_dir: &str, audio: bool) -> serde_json::Value {
        serde_json::json!({
            "name": name,
            "apiSocket": null,
            "audio": audio,
            "audioService": null,
            "audioStateFile": null,
            "bridge": null,
            "env": null,
            "gpuSocket": null,
            "staticIp": null,
            "sshUser": null,
            "isNetVm": false,
            "netVm": null,
            "stateDir": state_dir,
            "graphics": false,
            "tpm": false,
            "tpmSocket": null,
            "usbipYubikey": false,
            "usbipdHostIp": null,
            "tap": "work-l2",
            "observability": {
                "agentSocket": "/run/d2b/otlp.sock",
                "enabled": false,
                "vsockCid": 110,
                "vsockHostSocket": "/run/d2b/vm-a-vsock.sock"
            },
            "runtime": {
                "kind": "nixos",
                "provider": {
                    "driver": "cloud-hypervisor",
                    "id": "local-cloud-hypervisor",
                    "type": "local"
                },
                "capabilities": {
                    "lifecycle": true,
                    "display": false,
                    "usbHotplug": false,
                    "exec": true,
                    "configSync": true,
                    "ssh": true,
                    "storeSync": true,
                    "keys": true,
                    "inGuestObservability": true
                }
            }
        })
    }

    fn audio_processes_json() -> serde_json::Value {
        serde_json::json!({
            "schemaVersion": "v2",
            "vms": [
                {
                    "vm": "vm-a",
                    "nodes": [
                        {
                            "id": "audio",
                            "role": "audio",
                            "binaryPath": "/run/d2b/vms/vm-a/d2b-vm-a",
                            "readiness": [],
                            "profile": {
                                "profileId": "w1-audio",
                                "uid": 60100,
                                "gid": 60100,
                                "caps": [],
                                "namespaces": {
                                    "mount": false,
                                    "pid": false,
                                    "net": false,
                                    "ipc": false,
                                    "uts": false,
                                    "user": false
                                },
                                "mountPolicy": {
                                    "readOnlyPaths": [],
                                    "writablePaths": [],
                                    "nixStoreReadOnly": false,
                                    "hideDeviceNodesByDefault": false
                                },
                                "cgroupPlacement": {
                                    "subtree": "d2b.slice/vm-a/audio",
                                    "controllers": [],
                                    "delegated": false
                                }
                            }
                        }
                    ],
                    "edges": [],
                    "invariants": {
                        "swtpmPreStartFlush": false,
                        "perVmAuditPipeline": false,
                        "usbipGating": false,
                        "tpmOwnershipMigrationWithoutRunningVmMutation": false
                    }
                }
            ]
        })
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn test_state(
        manifest: &serde_json::Value,
        processes: &serde_json::Value,
    ) -> (ServerState, tempfile::TempDir) {
        use d2bd_runtime::daemon_config::{ArtifactPaths, DaemonConfig};

        let dir = tempfile::tempdir().expect("audio dispatch test state");
        let root = dir.path();
        let locks_dir = root.join("locks");
        std::fs::create_dir_all(&locks_dir).expect("create locks dir");
        let public_manifest_path = root.join("vms.json");
        let processes_path = root.join("processes.json");
        std::fs::write(
            &public_manifest_path,
            serde_json::to_vec(manifest).expect("manifest json"),
        )
        .expect("write manifest");
        std::fs::write(
            &processes_path,
            serde_json::to_vec(processes).expect("processes json"),
        )
        .expect("write processes");
        let broker_reap_log = d2bd_runtime::supervisor::pidfd_table::BrokerReapLog::new();
        let state = ServerState {
            config: DaemonConfig {
                broker_socket_path: root.join("broker.sock"),
                locks_dir,
                artifacts: ArtifactPaths {
                    public_manifest_path,
                    processes_path,
                    ..ArtifactPaths::default()
                },
                ..DaemonConfig::default()
            },
            daemon_uid: 0,
            daemon_audit: Arc::new(d2bd_runtime::daemon_audit::DaemonAuditLog::no_op()),
            daemon_state_dir: root.join("daemon-state"),
            pidfd_table: Arc::new(
                d2bd_runtime::supervisor::pidfd_table::PidfdTable::new(
                    root.join("daemon-state").join("pidfd-table.json"),
                )
                .with_broker_reap_log(Arc::clone(&broker_reap_log)),
            ),
            broker_reap_log,
            metrics_registry: Arc::new(d2bd_runtime::metrics::Registry::new()),
            exec_sessions: Arc::new(crate::exec_session::SessionTable::new(
                crate::exec_session::ExecSessionCaps::default(),
            )),
            console_sessions: Arc::new(tokio::sync::Mutex::new(
                crate::console_session::ConsoleSessionTable::default(),
            )),
            conn_semaphore: d2bd_runtime::concurrency::ConnSemaphore::new(8),
            op_locks: d2bd_runtime::concurrency::OpLockManager::new(),
            public_status_read_model: Arc::new(
                d2bd_runtime::public_read_model::PublicStatusReadModel::new(),
            ),
            provider_runtime: Arc::new(crate::provider_registry::ProviderRuntime::new()),
            resource_plane: Arc::new(tokio::sync::Mutex::new(None)),
            interaction_runtime: Arc::new(tokio::sync::Mutex::new(None)),
            interaction_listeners: Arc::new(tokio::sync::Mutex::new(None)),
            typed_shell_session_targets: d2bd_runtime::typed_shell_targets::new_cache(),
            zone_coordinator: d2bd_runtime::zone_authority::new_coordinator(),
            config_staging: Arc::new(tokio::sync::Mutex::new(Default::default())),
            guest_component_sessions: Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            guest_component_session_locks: Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            security_key_sessions: Arc::new(tokio::sync::Mutex::new(
                d2b_provider_device_security_key::SkSessionTable::default(),
            )),
            unsafe_local_helpers: Arc::new(d2bd_runtime::unsafe_local_helper::HelperRegistry::new(
                0,
                [],
            )),
            v3_planes: std::sync::Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            runtime_handle: tokio::runtime::Handle::try_current().unwrap_or_else(|_| {
                static TEST_FALLBACK_RUNTIME: std::sync::LazyLock<tokio::runtime::Runtime> =
                    std::sync::LazyLock::new(|| {
                        tokio::runtime::Builder::new_multi_thread()
                            .enable_all()
                            .thread_name("d2b-test-fallback")
                            .build()
                            .expect("build the test fallback tokio runtime")
                    });
                TEST_FALLBACK_RUNTIME.handle().clone()
            }),
        };
        (state, dir)
    }

    fn load_manifest(state: &ServerState) -> ManifestV04 {
        crate::load_json(&state.config.artifacts.public_manifest_path).expect("load manifest")
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn resolve_vm_audio_status_reports_every_closed_error_kind() {
        let state_dir = std::env::temp_dir().join(format!(
            "d2b-audio-status-{}",
            std::process::id()
        ));
        let (state, _dir) = test_state(
            &audio_manifest(serde_json::json!({
                "vm-a": audio_vm_json("vm-a", state_dir.to_str().expect("utf8 state dir"), true),
                "vm-b": audio_vm_json("vm-b", state_dir.to_str().expect("utf8 state dir"), false),
            })),
            &serde_json::json!({ "schemaVersion": "v2", "vms": [] }),
        );
        let manifest = load_manifest(&state);
        let caller = BrokerCallerRole::NotAuthorized;

        let missing = resolve_vm_audio_status(&state, "no-such-vm", &manifest, caller.clone())
            .expect_err("unknown vm");
        assert_eq!(missing.kind, AudioErrorKind::VmNotFound);
        assert_eq!(missing.vm, "no-such-vm");

        let not_enabled = resolve_vm_audio_status(&state, "vm-b", &manifest, caller.clone())
            .expect_err("audio disabled vm");
        assert_eq!(not_enabled.kind, AudioErrorKind::AudioNotEnabled);
        assert!(
            not_enabled.remediation.is_some(),
            "AudioNotEnabled must carry a remediation hint"
        );

        // A malformed state file is an internal error, not a silent default.
        let state_file = std::path::PathBuf::from(&state_dir).join("state/audio-state.json");
        tokio::fs::create_dir_all(state_file.parent().expect("state dir"))
            .await
            .expect("create state dir");
        tokio::fs::write(&state_file, b"not json")
            .await
            .expect("write malformed state");
        let internal = resolve_vm_audio_status(&state, "vm-a", &manifest, caller)
            .expect_err("malformed state file");
        assert_eq!(internal.kind, AudioErrorKind::InternalError);
    }

    #[test]
    fn dispatch_audio_status_aggregates_per_vm_errors_not_failures() {
        let state_dir = std::env::temp_dir().join(format!(
            "d2b-audio-status-agg-{}",
            std::process::id()
        ));
        let (state, _dir) = test_state(
            &audio_manifest(serde_json::json!({
                "vm-a": audio_vm_json("vm-a", state_dir.to_str().expect("utf8 state dir"), true),
                "vm-b": audio_vm_json("vm-b", state_dir.to_str().expect("utf8 state dir"), false),
            })),
            &serde_json::json!({ "schemaVersion": "v2", "vms": [] }),
        );
        let response = dispatch_audio_status(
            &state,
            BrokerCallerRole::NotAuthorized,
            AudioStatusArgs {
                vms: vec!["vm-b".to_owned(), "no-such-vm".to_owned()],
            },
        )
        .expect("status response");
        let result = match serde_json::from_value::<AudioOpResponse>(response)
            .expect("decode status response")
        {
            AudioOpResponse::Status(result) => result,
            other => panic!("expected Status response, got {other:?}"),
        };
        assert!(result.entries.is_empty());
        assert_eq!(result.errors.len(), 2);
        assert!(result
            .errors
            .iter()
            .any(|error| error.kind == AudioErrorKind::AudioNotEnabled && error.vm == "vm-b"));
        assert!(result
            .errors
            .iter()
            .any(|error| error.kind == AudioErrorKind::VmNotFound && error.vm == "no-such-vm"));
    }

    #[test]
    fn set_volume_fails_closed_when_host_enforcement_fails_on_an_increase() {
        let state_dir = std::env::temp_dir().join(format!(
            "d2b-audio-volume-{}",
            std::process::id()
        ));
        let (state, _dir) = test_state(
            &audio_manifest(serde_json::json!({
                "vm-a": audio_vm_json("vm-a", state_dir.to_str().expect("utf8 state dir"), true),
            })),
            &audio_processes_json(),
        );
        // The broker socket does not exist, so the PipeWire controller's
        // effect dispatch fails: a level increase must refuse as InternalIo
        // rather than persisting an unenforced level.
        let error = dispatch_audio_set_volume(
            &state,
            BrokerCallerRole::NotAuthorized,
            AudioSetVolumeArgs {
                vm: "vm-a".to_owned(),
                channel: AudioChannel::Speaker,
                level: LevelPercent::new(50).expect("valid level"),
            },
        )
        .expect_err("failed host enforcement must refuse the increase");
        match error {
            TypedError::InternalIo { context, detail } => {
                assert_eq!(context, "audio host enforcement");
                assert!(
                    detail.contains("host level enforcement failed"),
                    "unexpected detail: {detail}"
                );
            }
            other => panic!("expected InternalIo, got {other:?}"),
        }
    }

    #[test]
    fn mute_fails_closed_when_host_enforcement_fails_on_an_unmute() {
        let state_dir = std::env::temp_dir().join(format!(
            "d2b-audio-mute-{}",
            std::process::id()
        ));
        let (state, _dir) = test_state(
            &audio_manifest(serde_json::json!({
                "vm-a": audio_vm_json("vm-a", state_dir.to_str().expect("utf8 state dir"), true),
            })),
            &audio_processes_json(),
        );
        // Grant On (unmute) proves live host enforcement before persisting:
        // a failed enforcement must refuse as InternalIo.
        let error = dispatch_audio_mute(
            &state,
            BrokerCallerRole::NotAuthorized,
            AudioMuteArgs {
                vm: "vm-a".to_owned(),
                channel: AudioChannel::Speaker,
                mute: false,
            },
        )
        .expect_err("failed host enforcement must refuse the unmute");
        match error {
            TypedError::InternalIo { context, detail } => {
                assert_eq!(context, "audio host enforcement");
                assert!(
                    detail.contains("host grant enforcement failed"),
                    "unexpected detail: {detail}"
                );
            }
            other => panic!("expected InternalIo, got {other:?}"),
        }
    }
}
