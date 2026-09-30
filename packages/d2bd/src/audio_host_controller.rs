//! Host-side audio enforcement over admitted audio sessions (ADR 0041).
//!
//! Defines [`HostAudioController`], a typed trait for host-side audio
//! enforcement, plus concrete implementations:
//!
//! * [`AdmittedPipeWireHostController`] - the PipeWire host enforcement for
//!   Cloud Hypervisor NixOS targets. It reaches the PipeWire session only
//!   through the exact endpoint relationships the Zone graph admitted for
//!   each stream direction, and applies each channel's declared method
//!   (`set-speaker-grant`, `set-speaker-level`, `set-microphone-grant`,
//!   `set-microphone-gain`) on that one relationship.
//! * [`QemuAudioController`] - offline-only enforcement for qemu-media VMs.
//!   Writing the state file IS the policy for qemu-media; no live runtime
//!   enforcement exists and no target-local Process call is made.
//! * `FakeHostController` - test-only injectable with configurable results.
//!   Gated behind `#[cfg(test)]` so it never compiles into production builds.
//!
//! ## Admitted host sessions
//!
//! A channel effect is carried by one admitted `EndpointBinding`: the exact
//! endpoint the `AudioService` committed, the exact consumer, the channel's
//! stable slot, and the channel's declared purpose. The binding's fence is
//! re-checked immediately before the effect runs, so a relationship that was
//! revoked or re-committed stops serving effects even if the call was already
//! scheduled. The socket, the runtime directory, and the PipeWire tools come
//! from [`AudioSessionPinning`], which the endpoint owner resolved privately;
//! no constructor here reads them out of a workload's environment, so no
//! runtime value can redirect a channel to a neighbouring session (AE7).
//!
//! The broker's audio-specific `PipeWireAudio` operation and the wire the
//! daemon used to send it are gone from this file: a channel effect is a
//! declared method on an admitted relationship, not a broker action enum.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use d2b_contracts::types::{BundleOpId, RoleId, VmId};
use d2b_contracts_broker::broker_wire::{
    BrokerCallerRole, BrokerRequest, BrokerResponse, PipeWireAudioAction, PipeWireAudioChannel,
    PipeWireAudioRequest,
};
use d2b_contracts_control::public_wire::AudioChannel;
use d2b_core::bundle_resolver::intent_id_legacy_runner;
use d2b_core::processes::{ProcessNode, ProcessesJson, VmProcessDag};
use d2b_provider_audio_pipewire::{
    AUDIO_DECLARED_METHODS, AudioBindingFence, AudioEffectKind, AudioGrant, AudioMediatorError,
    AudioSessionOrigin, AudioSessionPlan, LevelPercent, PIPEWIRE_RUNTIME_SOCKET,
    audio_declared_method,
};
use serde_json::Value;

pub use crate::audio_dispatch::HostEnforcementResult;

/// How long one PipeWire probe or mutation may take.
///
/// A stalled `pw-dump` reports host-not-ready instead of pinning a daemon
/// worker, which is the same bound the retired broker arm applied.
const PIPEWIRE_EFFECT_TIMEOUT: Duration = Duration::from_secs(5);

// ── trait ────────────────────────────────────────────────────────────────────

/// Strategy for host-side audio enforcement.
///
/// The trait is `dyn`-safe so dispatch functions can accept `&dyn
/// HostAudioController` and tests can inject a fake.
pub(crate) trait HostAudioController {
    /// Enforce a mute/unmute grant on a running VM's audio node.
    ///
    /// Returns [`HostEnforcementResult::Applied`] only when enforcement was
    /// confirmed. Returns `Failed` when the channel's relationship is not
    /// admitted, when its fence is no longer current, or when the effect
    /// itself failed, so callers know `off` did **not** seal the host
    /// boundary. Returns `Unsupported` only for offline-only providers where
    /// no live enforcement path exists.
    fn enforce_grant(&self, grant: AudioGrant, channel: AudioChannel) -> HostEnforcementResult;

    /// Enforce a volume/gain level change on a running VM's audio node.
    ///
    /// Same success/failure contract as [`Self::enforce_grant`].
    fn enforce_level(&self, level: LevelPercent, channel: AudioChannel) -> HostEnforcementResult;
}

// ── AdmittedPipeWireHostController ───────────────────────────────────────────

/// The exact session one admitted audio endpoint relationship delivers to.
///
/// The endpoint owner resolved these privately from the committed `Endpoint`
/// row: a verified descriptor to that one socket, or the private presentation
/// of that same inode. Every path must be absolute and the socket must be
/// present, so a relative or absent value refuses rather than falling back to
/// whatever happens to sit in an ambient runtime directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AudioSessionPinning {
    runtime_dir: PathBuf,
    socket: PathBuf,
    wpctl: PathBuf,
    pw_dump: PathBuf,
}

impl AudioSessionPinning {
    /// Bind one admitted relationship to the session the endpoint owner
    /// resolved.
    ///
    /// # Errors
    ///
    /// Returns `EndpointBindingNotAdmitted` when the channel has no admitted
    /// relationship, `ImportedProjectionCannotGrant` when the relationship
    /// came from a ResourceImport projection, `EndpointBindingNotCurrent`
    /// when the fence has moved, and `ProviderSessionUnavailable` when any
    /// pinned path is relative or the pinned socket is not present.
    pub(crate) fn admit(
        session: &d2b_provider_audio_pipewire::AdmittedAudioSession,
        observed: &AudioBindingFence,
        runtime_dir: PathBuf,
        wpctl: PathBuf,
        pw_dump: PathBuf,
    ) -> Result<Self, AudioMediatorError> {
        if session.origin() == AudioSessionOrigin::ImportedProjection {
            return Err(AudioMediatorError::ImportedProjectionCannotGrant);
        }
        if !session.is_current(observed) {
            return Err(AudioMediatorError::EndpointBindingNotCurrent);
        }
        let socket = runtime_dir.join(PIPEWIRE_RUNTIME_SOCKET);
        if !runtime_dir.is_absolute()
            || !socket.is_absolute()
            || !wpctl.is_absolute()
            || !pw_dump.is_absolute()
            || !socket.exists()
        {
            return Err(AudioMediatorError::ProviderSessionUnavailable);
        }
        Ok(Self {
            runtime_dir,
            socket,
            wpctl,
            pw_dump,
        })
    }

    /// Run one pinned tool with a cleared environment and a bounded wait.
    ///
    /// The PipeWire runtime directory comes from the pinned delivery alone,
    /// so an inherited `PIPEWIRE_RUNTIME_DIR` or `XDG_RUNTIME_DIR` has no
    /// effect on which session the tool reaches. A tool that outlives the
    /// bound is killed and reported as a refusal, so a stalled `pw-dump`
    /// cannot pin a daemon worker.
    fn run(&self, tool: &std::path::Path, args: &[&str]) -> Result<std::process::Output, HostAudioFailure> {
        let mut command = std::process::Command::new(tool);
        command
            .env_clear()
            .env("PIPEWIRE_RUNTIME_DIR", &self.runtime_dir)
            .env("XDG_RUNTIME_DIR", &self.runtime_dir)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().map_err(|_| HostAudioFailure::Spawn)?;
        let stdout = child.stdout.take();
        let reader = std::thread::spawn(move || {
            let mut buffer = Vec::new();
            if let Some(mut stdout) = stdout {
                use std::io::Read as _;
                let _ = stdout.read_to_end(&mut buffer);
            }
            buffer
        });
        let deadline = std::time::Instant::now() + PIPEWIRE_EFFECT_TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = reader.join();
                        return Err(HostAudioFailure::Timeout);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => {
                    let _ = reader.join();
                    return Err(HostAudioFailure::Spawn);
                }
            }
        };
        let stdout = reader.join().unwrap_or_default();
        Ok(std::process::Output {
            status,
            stdout,
            stderr: Vec::new(),
        })
    }

    /// Run `pw-dump` and select the one node this channel's stream owns.
    ///
    /// A dump with no matching node and a dump with two matching nodes are
    /// both refusals: neither is a reason to act on a node the relationship
    /// does not own.
    fn select_node(&self, vm_name: &str, channel: AudioChannel) -> Result<String, HostAudioFailure> {
        let output = self.run(&self.pw_dump, &[])?;
        if !output.status.success() {
            return Err(HostAudioFailure::ProbeFailed);
        }
        target_node_from_pw_dump(&output.stdout, vm_name, channel)
            .ok_or(HostAudioFailure::NoSingleStream)
    }

    /// Apply one declared channel method to the node this relationship owns.
    ///
    /// The grant method maps to `wpctl set-mute` and the level method to
    /// `wpctl set-volume`; both run through the pinned session with a cleared
    /// environment and report success only on a zero exit.
    fn apply(
        &self,
        method: d2b_provider_audio_pipewire::AudioDeclaredMethod,
        node: &str,
        argument: &str,
    ) -> Result<(), HostAudioFailure> {
        let (verb, value) = match method.effect() {
            AudioEffectKind::Grant => (
                "set-mute",
                if argument == AudioGrant::On.as_wire_str() {
                    "0"
                } else {
                    "1"
                },
            ),
            AudioEffectKind::Level => ("set-volume", argument),
        };
        let output = self.run(&self.wpctl, &[verb, node, value])?;
        if output.status.success() {
            Ok(())
        } else {
            Err(HostAudioFailure::MutationFailed)
        }
    }
}

/// What one admitted host effect could not do.
///
/// Every variant is field-free and names only the failing stage, so a log
/// line built from it carries no host path and no target identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostAudioFailure {
    /// The channel has no admitted endpoint relationship.
    NotAdmitted,
    /// The admitted relationship's fence is no longer current.
    NotCurrent,
    /// The relationship came from a ResourceImport projection, not a local
    /// owner Service.
    NotOwner,
    /// The pinned tool could not be started.
    Spawn,
    /// The pinned tool did not finish inside the bound.
    Timeout,
    /// The probe reported failure.
    ProbeFailed,
    /// The admitted session named no single stream for this channel.
    NoSingleStream,
    /// The mutation the admitted session reported as failed.
    MutationFailed,
}

// ── AdmittedPipeWireHostController ───────────────────────────────────────────

/// The verified session locations the endpoint owner pinned for a target.
///
/// These are the deployment's own absolute paths for the PipeWire session and
/// its two tools. They are the endpoint owner's delivery, not a value read out
/// of a workload's launch environment, so nothing a caller sets in the process
/// can move the session they name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AudioSessionPinningSources {
    /// The PipeWire runtime directory the owner resolved.
    pub(crate) runtime_dir: PathBuf,
    /// The `wpctl` executable the deployment pinned.
    pub(crate) wpctl: PathBuf,
    /// The `pw-dump` executable the deployment pinned.
    pub(crate) pw_dump: PathBuf,
}

/// The host-side PipeWire enforcement for a target whose audio channels the
/// graph admitted.
///
/// The controller owns no authority of its own: each channel's effect runs
/// only when that channel's exact `EndpointBinding` is admitted, current, and
/// owned by a local `AudioService`, and the PipeWire session it reaches is the
/// one the endpoint owner pinned for that relationship.
#[derive(Debug, Clone)]
pub(crate) struct AdmittedPipeWireHostController {
    vm_id: VmId,
    plan: AudioSessionPlan,
    observed: Vec<AudioBindingFence>,
    speaker: Option<AudioSessionPinning>,
    microphone: Option<AudioSessionPinning>,
}

impl AdmittedPipeWireHostController {
    /// Build the controller from the admitted relationships of one target.
    ///
    /// A channel whose relationship was refused contributes no pinning, so
    /// its effects report failure instead of reaching for another channel's
    /// session.
    pub(crate) fn new(
        vm_name: impl Into<String>,
        plan: AudioSessionPlan,
        observed: Vec<AudioBindingFence>,
        pinning: AudioSessionPinningSources,
    ) -> Self {
        let resolve = |channel: d2b_provider_audio_pipewire::AudioChannel| {
            let session = plan.session(channel)?;
            let fence = observed.iter().find(|fence| session.is_current(fence))?;
            AudioSessionPinning::admit(
                session,
                fence,
                pinning.runtime_dir.clone(),
                pinning.wpctl.clone(),
                pinning.pw_dump.clone(),
            )
            .ok()
        };
        Self {
            vm_id: VmId::new(vm_name.into()),
            speaker: resolve(d2b_provider_audio_pipewire::AudioChannel::Speaker),
            microphone: resolve(d2b_provider_audio_pipewire::AudioChannel::Microphone),
            plan,
            observed,
        }
    }

    /// Find the audio runner node for a VM in a loaded [`ProcessesJson`].
    ///
    /// Returns `None` when no audio node exists (the VM has no audio
    /// sidecar). This is a presence probe over the committed process
    /// inventory, not an access decision.
    pub(crate) fn find_audio_node<'a>(
        processes: &'a ProcessesJson,
        vm_name: &str,
    ) -> Option<&'a ProcessNode> {
        let vm_dag: &VmProcessDag = processes.vms.iter().find(|v| v.vm == vm_name)?;
        vm_dag
            .nodes
            .iter()
            .find(|n| matches!(n.role, d2b_core::processes::ProcessRole::Audio))
    }

    fn pinning(&self, channel: AudioChannel) -> Result<&AudioSessionPinning, HostAudioFailure> {
        let provider_channel = match channel {
            AudioChannel::Speaker => d2b_provider_audio_pipewire::AudioChannel::Speaker,
            AudioChannel::Microphone => d2b_provider_audio_pipewire::AudioChannel::Microphone,
        };
        let pinned = match provider_channel {
            d2b_provider_audio_pipewire::AudioChannel::Speaker => self.speaker.as_ref(),
            d2b_provider_audio_pipewire::AudioChannel::Microphone => self.microphone.as_ref(),
        };
        let pinned = pinned.ok_or(HostAudioFailure::NotAdmitted)?;
        let session = self
            .plan
            .session(provider_channel)
            .ok_or(HostAudioFailure::NotAdmitted)?;
        let observed = self
            .observed
            .iter()
            .find(|fence| session.is_current(fence))
            .ok_or(HostAudioFailure::NotCurrent)?;
        if session.origin() == AudioSessionOrigin::ImportedProjection {
            return Err(HostAudioFailure::NotOwner);
        }
        if !session.is_current(observed) {
            return Err(HostAudioFailure::NotCurrent);
        }
        Ok(pinned)
    }

    fn apply(
        &self,
        channel: AudioChannel,
        effect: AudioEffectKind,
        argument: &str,
    ) -> HostEnforcementResult {
        let provider_channel = match channel {
            AudioChannel::Speaker => d2b_provider_audio_pipewire::AudioChannel::Speaker,
            AudioChannel::Microphone => d2b_provider_audio_pipewire::AudioChannel::Microphone,
        };
        let Some(method) = audio_declared_method(provider_channel, effect) else {
            return HostEnforcementResult::Failed;
        };
        debug_assert!(
            AUDIO_DECLARED_METHODS.contains(&method),
            "the applied method comes from the family's declared vocabulary"
        );
        let pinned = match self.pinning(channel) {
            Ok(pinned) => pinned,
            Err(failure) => {
                tracing::warn!(
                    method = method.name(),
                    channel = ?channel,
                    reason = ?failure,
                    "audio host effect refused: the channel's endpoint relationship is not usable"
                );
                return HostEnforcementResult::Failed;
            }
        };
        let node = match pinned.select_node(self.vm_id.as_str(), channel) {
            Ok(node) => node,
            Err(failure) => {
                tracing::warn!(
                    method = method.name(),
                    channel = ?channel,
                    reason = ?failure,
                    "audio host effect refused: the admitted session named no single stream"
                );
                return HostEnforcementResult::Failed;
            }
        };
        match pinned.apply(method, &node, argument) {
            Ok(()) => HostEnforcementResult::Applied,
            Err(failure) => {
                tracing::warn!(
                    method = method.name(),
                    channel = ?channel,
                    reason = ?failure,
                    "audio host effect refused: the pinned session did not apply the method"
                );
                HostEnforcementResult::Failed
            }
        }
    }
}

impl HostAudioController for AdmittedPipeWireHostController {
    fn enforce_grant(&self, grant: AudioGrant, channel: AudioChannel) -> HostEnforcementResult {
        self.apply(channel, AudioEffectKind::Grant, grant.as_wire_str())
    }

    fn enforce_level(&self, level: LevelPercent, channel: AudioChannel) -> HostEnforcementResult {
        let percent = level.get().to_string();
        self.apply(channel, AudioEffectKind::Level, &percent)
    }
}

// ── PipeWireHostController (retained legacy adapter) ──────────────────────────

/// The pre-graph host controller: a broker audio action over the authenticated
/// broker transport, addressed by the runner's legacy intent identity.
///
/// This is the adapter the retired CLI path still uses. The v3 graph
/// composition must not reach it: new-graph audio carries each channel's
/// effect on the admitted [`AdmittedPipeWireHostController`] instead, and the
/// broker's audio-specific operation, its wire types, and this adapter are
/// removed together once the retained path is gone.
#[derive(Debug, Clone)]
pub(crate) struct PipeWireHostController {
    broker_socket: PathBuf,
    caller_role: BrokerCallerRole,
    vm_id: VmId,
    role_id: RoleId,
    bundle_runner_intent_ref: BundleOpId,
}

impl PipeWireHostController {
    /// Find the audio runner node for a VM in a loaded [`ProcessesJson`].
    pub(crate) fn find_audio_node<'a>(
        processes: &'a ProcessesJson,
        vm_name: &str,
    ) -> Option<&'a ProcessNode> {
        AdmittedPipeWireHostController::find_audio_node(processes, vm_name)
    }

    /// Construct from the audio runner [`ProcessNode`] identity and the
    /// daemon's authenticated broker transport.
    ///
    /// Tool paths, runtime paths, and node identifiers are resolved only by
    /// the broker from the trusted runner intent.
    pub(crate) fn from_audio_node(
        node: &ProcessNode,
        vm_name: &str,
        broker_socket: PathBuf,
        caller_role: BrokerCallerRole,
    ) -> Self {
        Self {
            broker_socket,
            caller_role,
            vm_id: VmId::new(vm_name),
            role_id: RoleId::new(node.id.0.clone()),
            bundle_runner_intent_ref: BundleOpId::new(intent_id_legacy_runner(vm_name, &node.id.0)),
        }
    }

    fn dispatch_effect(
        &self,
        channel: AudioChannel,
        action: PipeWireAudioAction,
    ) -> HostEnforcementResult {
        let channel = match channel {
            AudioChannel::Speaker => PipeWireAudioChannel::Speaker,
            AudioChannel::Microphone => PipeWireAudioChannel::Microphone,
        };
        let request = BrokerRequest::PipeWireAudio(PipeWireAudioRequest {
            vm_id: self.vm_id.clone(),
            role_id: self.role_id.clone(),
            bundle_runner_intent_ref: self.bundle_runner_intent_ref.clone(),
            channel,
            action,
            tracing_span_id: None,
        });
        match crate::dispatch_broker_request_to_socket(
            &self.broker_socket,
            request,
            self.caller_role.clone(),
            Some(Duration::from_secs(10)),
        ) {
            Ok(BrokerResponse::PipeWireAudio(response)) if response.applied => {
                HostEnforcementResult::Applied
            }
            Ok(BrokerResponse::PipeWireAudio(response)) if !response.host_ready => {
                HostEnforcementResult::Failed
            }
            Ok(BrokerResponse::PipeWireAudio(_)) => HostEnforcementResult::Unsupported,
            Ok(BrokerResponse::Error(_)) | Err(_) | Ok(_) => HostEnforcementResult::Failed,
        }
    }
}

impl HostAudioController for PipeWireHostController {
    fn enforce_grant(&self, grant: AudioGrant, channel: AudioChannel) -> HostEnforcementResult {
        self.dispatch_effect(channel, PipeWireAudioAction::SetGrant { on: grant.is_on() })
    }

    fn enforce_level(&self, level: LevelPercent, channel: AudioChannel) -> HostEnforcementResult {
        self.dispatch_effect(
            channel,
            PipeWireAudioAction::SetLevel {
                percent: level.get(),
            },
        )
    }
}

// ── QemuAudioController ──────────────────────────────────────────────────────


/// Offline-only host controller for qemu-media VMs.
///
/// qemu-media VMs have no vhost-user-sound sidecar; the qemu audio backend
/// is configured at VM start time. The state-file write that the dispatch
/// layer performs BEFORE calling the controller is the authoritative policy
/// change - the next VM restart picks up the new policy.
///
/// This controller returns [`HostEnforcementResult::Applied`] to signal that
/// the offline policy has been committed, not that live runtime enforcement
/// occurred. The response's `applied` field will be `HostOnly`, which is
/// accurate: the host state file is updated; there is no guest enforcement
/// path for qemu-media VMs.
///
/// The controller never calls a target-local Process - the qemu-media capability row has
/// `guest_enforcement = Unsupported`, and that invariant is enforced at the
/// dispatch layer, not here.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct QemuAudioController;

impl HostAudioController for QemuAudioController {
    fn enforce_grant(
        &self,
        _grant: AudioGrant,
        _channel: AudioChannel,
    ) -> HostEnforcementResult {
        // Offline policy committed by the state-file write in the dispatch
        // layer. Return Applied so the response reflects the actual state.
        HostEnforcementResult::Applied
    }

    fn enforce_level(
        &self,
        _level: LevelPercent,
        _channel: AudioChannel,
    ) -> HostEnforcementResult {
        HostEnforcementResult::Applied
    }
}

// ── FakeHostController ───────────────────────────────────────────────────────

/// Configurable fake controller for tests.
///
/// Gated behind `#[cfg(test)]` so it never compiles into production builds.
///
/// **Tests must set results explicitly.** There is intentionally NO default
/// that returns `Applied` - callers that forget to configure the fake will
/// get `Failed`, surfacing the omission.
#[cfg(test)]
#[derive(Debug, Clone)]
pub struct FakeHostController {
    /// Result returned by [`HostAudioController::enforce_grant`].
    pub grant_result: HostEnforcementResult,
    /// Result returned by [`HostAudioController::enforce_level`].
    pub level_result: HostEnforcementResult,
}

#[cfg(test)]
impl FakeHostController {
    /// Build a fake that simulates successful enforcement on both channels.
    pub fn success() -> Self {
        Self {
            grant_result: HostEnforcementResult::Applied,
            level_result: HostEnforcementResult::Applied,
        }
    }

    /// Build a fake that simulates a subprocess failure on both channels.
    pub fn failed() -> Self {
        Self {
            grant_result: HostEnforcementResult::Failed,
            level_result: HostEnforcementResult::Failed,
        }
    }
}

#[cfg(test)]
impl HostAudioController for FakeHostController {
    fn enforce_grant(
        &self,
        _grant: AudioGrant,
        _channel: AudioChannel,
    ) -> HostEnforcementResult {
        self.grant_result
    }

    fn enforce_level(
        &self,
        _level: LevelPercent,
        _channel: AudioChannel,
    ) -> HostEnforcementResult {
        self.level_result
    }
}

// ── private helpers ──────────────────────────────────────────────────────────

/// The PipeWire media class one channel's stream is selected by.
fn channel_media_class(channel: AudioChannel) -> &'static str {
    match channel {
        AudioChannel::Speaker => "Stream/Output/Audio",
        AudioChannel::Microphone => "Stream/Input/Audio",
    }
}

/// The single node one channel's admitted session owns.
///
/// The vhost-user-sound sidecar is launched with
/// `application.name = "d2b-<vm>"`, so the node is selected by that name plus
/// the channel's media class: a speaker control never reaches the
/// microphone's node and vice versa. Two matching nodes are a refusal, not a
/// choice.
fn target_node_from_pw_dump(bytes: &[u8], vm_name: &str, channel: AudioChannel) -> Option<String> {
    let docs: Value = serde_json::from_slice(bytes).ok()?;
    let array = docs.as_array()?;
    let expected_app = format!("d2b-{vm_name}");
    let expected_class = channel_media_class(channel);
    let mut matches = array.iter().filter_map(|entry| {
        let props = entry.get("info")?.get("props")?;
        let app = props.get("application.name")?.as_str()?;
        let media_class = props.get("media.class")?.as_str()?;
        if app != expected_app || media_class != expected_class {
            return None;
        }
        entry
            .get("id")
            .and_then(Value::as_u64)
            .map(|id| id.to_string())
    });
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(first)
}

// ── unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_control::public_wire::AudioChannel;
    use d2b_contracts_resource::v3::{
        BindingSlot, EndpointAttachmentKind, EndpointBindingRequest, ResourceGeneration,
        ResourceRef, ResourceUid, ZoneRevision, execution_policy::BoundedToken,
    };
    use d2b_core::processes::{ProcessRole, VmProcessInvariants};
    use d2b_provider_audio_pipewire::{
        AdmittedAudioSession, AudioBindingFence, AudioChannel as ProviderChannel,
        AudioMediatorError, AudioSessionOrigin,
    };
    use std::os::unix::fs::PermissionsExt as _;

    fn uid(byte: u8) -> ResourceUid {
        ResourceUid::from_bytes(&[byte; 16]).expect("uuid")
    }

    fn fence(byte: u8, generation: u64) -> AudioBindingFence {
        AudioBindingFence::new(
            uid(byte),
            ResourceGeneration::new(generation).expect("generation"),
            ZoneRevision::new(7),
        )
    }

    fn request(channel: ProviderChannel, endpoint: &str) -> EndpointBindingRequest {
        EndpointBindingRequest::new(
            ResourceRef::parse(endpoint).expect("endpoint"),
            ResourceRef::parse("Guest/workstation").expect("guest"),
            BindingSlot::parse(channel.binding_slot()).expect("slot"),
            EndpointAttachmentKind::Connect,
            BoundedToken::parse(channel.declared_purpose()).expect("purpose"),
        )
        .expect("endpoint binding request")
    }

    fn session(
        channel: ProviderChannel,
        endpoint: &str,
        fence: &AudioBindingFence,
        origin: AudioSessionOrigin,
    ) -> AdmittedAudioSession {
        AdmittedAudioSession::new(channel, request(channel, endpoint), fence.clone(), origin)
            .expect("admitted session")
    }

    /// A pinned session whose stub tools record what they were asked to do.
    struct Pinning {
        dir: tempfile::TempDir,
        log: PathBuf,
    }

    impl Pinning {
        fn new(dump_nodes: &str, wpctl_exit: &str) -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let log = dir.path().join("invocations.log");
            let dump = dir.path().join("pw-dump");
            // The stub runs with a cleared environment, so it uses shell
            // builtins only and never depends on PATH.
            std::fs::write(
                &dump,
                format!("#!/bin/sh\nprintf '%s\\n' '{dump_nodes}'\n"),
            )
            .expect("write pw-dump");
            std::fs::set_permissions(&dump, std::fs::Permissions::from_mode(0o755))
                .expect("chmod pw-dump");
            let wpctl = dir.path().join("wpctl");
            std::fs::write(
                &wpctl,
                format!(
                    "#!/bin/sh\necho \"$PIPEWIRE_RUNTIME_DIR|$*\" >> {}\nexit {}\n",
                    log.display(),
                    wpctl_exit
                ),
            )
            .expect("write wpctl");
            std::fs::set_permissions(&wpctl, std::fs::Permissions::from_mode(0o755))
                .expect("chmod wpctl");
            std::fs::write(dir.path().join(PIPEWIRE_RUNTIME_SOCKET), b"").expect("socket");
            Self { dir, log }
        }

        fn sources(&self) -> AudioSessionPinningSources {
            AudioSessionPinningSources {
                runtime_dir: self.dir.path().to_path_buf(),
                wpctl: self.dir.path().join("wpctl"),
                pw_dump: self.dir.path().join("pw-dump"),
            }
        }

        /// A plan holding the admitted relationship of every channel given.
        fn plan(
            sessions: &[(ProviderChannel, AdmittedAudioSession)],
        ) -> (AudioSessionPlan, Vec<AudioBindingFence>) {
            let mut speaker = None;
            let mut microphone = None;
            let mut observed = Vec::with_capacity(sessions.len());
            for (channel, session) in sessions {
                observed.push(session.fence().clone());
                match channel {
                    ProviderChannel::Speaker => speaker = Some(session.clone()),
                    ProviderChannel::Microphone => microphone = Some(session.clone()),
                }
            }
            (AudioSessionPlan::new(speaker, microphone), observed)
        }

        fn controller(
            &self,
            sessions: Vec<(ProviderChannel, AdmittedAudioSession)>,
        ) -> AdmittedPipeWireHostController {
            let (plan, observed) = Self::plan(&sessions);
            AdmittedPipeWireHostController::new("corp-vm", plan, observed, self.sources())
        }

        fn invocations(&self) -> String {
            std::fs::read_to_string(&self.log).unwrap_or_default()
        }
    }

    const TWO_NODES: &str = r#"[
      {"id": 41, "info": {"props": {"application.name": "d2b-corp-vm", "media.class": "Stream/Output/Audio"}}},
      {"id": 42, "info": {"props": {"application.name": "d2b-corp-vm", "media.class": "Stream/Input/Audio"}}}
    ]"#;
    const SPEAKER_ONLY: &str =
        r#"[{"id": 41, "info": {"props": {"application.name": "d2b-corp-vm", "media.class": "Stream/Output/Audio"}}}]"#;
    const TWO_SPEAKER_NODES: &str = r#"[
      {"id": 41, "info": {"props": {"application.name": "d2b-corp-vm", "media.class": "Stream/Output/Audio"}}},
      {"id": 42, "info": {"props": {"application.name": "d2b-corp-vm", "media.class": "Stream/Output/Audio"}}}
    ]"#;

    fn speaker_session(fence: &AudioBindingFence, origin: AudioSessionOrigin) -> AdmittedAudioSession {
        session(
            ProviderChannel::Speaker,
            "Endpoint/audio-speaker",
            fence,
            origin,
        )
    }

    fn microphone_session(
        fence: &AudioBindingFence,
        origin: AudioSessionOrigin,
    ) -> AdmittedAudioSession {
        session(
            ProviderChannel::Microphone,
            "Endpoint/audio-microphone",
            fence,
            origin,
        )
    }

    #[test]
    fn qemu_controller_grant_is_applied() {
        let ctrl = QemuAudioController;
        assert_eq!(
            ctrl.enforce_grant(AudioGrant::Off, AudioChannel::Speaker),
            HostEnforcementResult::Applied,
        );
    }

    #[test]
    fn an_admitted_session_applies_each_channel_through_its_own_relationship() {
        let pinning = Pinning::new(TWO_NODES, "0");
        let speaker_fence = fence(0x11, 1);
        let microphone_fence = fence(0x22, 1);
        let controller = pinning.controller(vec![
            (
                ProviderChannel::Speaker,
                speaker_session(&speaker_fence, AudioSessionOrigin::Owner),
            ),
            (
                ProviderChannel::Microphone,
                microphone_session(&microphone_fence, AudioSessionOrigin::Owner),
            ),
        ]);

        assert_eq!(
            controller.enforce_grant(AudioGrant::On, AudioChannel::Speaker),
            HostEnforcementResult::Applied
        );
        assert_eq!(
            controller.enforce_level(
                LevelPercent::new(40).expect("level"),
                AudioChannel::Microphone
            ),
            HostEnforcementResult::Applied
        );
        let invocations = pinning.invocations();
        assert!(
            invocations.contains("set-mute 41 0"),
            "the speaker grant rides the speaker node: {invocations}"
        );
        assert!(
            invocations.contains("set-volume 42 40"),
            "the microphone gain rides the microphone node: {invocations}"
        );
    }

    #[test]
    fn a_channel_without_its_own_admission_never_reaches_the_session() {
        let pinning = Pinning::new(SPEAKER_ONLY, "0");
        let speaker_fence = fence(0x11, 1);
        let controller = pinning.controller(vec![(
            ProviderChannel::Speaker,
            speaker_session(&speaker_fence, AudioSessionOrigin::Owner),
        )]);

        assert_eq!(
            controller.enforce_grant(AudioGrant::On, AudioChannel::Speaker),
            HostEnforcementResult::Applied
        );
        assert_eq!(
            controller.enforce_grant(AudioGrant::On, AudioChannel::Microphone),
            HostEnforcementResult::Failed,
            "the microphone has no admitted relationship of its own"
        );
        assert_eq!(
            pinning.invocations().matches("set-mute").count(),
            1,
            "the microphone refusal never reached the pinned session"
        );
    }

    #[test]
    fn a_revoked_relationship_stops_serving_effects() {
        let pinning = Pinning::new(SPEAKER_ONLY, "0");
        let admitted = fence(0x11, 1);
        let re_admitted = fence(0x11, 2);
        let admitted_session = speaker_session(&admitted, AudioSessionOrigin::Owner);

        let stale = AdmittedPipeWireHostController::new(
            "corp-vm",
            AudioSessionPlan::new(Some(admitted_session.clone()), None),
            vec![re_admitted],
            pinning.sources(),
        );
        assert_eq!(
            stale.enforce_grant(AudioGrant::On, AudioChannel::Speaker),
            HostEnforcementResult::Failed,
            "the observed fence moved past the admitted one"
        );
        assert_eq!(pinning.invocations(), "", "the stale pass ran no tool");

        let fresh = AdmittedPipeWireHostController::new(
            "corp-vm",
            AudioSessionPlan::new(Some(admitted_session), None),
            vec![admitted],
            pinning.sources(),
        );
        assert_eq!(
            fresh.enforce_grant(AudioGrant::On, AudioChannel::Speaker),
            HostEnforcementResult::Applied,
            "the same relationship is admitted again against its own fence"
        );
    }

    #[test]
    fn an_imported_projection_cannot_mint_a_host_grant() {
        let pinning = Pinning::new(SPEAKER_ONLY, "0");
        let projected = fence(0x33, 1);
        let controller = pinning.controller(vec![(
            ProviderChannel::Speaker,
            speaker_session(&projected, AudioSessionOrigin::ImportedProjection),
        )]);
        assert_eq!(
            controller.enforce_grant(AudioGrant::On, AudioChannel::Speaker),
            HostEnforcementResult::Failed,
            "an imported Service projection has no local host session to grant through"
        );
        assert_eq!(pinning.invocations(), "", "no tool ran for the projection");
    }

    #[test]
    fn the_pinned_session_never_runs_a_tool_against_an_ambient_directory() {
        let pinning = Pinning::new(SPEAKER_ONLY, "0");
        let speaker_fence = fence(0x11, 1);
        let controller = pinning.controller(vec![(
            ProviderChannel::Speaker,
            speaker_session(&speaker_fence, AudioSessionOrigin::Owner),
        )]);
        assert_eq!(
            controller.enforce_grant(AudioGrant::On, AudioChannel::Speaker),
            HostEnforcementResult::Applied
        );
        let pinned_dir = pinning.dir.path().to_string_lossy().to_string();
        let invocations = pinning.invocations();
        assert_eq!(
            invocations.lines().count(),
            1,
            "one admitted effect is one tool invocation"
        );
        for line in invocations.lines() {
            let runtime_dir = line.split('|').next().expect("runtime dir column");
            assert_eq!(
                runtime_dir, pinned_dir,
                "the tool ran with a cleared environment against the admitted delivery"
            );
        }
    }

    #[test]
    fn a_relative_or_absent_pinned_location_is_never_a_delivery() {
        let pinning = Pinning::new(SPEAKER_ONLY, "0");
        let speaker_fence = fence(0x11, 1);
        let admitted = speaker_session(&speaker_fence, AudioSessionOrigin::Owner);
        assert_eq!(
            AudioSessionPinning::admit(
                &admitted,
                &speaker_fence,
                PathBuf::from("run/user/1000"),
                PathBuf::from("/bin/wpctl"),
                PathBuf::from("/bin/pw-dump"),
            ),
            Err(AudioMediatorError::ProviderSessionUnavailable),
            "a relative runtime directory is not a session the owner pinned"
        );
        assert_eq!(
            AudioSessionPinning::admit(
                &admitted,
                &speaker_fence,
                pinning.dir.path().to_path_buf(),
                PathBuf::from("wpctl"),
                PathBuf::from("pw-dump"),
            ),
            Err(AudioMediatorError::ProviderSessionUnavailable),
            "a relative tool path is not a pinned executable"
        );
        std::fs::remove_file(pinning.dir.path().join(PIPEWIRE_RUNTIME_SOCKET))
            .expect("remove the socket");
        assert_eq!(
            AudioSessionPinning::admit(
                &admitted,
                &speaker_fence,
                pinning.dir.path().to_path_buf(),
                pinning.dir.path().join("wpctl"),
                pinning.dir.path().join("pw-dump"),
            ),
            Err(AudioMediatorError::ProviderSessionUnavailable),
            "a delivery the endpoint owner cannot see is not a delivery"
        );
    }

    #[test]
    fn an_ambiguous_session_never_acts_on_a_chosen_node() {
        let pinning = Pinning::new(TWO_SPEAKER_NODES, "0");
        let speaker_fence = fence(0x11, 1);
        let controller = pinning.controller(vec![(
            ProviderChannel::Speaker,
            speaker_session(&speaker_fence, AudioSessionOrigin::Owner),
        )]);
        assert_eq!(
            controller.enforce_grant(AudioGrant::On, AudioChannel::Speaker),
            HostEnforcementResult::Failed
        );
        assert_eq!(
            pinning.invocations(),
            "",
            "two candidate streams is a refusal, not a choice between them"
        );
    }

    #[test]
    fn a_failed_mutation_is_reported_rather_than_applied() {
        let pinning = Pinning::new(SPEAKER_ONLY, "1");
        let speaker_fence = fence(0x11, 1);
        let controller = pinning.controller(vec![(
            ProviderChannel::Speaker,
            speaker_session(&speaker_fence, AudioSessionOrigin::Owner),
        )]);
        assert_eq!(
            controller.enforce_grant(AudioGrant::Off, AudioChannel::Speaker),
            HostEnforcementResult::Failed,
            "a non-zero exit never reports an applied host effect"
        );
        assert!(
            pinning.invocations().contains("set-mute 41 1"),
            "the mute was still attempted against the admitted node"
        );
    }

    #[test]
    fn pw_dump_target_selects_requested_channel() {
        let dump = br#"[
          {"id": 41, "info": {"props": {"application.name": "d2b-corp", "media.class": "Stream/Output/Audio"}}},
          {"id": 42, "info": {"props": {"application.name": "d2b-corp", "media.class": "Stream/Input/Audio"}}}
        ]"#;
        assert_eq!(
            target_node_from_pw_dump(dump, "corp", AudioChannel::Speaker).as_deref(),
            Some("41")
        );
        assert_eq!(
            target_node_from_pw_dump(dump, "corp", AudioChannel::Microphone).as_deref(),
            Some("42")
        );
    }

    #[test]
    fn pw_dump_target_rejects_ambiguous_channel() {
        let dump = br#"[
          {"id": 41, "info": {"props": {"application.name": "d2b-corp", "media.class": "Stream/Output/Audio"}}},
          {"id": 42, "info": {"props": {"application.name": "d2b-corp", "media.class": "Stream/Output/Audio"}}}
        ]"#;
        assert_eq!(
            target_node_from_pw_dump(dump, "corp", AudioChannel::Speaker),
            None
        );
    }

    #[test]
    fn find_audio_node_returns_none_when_absent() {
        let processes = ProcessesJson {
            schema_version: "v3".to_owned(),
            vms: vec![VmProcessDag {
                workload_identity: None,
                vm: "corp-vm".to_owned(),
                nodes: vec![],
                edges: vec![],
                invariants: VmProcessInvariants {
                    swtpm_pre_start_flush: false,
                    per_vm_audit_pipeline: false,
                    usbip_gating: false,
                    tpm_ownership_migration_without_running_vm_mutation: false,
                },
            }],
        };
        assert!(
            AdmittedPipeWireHostController::find_audio_node(&processes, "corp-vm").is_none()
        );
    }

    #[test]
    fn find_audio_node_returns_audio_role() {
        let audio_node = make_audio_node();
        let processes = ProcessesJson {
            schema_version: "v3".to_owned(),
            vms: vec![VmProcessDag {
                workload_identity: None,
                vm: "corp-vm".to_owned(),
                nodes: vec![audio_node.clone()],
                edges: vec![],
                invariants: VmProcessInvariants {
                    swtpm_pre_start_flush: false,
                    per_vm_audit_pipeline: false,
                    usbip_gating: false,
                    tpm_ownership_migration_without_running_vm_mutation: false,
                },
            }],
        };
        let found = AdmittedPipeWireHostController::find_audio_node(&processes, "corp-vm");
        assert!(matches!(
            found.map(|node| &node.role),
            Some(ProcessRole::Audio)
        ));
    }

    fn make_audio_node() -> ProcessNode {
        use d2b_core::sandbox_profile::{CgroupPlacement, MountPolicy, NamespaceSet};
        use d2b_core::processes::{NodeId, RoleProfile};

        ProcessNode {
            execution_ref: None,
            execution_domain: None,
            user_ref: None,
            id: NodeId("audio".to_owned()),
            role: ProcessRole::Audio,
            unit: None,
            binary_path: Some("/run/d2b/vms/corp-vm/d2b-corp-vm".to_owned()),
            argv: vec!["d2b-corp-vm-snd".to_owned()],
            env: vec![
                "PIPEWIRE_RUNTIME_DIR=/run/user/1000".to_owned(),
                "WPCTL_PATH=/nix/store/wpctl/bin/wpctl".to_owned(),
            ],
            plan_ops: vec![],
            network_interfaces: Vec::new(),
            profile: RoleProfile {
                profile_id: "w1-audio".to_owned(),
                uid: 60100,
                gid: 60100,
                adr_carve_out: None,
                caps: vec![],
                namespaces: NamespaceSet {
                    mount: false,
                    pid: false,
                    net: false,
                    ipc: false,
                    uts: false,
                    user: false,
                },
                seccomp_policy_ref: Some("w1-audio".to_owned()),
                mount_policy: MountPolicy {
                    read_only_paths: vec![],
                    writable_paths: vec![],
                    nix_store_read_only: false,
                    hide_device_nodes_by_default: false,
                    device_binds: vec![],
                    bind_mounts: vec![],
                },
                cgroup_placement: CgroupPlacement {
                    subtree: "d2b.slice/corp-vm/audio".to_owned(),
                    controllers: vec![],
                    delegated: false,
                },
                user_namespace: None,
                umask: None,
            },
            readiness: vec![],
        }
    }
}
