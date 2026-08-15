//! Host-side audio controller strategy (ADR 0041).
//!
//! Defines [`HostAudioController`], a typed trait for host-side audio
//! enforcement, plus concrete implementations:
//!
//! * [`PipeWireHostController`] - argv-only `wpctl` subprocess targeting
//!   the per-VM vhost-user-sound PipeWire stream for the requested direction.
//!   Credential-aware: the
//!   PipeWire socket access is probed with `access(2)` before any
//!   subprocess is spawned. Returns [`HostEnforcementResult::Failed`] (not
//!   `Unsupported`) when the credential check fails so callers know `off`
//!   did **not** seal the host boundary.
//! * [`QemuAudioController`] - offline-only enforcement for qemu-media VMs.
//!   Writing the state file IS the policy for qemu-media; no live runtime
//!   enforcement exists and no guestd call is made.
//! * `FakeHostController` - test-only injectable with configurable results.
//!   Gated behind `#[cfg(test)]` so it never compiles into production builds.
//!
//! ## PipeWire node targeting
//!
//! The vhost-user-sound sidecar is launched with
//! `PIPEWIRE_PROPS={ application.name = "d2b-<vm>" ... }`. The controller
//! resolves the live PipeWire node id with `pw-dump`, filtering by
//! `application.name` plus `media.class` so speaker and microphone controls do
//! not target the same ambiguous node name.
//!
//! ## Credential posture
//!
//! `d2bd` runs as the `d2bd` system user, which does NOT have PipeWire socket
//! access by default. Access is granted explicitly by the broker's
//! `SetSocketAcl` pre-spawn path for audio runners. The controller checks
//! `access(2)` on `<PIPEWIRE_RUNTIME_DIR>/pipewire-0` with `WRITE_OK`
//! before spawning any subprocess. If the check fails, `Failed` is returned and
//! the dispatcher does not persist the requested policy as applied.

use std::path::PathBuf;
use std::time::Duration;

use d2b_contracts::{
    broker_wire::{
        BrokerCallerRole, BrokerRequest, BrokerResponse, PipeWireAudioAction, PipeWireAudioChannel,
        PipeWireAudioRequest,
    },
    public_wire::AudioChannel,
    types::{BundleOpId, RoleId, VmId},
};
use d2b_core::audio_policy::{AudioGrant, LevelPercent};
use d2b_core::bundle_resolver::intent_id_runner;
use d2b_core::processes::{ProcessNode, ProcessRole, ProcessesJson, VmProcessDag};
#[cfg(test)]
use serde_json::Value;

pub use crate::audio_dispatch::HostEnforcementResult;

// ── trait ────────────────────────────────────────────────────────────────────

/// Strategy for host-side audio enforcement.
///
/// The trait is `dyn`-safe so dispatch functions can accept `&dyn
/// HostAudioController` and tests can inject a fake.
pub trait HostAudioController {
    /// Enforce a mute/unmute grant on a running VM's audio node.
    ///
    /// Returns [`HostEnforcementResult::Applied`] only when enforcement was
    /// confirmed (subprocess exited 0). Returns `Failed` on subprocess error
    /// or credential failure. Returns `Unsupported` only for offline-only
    /// providers where no live enforcement path exists.
    fn enforce_grant(
        &self,
        vm_name: &str,
        grant: AudioGrant,
        channel: AudioChannel,
    ) -> HostEnforcementResult;

    /// Enforce a volume/gain level change on a running VM's audio node.
    ///
    /// Same success/failure contract as [`Self::enforce_grant`].
    fn enforce_level(
        &self,
        vm_name: &str,
        level: LevelPercent,
        channel: AudioChannel,
    ) -> HostEnforcementResult;
}

// ── PipeWireHostController ───────────────────────────────────────────────────

/// Daemon-side handle for broker-owned PipeWire effects.
///
/// The controller retains only opaque bundle identities and the authenticated
/// broker transport. Tool paths, runtime paths, and node identifiers remain
/// broker-local.
#[derive(Debug, Clone)]
pub struct PipeWireHostController {
    broker_socket: PathBuf,
    caller_role: BrokerCallerRole,
    vm_id: VmId,
    role_id: RoleId,
    bundle_runner_intent_ref: BundleOpId,
}

impl PipeWireHostController {
    /// Construct from the audio runner [`ProcessNode`] identity and the
    /// daemon's authenticated broker transport.
    ///
    /// Tool paths, runtime paths, and node identifiers are resolved only by
    /// the broker from the trusted runner intent.
    pub fn from_audio_node(
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
            bundle_runner_intent_ref: BundleOpId::new(intent_id_runner(vm_name, &node.id.0)),
        }
    }

    /// Find the audio runner node for a VM in a loaded [`ProcessesJson`].
    ///
    /// Returns `None` when no audio node exists (VM has no audio sidecar).
    pub fn find_audio_node<'a>(
        processes: &'a ProcessesJson,
        vm_name: &str,
    ) -> Option<&'a ProcessNode> {
        let vm_dag: &VmProcessDag = processes.vms.iter().find(|v| v.vm == vm_name)?;
        vm_dag
            .nodes
            .iter()
            .find(|n| matches!(n.role, ProcessRole::Audio))
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
    fn enforce_grant(
        &self,
        _vm_name: &str,
        grant: AudioGrant,
        channel: AudioChannel,
    ) -> HostEnforcementResult {
        self.dispatch_effect(channel, PipeWireAudioAction::SetGrant { on: grant.is_on() })
    }

    fn enforce_level(
        &self,
        _vm_name: &str,
        level: LevelPercent,
        channel: AudioChannel,
    ) -> HostEnforcementResult {
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
/// The controller never calls guestd - the qemu-media capability row has
/// `guest_enforcement = Unsupported`, and that invariant is enforced at the
/// dispatch layer, not here.
#[derive(Debug, Clone, Copy, Default)]
pub struct QemuAudioController;

impl HostAudioController for QemuAudioController {
    fn enforce_grant(
        &self,
        _vm_name: &str,
        _grant: AudioGrant,
        _channel: AudioChannel,
    ) -> HostEnforcementResult {
        // Offline policy committed by the state-file write in the dispatch
        // layer. Return Applied so the response reflects the actual state.
        HostEnforcementResult::Applied
    }

    fn enforce_level(
        &self,
        _vm_name: &str,
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

    /// Build a fake that simulates an unsupported/unavailable enforcement.
    pub fn unsupported() -> Self {
        Self {
            grant_result: HostEnforcementResult::Unsupported,
            level_result: HostEnforcementResult::Unsupported,
        }
    }
}

#[cfg(test)]
impl HostAudioController for FakeHostController {
    fn enforce_grant(
        &self,
        _vm_name: &str,
        _grant: AudioGrant,
        _channel: AudioChannel,
    ) -> HostEnforcementResult {
        self.grant_result
    }

    fn enforce_level(
        &self,
        _vm_name: &str,
        _level: LevelPercent,
        _channel: AudioChannel,
    ) -> HostEnforcementResult {
        self.level_result
    }
}

// ── private helpers ──────────────────────────────────────────────────────────

#[cfg(test)]
fn channel_media_class(channel: AudioChannel) -> &'static str {
    match channel {
        AudioChannel::Speaker => "Stream/Output/Audio",
        AudioChannel::Microphone => "Stream/Input/Audio",
    }
}

#[cfg(test)]
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
    use d2b_contracts::public_wire::AudioChannel;
    use d2b_core::audio_policy::{AudioGrant, LevelPercent};

    // ── FakeHostController ──────────────────────────────────────────────────

    #[test]
    fn fake_success_returns_applied_for_grant() {
        let ctrl = FakeHostController::success();
        assert_eq!(
            ctrl.enforce_grant("corp-vm", AudioGrant::Off, AudioChannel::Speaker),
            HostEnforcementResult::Applied,
        );
    }

    #[test]
    fn fake_success_returns_applied_for_level() {
        let ctrl = FakeHostController::success();
        let level = LevelPercent::new(75).unwrap();
        assert_eq!(
            ctrl.enforce_level("corp-vm", level, AudioChannel::Speaker),
            HostEnforcementResult::Applied,
        );
    }

    #[test]
    fn fake_failed_returns_failed_for_grant() {
        let ctrl = FakeHostController::failed();
        assert_eq!(
            ctrl.enforce_grant("corp-vm", AudioGrant::Off, AudioChannel::Speaker),
            HostEnforcementResult::Failed,
        );
    }

    #[test]
    fn fake_failed_returns_failed_for_level() {
        let ctrl = FakeHostController::failed();
        let level = LevelPercent::new(50).unwrap();
        assert_eq!(
            ctrl.enforce_level("corp-vm", level, AudioChannel::Microphone),
            HostEnforcementResult::Failed,
        );
    }

    #[test]
    fn fake_unsupported_returns_unsupported() {
        let ctrl = FakeHostController::unsupported();
        assert_eq!(
            ctrl.enforce_grant("corp-vm", AudioGrant::Off, AudioChannel::Microphone),
            HostEnforcementResult::Unsupported,
        );
        let level = LevelPercent::new(20).unwrap();
        assert_eq!(
            ctrl.enforce_level("corp-vm", level, AudioChannel::Speaker),
            HostEnforcementResult::Unsupported,
        );
    }

    // ── QemuAudioController ─────────────────────────────────────────────────

    #[test]
    fn qemu_controller_grant_is_applied() {
        let ctrl = QemuAudioController;
        assert_eq!(
            ctrl.enforce_grant("qemu-vm", AudioGrant::Off, AudioChannel::Speaker),
            HostEnforcementResult::Applied,
        );
    }

    #[test]
    fn qemu_controller_level_is_applied() {
        let ctrl = QemuAudioController;
        let level = LevelPercent::new(80).unwrap();
        assert_eq!(
            ctrl.enforce_level("qemu-vm", level, AudioChannel::Microphone),
            HostEnforcementResult::Applied,
        );
    }

    #[test]
    fn qemu_controller_on_grant_is_applied() {
        let ctrl = QemuAudioController;
        // Unmute (grant=On) should also return Applied for qemu-media.
        assert_eq!(
            ctrl.enforce_grant("qemu-vm", AudioGrant::On, AudioChannel::Speaker),
            HostEnforcementResult::Applied,
        );
    }

    // ── PipeWireHostController construction ─────────────────────────────────

    #[test]
    fn pipewire_controller_builds_from_runner_identity() {
        let node = make_audio_node(Vec::new());
        let ctrl = PipeWireHostController::from_audio_node(
            &node,
            "corp-vm",
            PathBuf::from("/run/d2b/priv.sock"),
            BrokerCallerRole::AdminUid { uid: 0 },
        );
        assert_eq!(ctrl.vm_id.as_str(), "corp-vm");
        assert_eq!(ctrl.role_id.as_str(), "audio");
        assert_eq!(
            ctrl.bundle_runner_intent_ref.as_str(),
            intent_id_runner("corp-vm", "audio")
        );
    }

    #[test]
    fn pipewire_broker_unavailable_fails_closed() {
        let node = make_audio_node(Vec::new());
        let ctrl = PipeWireHostController::from_audio_node(
            &node,
            "corp-vm",
            PathBuf::from("/nonexistent/d2b-priv.sock"),
            BrokerCallerRole::AdminUid { uid: 0 },
        );
        let result = ctrl.enforce_grant("corp-vm", AudioGrant::Off, AudioChannel::Speaker);
        assert_eq!(
            result,
            HostEnforcementResult::Failed,
            "broker transport failure must not report an applied host effect"
        );
    }

    // ── find_audio_node ─────────────────────────────────────────────────────

    #[test]
    fn find_audio_node_returns_none_when_absent() {
        use d2b_core::processes::{ProcessesJson, VmProcessDag, VmProcessInvariants};
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
        let result = PipeWireHostController::find_audio_node(&processes, "corp-vm");
        assert!(result.is_none());
    }

    #[test]
    fn find_audio_node_returns_audio_role() {
        use d2b_core::processes::{ProcessesJson, VmProcessDag, VmProcessInvariants};
        let audio_node = make_audio_node(vec![
            "PIPEWIRE_RUNTIME_DIR=/run/user/1000".to_owned(),
            "WPCTL_PATH=/nix/store/wpctl/bin/wpctl".to_owned(),
            "PW_DUMP_PATH=/nix/store/pipewire/bin/pw-dump".to_owned(),
        ]);
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
        let result = PipeWireHostController::find_audio_node(&processes, "corp-vm");
        assert!(result.is_some());
        assert!(matches!(result.unwrap().role, ProcessRole::Audio));
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

    // ── helper ───────────────────────────────────────────────────────────────

    fn make_audio_node(env: Vec<String>) -> ProcessNode {
        use d2b_core::minijail_profile::{CgroupPlacement, MountPolicy, NamespaceSet};
        use d2b_core::processes::{NodeId, ProcessRole, RoleProfile};

        ProcessNode {
            execution_ref: None,
            execution_domain: None,
            user_ref: None,
            id: NodeId("audio".to_owned()),
            role: ProcessRole::Audio,
            unit: None,
            binary_path: Some("/run/d2b/vms/corp-vm/d2b-corp-vm".to_owned()),
            argv: vec!["d2b-corp-vm-snd".to_owned()],
            env,
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
