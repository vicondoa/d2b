//! Host-side audio controller strategy (ADR 0041).
//!
//! Defines [`HostAudioController`], a typed trait for host-side audio
//! enforcement, plus concrete implementations:
//!
//! * [`PipeWireHostController`] — argv-only `wpctl` subprocess targeting
//!   the per-VM vhost-user-sound PipeWire stream for the requested direction.
//!   Credential-aware: the
//!   PipeWire socket access is probed with `access(2)` before any
//!   subprocess is spawned. Returns [`HostEnforcementResult::Failed`] (not
//!   `Unsupported`) when the credential check fails so callers know `off`
//!   did **not** seal the host boundary.
//! * [`QemuAudioController`] — offline-only enforcement for qemu-media VMs.
//!   Writing the state file IS the policy for qemu-media; no live runtime
//!   enforcement exists and no guestd call is made.
//! * [`FakeHostController`] — test-only injectable with configurable results.
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

use std::path::{Path, PathBuf};
use std::process::Stdio;

use d2b_contracts::public_wire::AudioChannel;
use d2b_core::audio_policy::{AudioGrant, LevelPercent};
use d2b_core::processes::{ProcessNode, ProcessRole, ProcessesJson, VmProcessDag};
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
    /// Same success/failure contract as [`enforce_grant`].
    fn enforce_level(
        &self,
        vm_name: &str,
        level: LevelPercent,
        channel: AudioChannel,
    ) -> HostEnforcementResult;
}

// ── PipeWireHostController ───────────────────────────────────────────────────

/// Absolute path to `wpctl` (from the WirePlumber Nix store derivation).
///
/// Extracted from the audio ProcessNode's `env` list under the key
/// `WPCTL_PATH`. Using a store path avoids PATH lookups and ensures the
/// binary is the same revision as the PipeWire/WirePlumber session.
#[derive(Debug, Clone)]
pub struct PipeWireHostController {
    /// Absolute path to `wpctl` binary (e.g.
    /// `/nix/store/<hash>-wireplumber-<ver>/bin/wpctl`).
    wpctl_path: PathBuf,
    /// Absolute path to `pw-dump` for channel-specific node discovery.
    pw_dump_path: PathBuf,
    /// PipeWire runtime directory (e.g. `/run/user/1000`).
    /// Sourced from `PIPEWIRE_RUNTIME_DIR` in the audio runner env.
    pipewire_runtime_dir: PathBuf,
}

impl PipeWireHostController {
    /// Construct from the audio runner [`ProcessNode`] env.
    ///
    /// Returns `None` when `WPCTL_PATH`, `PW_DUMP_PATH`, or
    /// `PIPEWIRE_RUNTIME_DIR` is absent from the node env — caller should fall back to returning
    /// `Unsupported`.
    pub fn from_audio_node(node: &ProcessNode) -> Option<Self> {
        let wpctl_path = extract_env_value(&node.env, "WPCTL_PATH")?;
        let pw_dump_path = extract_env_value(&node.env, "PW_DUMP_PATH")?;
        let pipewire_runtime_dir = extract_env_value(&node.env, "PIPEWIRE_RUNTIME_DIR")?;
        Some(Self {
            wpctl_path: PathBuf::from(wpctl_path),
            pw_dump_path: PathBuf::from(pw_dump_path),
            pipewire_runtime_dir: PathBuf::from(pipewire_runtime_dir),
        })
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

    /// Probe whether `d2bd` holds the necessary credentials to reach the
    /// PipeWire socket.
    ///
    /// Uses `rustix::fs::access` with `WRITE_OK` on
    /// `<pipewire_runtime_dir>/pipewire-0`.  This is an explicit credential
    /// posture check per ADR 0041 — d2bd MUST NOT traverse `/run/user/<uid>`
    /// without first confirming access.
    fn has_pipewire_access(&self) -> bool {
        let socket = self.pipewire_runtime_dir.join("pipewire-0");
        // access(2) checks the *process* credentials, not file-descriptor ACLs,
        // which is exactly what we need: does the current d2bd UID/GID have
        // write access to the socket? Linux unix(7) requires write permission
        // on a stream socket path for connect(2).
        rustix::fs::access(&socket, rustix::fs::Access::WRITE_OK).is_ok()
    }

    fn resolve_channel_target(&self, vm_name: &str, channel: AudioChannel) -> Option<String> {
        let output = run_subprocess_capture(&self.pw_dump_path, &[], &self.pipewire_runtime_dir)?;
        target_node_from_pw_dump(&output, vm_name, channel)
    }

    /// Run `wpctl set-mute <node-id> <0|1>` as a subprocess.
    ///
    /// The subprocess inherits no environment except `PIPEWIRE_RUNTIME_DIR`
    /// so that `wpctl` can locate the PipeWire socket without needing the
    /// full user session environment.
    fn run_wpctl_mute(&self, node_id: &str, mute: bool) -> HostEnforcementResult {
        let mute_arg = if mute { "1" } else { "0" };
        run_subprocess(
            &self.wpctl_path,
            &["set-mute", node_id, mute_arg],
            &self.pipewire_runtime_dir,
        )
    }

    /// Run `wpctl set-volume <node-id> <level>%` as a subprocess.
    fn run_wpctl_volume(&self, node_id: &str, level: LevelPercent) -> HostEnforcementResult {
        let level_arg = format!("{}%", level.get());
        run_subprocess(
            &self.wpctl_path,
            &["set-volume", node_id, &level_arg],
            &self.pipewire_runtime_dir,
        )
    }
}

impl HostAudioController for PipeWireHostController {
    fn enforce_grant(
        &self,
        vm_name: &str,
        grant: AudioGrant,
        channel: AudioChannel,
    ) -> HostEnforcementResult {
        // Credential posture check: fail immediately if we cannot reach PipeWire.
        // For `off` this means the host boundary is NOT sealed; callers must
        // surface degraded state rather than reporting false success.
        if !self.has_pipewire_access() {
            return HostEnforcementResult::Failed;
        }
        let Some(node_id) = self.resolve_channel_target(vm_name, channel) else {
            return HostEnforcementResult::Unsupported;
        };
        self.run_wpctl_mute(&node_id, !grant.is_on())
    }

    fn enforce_level(
        &self,
        vm_name: &str,
        level: LevelPercent,
        channel: AudioChannel,
    ) -> HostEnforcementResult {
        if !self.has_pipewire_access() {
            return HostEnforcementResult::Failed;
        }
        let Some(node_id) = self.resolve_channel_target(vm_name, channel) else {
            return HostEnforcementResult::Unsupported;
        };
        self.run_wpctl_volume(&node_id, level)
    }
}

// ── QemuAudioController ──────────────────────────────────────────────────────

/// Offline-only host controller for qemu-media VMs.
///
/// qemu-media VMs have no vhost-user-sound sidecar; the qemu audio backend
/// is configured at VM start time. The state-file write that the dispatch
/// layer performs BEFORE calling the controller is the authoritative policy
/// change — the next VM restart picks up the new policy.
///
/// This controller returns [`HostEnforcementResult::Applied`] to signal that
/// the offline policy has been committed, not that live runtime enforcement
/// occurred. The response's `applied` field will be `HostOnly`, which is
/// accurate: the host state file is updated; there is no guest enforcement
/// path for qemu-media VMs.
///
/// The controller never calls guestd — the qemu-media capability row has
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
/// that returns `Applied` — callers that forget to configure the fake will
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

/// Extract a `KEY=VALUE` pair from an env list, returning the value slice.
fn extract_env_value<'a>(env: &'a [String], key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    env.iter().find_map(|entry| entry.strip_prefix(&prefix))
}

fn channel_media_class(channel: AudioChannel) -> &'static str {
    match channel {
        AudioChannel::Speaker => "Stream/Output/Audio",
        AudioChannel::Microphone => "Stream/Input/Audio",
    }
}

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

fn run_subprocess_capture(
    program: &Path,
    args: &[&str],
    pipewire_runtime_dir: &Path,
) -> Option<Vec<u8>> {
    let output = std::process::Command::new(program)
        .args(args)
        .env_clear()
        .env("PIPEWIRE_RUNTIME_DIR", pipewire_runtime_dir)
        .env("XDG_RUNTIME_DIR", pipewire_runtime_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if output.status.success() {
        Some(output.stdout)
    } else {
        None
    }
}

/// Spawn a subprocess at `program` with `args`.
///
/// The subprocess runs as d2bd using the broker-granted PipeWire socket ACL.
/// Both `PIPEWIRE_RUNTIME_DIR` and `XDG_RUNTIME_DIR` are set to the runtime
/// dir path. stderr is not logged because wpctl diagnostics may contain node
/// identifiers, paths, or volume values.
///
/// Returns `Applied` on exit-code 0, `Failed` on any other outcome.
fn run_subprocess(
    program: &Path,
    args: &[&str],
    pipewire_runtime_dir: &Path,
) -> HostEnforcementResult {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args)
        .env_clear()
        .env("PIPEWIRE_RUNTIME_DIR", pipewire_runtime_dir)
        .env("XDG_RUNTIME_DIR", pipewire_runtime_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let output = cmd.output();

    match output {
        Ok(out) if out.status.success() => HostEnforcementResult::Applied,
        Ok(_) => {
            tracing::warn!(subsystem = "d2bd-audio", "wpctl subprocess failed");
            HostEnforcementResult::Failed
        }
        Err(_) => HostEnforcementResult::Failed,
    }
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
    fn pipewire_controller_builds_from_full_env() {
        let node = make_audio_node(vec![
            "PIPEWIRE_RUNTIME_DIR=/run/user/1000".to_owned(),
            "XDG_RUNTIME_DIR=/run/user/1000".to_owned(),
            "WPCTL_PATH=/nix/store/test-wpctl/bin/wpctl".to_owned(),
            "PW_DUMP_PATH=/nix/store/test-pipewire/bin/pw-dump".to_owned(),
        ]);
        let ctrl = PipeWireHostController::from_audio_node(&node);
        assert!(ctrl.is_some(), "should build from full env");
    }

    #[test]
    fn pipewire_controller_returns_none_without_wpctl_path() {
        let node = make_audio_node(vec!["PIPEWIRE_RUNTIME_DIR=/run/user/1000".to_owned()]);
        let ctrl = PipeWireHostController::from_audio_node(&node);
        assert!(ctrl.is_none(), "must require WPCTL_PATH");
    }

    #[test]
    fn pipewire_controller_returns_none_without_runtime_dir() {
        let node = make_audio_node(vec![
            "WPCTL_PATH=/nix/store/test-wpctl/bin/wpctl".to_owned(),
            "PW_DUMP_PATH=/nix/store/test-pipewire/bin/pw-dump".to_owned(),
        ]);
        let ctrl = PipeWireHostController::from_audio_node(&node);
        assert!(ctrl.is_none(), "must require PIPEWIRE_RUNTIME_DIR");
    }

    // ── PipeWireHostController credential check ──────────────────────────────
    // When PIPEWIRE_RUNTIME_DIR points at a non-existent path, access(2) fails
    // and the controller returns Failed (not Unsupported) for Off, signalling
    // that the host boundary was NOT sealed.

    #[test]
    fn pipewire_inaccessible_socket_returns_failed_for_off() {
        let ctrl = PipeWireHostController {
            wpctl_path: PathBuf::from("/dev/null/nonexistent/wpctl"),
            pw_dump_path: PathBuf::from("/dev/null/nonexistent/pw-dump"),
            pipewire_runtime_dir: PathBuf::from("/nonexistent/pipewire-runtime"),
        };
        // Off → boundary must be sealed; if credentials fail, return Failed.
        let result = ctrl.enforce_grant("corp-vm", AudioGrant::Off, AudioChannel::Speaker);
        assert_eq!(
            result,
            HostEnforcementResult::Failed,
            "Off with inaccessible socket must return Failed, not Unsupported"
        );
    }

    #[test]
    fn pipewire_inaccessible_socket_returns_failed_for_on() {
        let ctrl = PipeWireHostController {
            wpctl_path: PathBuf::from("/dev/null/nonexistent/wpctl"),
            pw_dump_path: PathBuf::from("/dev/null/nonexistent/pw-dump"),
            pipewire_runtime_dir: PathBuf::from("/nonexistent/pipewire-runtime"),
        };
        let result = ctrl.enforce_grant("corp-vm", AudioGrant::On, AudioChannel::Speaker);
        assert_eq!(result, HostEnforcementResult::Failed);
    }

    #[test]
    fn pipewire_inaccessible_socket_returns_failed_for_level() {
        let ctrl = PipeWireHostController {
            wpctl_path: PathBuf::from("/dev/null/nonexistent/wpctl"),
            pw_dump_path: PathBuf::from("/dev/null/nonexistent/pw-dump"),
            pipewire_runtime_dir: PathBuf::from("/nonexistent/pipewire-runtime"),
        };
        let level = LevelPercent::new(60).unwrap();
        let result = ctrl.enforce_level("corp-vm", level, AudioChannel::Speaker);
        assert_eq!(result, HostEnforcementResult::Failed);
    }

    // ── find_audio_node ─────────────────────────────────────────────────────

    #[test]
    fn find_audio_node_returns_none_when_absent() {
        use d2b_core::processes::{ProcessesJson, VmProcessDag, VmProcessInvariants};
        let processes = ProcessesJson {
            schema_version: "v3".to_owned(),
            vms: vec![VmProcessDag {
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

    // ── extract_env_value ───────────────────────────────────────────────────

    #[test]
    fn extract_env_value_finds_entry() {
        let env = vec![
            "FOO=bar".to_owned(),
            "PIPEWIRE_RUNTIME_DIR=/run/user/1000".to_owned(),
        ];
        assert_eq!(
            extract_env_value(&env, "PIPEWIRE_RUNTIME_DIR"),
            Some("/run/user/1000")
        );
    }

    #[test]
    fn extract_env_value_missing_key_returns_none() {
        let env = vec!["FOO=bar".to_owned()];
        assert_eq!(extract_env_value(&env, "MISSING_KEY"), None);
    }

    #[test]
    fn extract_env_value_empty_value() {
        let env = vec!["FOO=".to_owned()];
        assert_eq!(extract_env_value(&env, "FOO"), Some(""));
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
