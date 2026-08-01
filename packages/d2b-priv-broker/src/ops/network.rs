//! Trusted bridge lifecycle and opaque persistent-TAP deletion operations.

use std::fs;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use d2b_contracts::broker_wire::DeletePersistentTapRequest;
use d2b_core::bundle_resolver::ResolvedBridgeIntent;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Value-free network operation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkOpError {
    /// The trusted bridge intent was inconsistent with the existing link.
    BridgeParameterMismatch,
    /// Bridge creation or deletion failed.
    BridgeBackend,
    /// A bridge still has attached links and cannot be removed.
    BridgeNotEmpty,
    /// The opaque realization record was missing or unsafe.
    RealizationUnavailable,
    /// The Network generation fence was stale.
    StaleNetworkGeneration,
    /// The attachment generation fence was stale.
    StaleAttachmentGeneration,
    /// Trusted and observed ownership markers did not match.
    ForeignOwnership,
    /// Persistent-TAP deletion failed transiently.
    TapDeleteFailed,
}

impl NetworkOpError {
    /// Return the stable redacted reason code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::BridgeParameterMismatch => "bridge-parameter-mismatch",
            Self::BridgeBackend => "bridge-backend-error",
            Self::BridgeNotEmpty => "bridge-attached-link-present",
            Self::RealizationUnavailable => "attachment-realization-unavailable",
            Self::StaleNetworkGeneration => "stale-network-generation",
            Self::StaleAttachmentGeneration => "stale-attachment-generation",
            Self::ForeignOwnership => "attachment-ownership-conflict",
            Self::TapDeleteFailed => "attachment-delete-failed",
        }
    }
}

impl core::fmt::Display for NetworkOpError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for NetworkOpError {}

/// Identity-free bridge readback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeReadback {
    /// Link exists.
    pub present: bool,
    /// Link is a bridge.
    pub is_bridge: bool,
    /// Observed MTU.
    pub mtu: u16,
    /// STP is disabled.
    pub stp_disabled: bool,
    /// Multicast snooping is disabled.
    pub multicast_snooping_disabled: bool,
    /// IPv6 is disabled.
    pub ipv6_suppressed: bool,
    /// Number of links currently attached to the bridge.
    pub attached_links: usize,
}

/// Injected bridge kernel adapter.
pub trait BridgeBackend {
    /// Read current bridge parameters without exposing the interface in errors.
    fn read_bridge(&self, intent: &ResolvedBridgeIntent) -> Result<BridgeReadback, NetworkOpError>;
    /// Create a down bridge.
    fn create_bridge_down(&self, intent: &ResolvedBridgeIntent) -> Result<(), NetworkOpError>;
    /// Apply all trusted parameters while the bridge remains down.
    fn configure_bridge(&self, intent: &ResolvedBridgeIntent) -> Result<(), NetworkOpError>;
    /// Bring the configured bridge up.
    fn set_bridge_up(&self, intent: &ResolvedBridgeIntent) -> Result<(), NetworkOpError>;
    /// Delete an empty bridge.
    fn delete_bridge(&self, intent: &ResolvedBridgeIntent) -> Result<(), NetworkOpError>;
}

/// Ensure one trusted bridge, applying IPv6 suppression before link-up.
pub fn create_bridge<B: BridgeBackend>(
    backend: &B,
    intent: &ResolvedBridgeIntent,
) -> Result<String, NetworkOpError> {
    let observed = backend.read_bridge(intent)?;
    if observed.present {
        if bridge_matches(intent, observed) {
            return Ok(bridge_intent_digest(intent));
        }
        return Err(NetworkOpError::BridgeParameterMismatch);
    }
    backend.create_bridge_down(intent)?;
    if backend.configure_bridge(intent).is_err() || backend.set_bridge_up(intent).is_err() {
        let _ = backend.delete_bridge(intent);
        return Err(NetworkOpError::BridgeBackend);
    }
    let observed = backend.read_bridge(intent)?;
    if !bridge_matches(intent, observed) {
        let _ = backend.delete_bridge(intent);
        return Err(NetworkOpError::BridgeParameterMismatch);
    }
    Ok(bridge_intent_digest(intent))
}

/// Delete one trusted bridge without cascading into attached links.
pub fn delete_bridge<B: BridgeBackend>(
    backend: &B,
    intent: &ResolvedBridgeIntent,
) -> Result<String, NetworkOpError> {
    let observed = backend.read_bridge(intent)?;
    if !observed.present {
        return Ok(bridge_intent_digest(intent));
    }
    if !bridge_matches(intent, observed) {
        return Err(NetworkOpError::BridgeParameterMismatch);
    }
    if observed.attached_links != 0 {
        return Err(NetworkOpError::BridgeNotEmpty);
    }
    backend.delete_bridge(intent)?;
    Ok(bridge_intent_digest(intent))
}

fn bridge_matches(intent: &ResolvedBridgeIntent, observed: BridgeReadback) -> bool {
    observed.present
        && observed.is_bridge
        && observed.mtu == intent.mtu
        && observed.stp_disabled == intent.stp_disabled
        && observed.multicast_snooping_disabled == intent.multicast_snooping_disabled
        && observed.ipv6_suppressed == intent.ipv6_suppressed
}

/// Path-free digest over trusted bridge configuration.
pub fn bridge_intent_digest(intent: &ResolvedBridgeIntent) -> String {
    digest_parts(&[
        b"bridge-intent-v1",
        intent.intent_id.as_bytes(),
        &intent.mtu.to_be_bytes(),
        &[intent.stp_disabled as u8],
        &[intent.multicast_snooping_disabled as u8],
        &[intent.ipv6_suppressed as u8],
    ])
}

/// System bridge adapter using `ip` and fixed sysctl/sysfs leaves.
pub struct SystemBridgeBackend;

impl core::fmt::Debug for SystemBridgeBackend {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("SystemBridgeBackend")
    }
}

impl BridgeBackend for SystemBridgeBackend {
    fn read_bridge(&self, intent: &ResolvedBridgeIntent) -> Result<BridgeReadback, NetworkOpError> {
        let output = ip_command(&[
            "-d",
            "-j",
            "link",
            "show",
            "dev",
            intent.bridge_ifname.as_str(),
        ])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("does not exist") || stderr.contains("Cannot find device") {
                return Ok(BridgeReadback {
                    present: false,
                    is_bridge: false,
                    mtu: 0,
                    stp_disabled: false,
                    multicast_snooping_disabled: false,
                    ipv6_suppressed: false,
                    attached_links: 0,
                });
            }
            return Err(NetworkOpError::BridgeBackend);
        }
        let links: Vec<serde_json::Value> =
            serde_json::from_slice(&output.stdout).map_err(|_| NetworkOpError::BridgeBackend)?;
        let link = links.first().ok_or(NetworkOpError::BridgeBackend)?;
        let mtu = link
            .get("mtu")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .ok_or(NetworkOpError::BridgeBackend)?;
        let is_bridge = link
            .pointer("/linkinfo/info_kind")
            .and_then(serde_json::Value::as_str)
            == Some("bridge");
        let root = PathBuf::from("/sys/class/net").join(intent.bridge_ifname.as_str());
        let stp_disabled = read_trimmed(&root.join("bridge/stp_state"))? == "0";
        let multicast_snooping_disabled =
            read_trimmed(&root.join("bridge/multicast_snooping"))? == "0";
        let ipv6_suppressed = read_trimmed(
            &PathBuf::from("/proc/sys/net/ipv6/conf")
                .join(intent.bridge_ifname.as_str())
                .join("disable_ipv6"),
        )? == "1";
        let attached = ip_command(&[
            "-j",
            "link",
            "show",
            "master",
            intent.bridge_ifname.as_str(),
        ])?;
        let attached_links = if attached.status.success() {
            serde_json::from_slice::<Vec<serde_json::Value>>(&attached.stdout)
                .map_err(|_| NetworkOpError::BridgeBackend)?
                .len()
        } else {
            return Err(NetworkOpError::BridgeBackend);
        };
        Ok(BridgeReadback {
            present: true,
            is_bridge,
            mtu,
            stp_disabled,
            multicast_snooping_disabled,
            ipv6_suppressed,
            attached_links,
        })
    }

    fn create_bridge_down(&self, intent: &ResolvedBridgeIntent) -> Result<(), NetworkOpError> {
        run_ip(&[
            "link",
            "add",
            "name",
            intent.bridge_ifname.as_str(),
            "type",
            "bridge",
        ])
    }

    fn configure_bridge(&self, intent: &ResolvedBridgeIntent) -> Result<(), NetworkOpError> {
        run_ip(&[
            "link",
            "set",
            "dev",
            intent.bridge_ifname.as_str(),
            "mtu",
            &intent.mtu.to_string(),
        ])?;
        let root = PathBuf::from("/sys/class/net").join(intent.bridge_ifname.as_str());
        write_fixed(&root.join("bridge/stp_state"), "0")?;
        write_fixed(&root.join("bridge/multicast_snooping"), "0")?;
        let ipv6 = PathBuf::from("/proc/sys/net/ipv6/conf").join(intent.bridge_ifname.as_str());
        write_fixed(&ipv6.join("disable_ipv6"), "1")?;
        write_fixed(&ipv6.join("accept_ra"), "0")?;
        write_fixed(&ipv6.join("autoconf"), "0")
    }

    fn set_bridge_up(&self, intent: &ResolvedBridgeIntent) -> Result<(), NetworkOpError> {
        run_ip(&["link", "set", "dev", intent.bridge_ifname.as_str(), "up"])
    }

    fn delete_bridge(&self, intent: &ResolvedBridgeIntent) -> Result<(), NetworkOpError> {
        run_ip(&["link", "delete", "dev", intent.bridge_ifname.as_str()])
    }
}

fn ip_command(args: &[&str]) -> Result<std::process::Output, NetworkOpError> {
    Command::new("/run/current-system/sw/bin/ip")
        .args(args)
        .env_remove("NOTIFY_SOCKET")
        .stdin(Stdio::null())
        .output()
        .map_err(|_| NetworkOpError::BridgeBackend)
}

fn run_ip(args: &[&str]) -> Result<(), NetworkOpError> {
    let output = ip_command(args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(NetworkOpError::BridgeBackend)
    }
}

fn read_trimmed(path: &Path) -> Result<String, NetworkOpError> {
    fs::read_to_string(path)
        .map(|value| value.trim().to_owned())
        .map_err(|_| NetworkOpError::BridgeBackend)
}

fn write_fixed(path: &Path, value: &str) -> Result<(), NetworkOpError> {
    fs::write(path, value).map_err(|_| NetworkOpError::BridgeBackend)
}

/// Trusted broker-owned attachment realization record.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersistentTapRealization {
    /// Opaque attachment identity.
    pub attachment_id: String,
    /// Network generation bound at realization.
    pub network_generation: u64,
    /// Attachment generation bound at realization.
    pub attachment_generation: u64,
    /// Trusted kernel interface name, private to the broker record.
    pub ifname: String,
    /// Trusted ownership marker written by the realization owner.
    pub ownership_marker: String,
}

impl core::fmt::Debug for PersistentTapRealization {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("PersistentTapRealization(<redacted>)")
    }
}

/// Injected persistent-TAP kernel adapter.
pub trait PersistentTapBackend {
    /// Whether the trusted interface exists.
    fn tap_exists(&self, ifname: &str) -> Result<bool, NetworkOpError>;
    /// Delete only the trusted persistent TAP.
    fn delete_tap(&self, ifname: &str) -> Result<(), NetworkOpError>;
}

/// Delete one generation-fenced opaque realization.
pub fn delete_persistent_tap<B: PersistentTapBackend>(
    backend: &B,
    realization: &PersistentTapRealization,
    request: &DeletePersistentTapRequest,
) -> Result<String, NetworkOpError> {
    if realization.attachment_id != request.attachment_id.as_str() {
        return Err(NetworkOpError::RealizationUnavailable);
    }
    if realization.network_generation != request.expected_network_generation.get() {
        return Err(NetworkOpError::StaleNetworkGeneration);
    }
    if realization.attachment_generation != request.expected_attachment_generation.get() {
        return Err(NetworkOpError::StaleAttachmentGeneration);
    }
    let expected_marker = format!("d2b managed: attachment:{}", realization.attachment_id);
    if realization.ownership_marker != expected_marker {
        return Err(NetworkOpError::ForeignOwnership);
    }
    if backend.tap_exists(&realization.ifname)? {
        backend.delete_tap(&realization.ifname)?;
    }
    Ok(attachment_digest(&realization.attachment_id))
}

/// Load a realization from a broker-owned, fd-safe state row.
pub fn load_persistent_tap_realization(
    state_dir: &Path,
    request: &DeletePersistentTapRequest,
) -> Result<PersistentTapRealization, NetworkOpError> {
    let root = state_dir.join("network-attachments");
    let metadata =
        fs::symlink_metadata(&root).map_err(|_| NetworkOpError::RealizationUnavailable)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.mode() & 0o022 != 0 {
        return Err(NetworkOpError::RealizationUnavailable);
    }
    let row = root.join(format!("{}.json", request.attachment_id.as_str()));
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW)
        .open(row)
        .map_err(|_| NetworkOpError::RealizationUnavailable)?;
    let metadata = file
        .metadata()
        .map_err(|_| NetworkOpError::RealizationUnavailable)?;
    if !metadata.is_file() || metadata.mode() & 0o022 != 0 {
        return Err(NetworkOpError::RealizationUnavailable);
    }
    serde_json::from_reader(file).map_err(|_| NetworkOpError::RealizationUnavailable)
}

/// System persistent-TAP adapter.
pub struct SystemPersistentTapBackend;

impl PersistentTapBackend for SystemPersistentTapBackend {
    fn tap_exists(&self, ifname: &str) -> Result<bool, NetworkOpError> {
        let output = ip_command(&["-j", "link", "show", "dev", ifname])?;
        if output.status.success() {
            Ok(true)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("does not exist") || stderr.contains("Cannot find device") {
                Ok(false)
            } else {
                Err(NetworkOpError::TapDeleteFailed)
            }
        }
    }

    fn delete_tap(&self, ifname: &str) -> Result<(), NetworkOpError> {
        let output = ip_command(&["link", "delete", "dev", ifname])?;
        if output.status.success() {
            Ok(())
        } else {
            Err(NetworkOpError::TapDeleteFailed)
        }
    }
}

/// Path-free digest for audit.
pub fn attachment_digest(attachment_id: &str) -> String {
    digest_parts(&[b"attachment-v1", attachment_id.as_bytes()])
}

fn digest_parts(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    let raw: [u8; 32] = hasher.finalize().into();
    format!(
        "sha256:{}",
        raw.iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::{ResourceGeneration, ResourceUid};
    use d2b_core::host::IfName;
    use std::cell::Cell;

    struct FakeBridge {
        state: Cell<BridgeReadback>,
        creates: Cell<usize>,
        deletes: Cell<usize>,
    }

    impl BridgeBackend for FakeBridge {
        fn read_bridge(
            &self,
            _intent: &ResolvedBridgeIntent,
        ) -> Result<BridgeReadback, NetworkOpError> {
            Ok(self.state.get())
        }

        fn create_bridge_down(&self, _intent: &ResolvedBridgeIntent) -> Result<(), NetworkOpError> {
            self.creates.set(self.creates.get() + 1);
            Ok(())
        }

        fn configure_bridge(&self, intent: &ResolvedBridgeIntent) -> Result<(), NetworkOpError> {
            self.state.set(BridgeReadback {
                present: true,
                is_bridge: true,
                mtu: intent.mtu,
                stp_disabled: true,
                multicast_snooping_disabled: true,
                ipv6_suppressed: true,
                attached_links: 0,
            });
            Ok(())
        }

        fn set_bridge_up(&self, _intent: &ResolvedBridgeIntent) -> Result<(), NetworkOpError> {
            Ok(())
        }

        fn delete_bridge(&self, _intent: &ResolvedBridgeIntent) -> Result<(), NetworkOpError> {
            self.deletes.set(self.deletes.get() + 1);
            self.state.set(BridgeReadback {
                present: false,
                is_bridge: false,
                mtu: 0,
                stp_disabled: false,
                multicast_snooping_disabled: false,
                ipv6_suppressed: false,
                attached_links: 0,
            });
            Ok(())
        }
    }

    fn bridge_intent() -> ResolvedBridgeIntent {
        ResolvedBridgeIntent {
            intent_id: "bridge-opaque".to_owned(),
            scope_label: "scope".to_owned(),
            bridge_ifname: IfName::new("d2b-b12345678").unwrap(),
            mtu: 1500,
            stp_disabled: true,
            multicast_snooping_disabled: true,
            ipv6_suppressed: true,
        }
    }

    #[test]
    fn create_bridge_configures_before_success_and_delete_refuses_attached_links() {
        let absent = BridgeReadback {
            present: false,
            is_bridge: false,
            mtu: 0,
            stp_disabled: false,
            multicast_snooping_disabled: false,
            ipv6_suppressed: false,
            attached_links: 0,
        };
        let backend = FakeBridge {
            state: Cell::new(absent),
            creates: Cell::new(0),
            deletes: Cell::new(0),
        };
        create_bridge(&backend, &bridge_intent()).unwrap();
        assert_eq!(backend.creates.get(), 1);
        let mut attached = backend.state.get();
        attached.attached_links = 1;
        backend.state.set(attached);
        assert_eq!(
            delete_bridge(&backend, &bridge_intent()),
            Err(NetworkOpError::BridgeNotEmpty)
        );
        assert_eq!(backend.deletes.get(), 0);
    }

    struct FakeTap {
        present: Cell<bool>,
        deletes: Cell<usize>,
    }

    impl PersistentTapBackend for FakeTap {
        fn tap_exists(&self, _ifname: &str) -> Result<bool, NetworkOpError> {
            Ok(self.present.get())
        }

        fn delete_tap(&self, _ifname: &str) -> Result<(), NetworkOpError> {
            self.deletes.set(self.deletes.get() + 1);
            self.present.set(false);
            Ok(())
        }
    }

    fn request(network: u64, attachment: u64) -> DeletePersistentTapRequest {
        DeletePersistentTapRequest {
            attachment_id: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            expected_network_generation: ResourceGeneration::new(network).unwrap(),
            expected_attachment_generation: ResourceGeneration::new(attachment).unwrap(),
            tracing_span_id: None,
        }
    }

    fn realization() -> PersistentTapRealization {
        PersistentTapRealization {
            attachment_id: "123e4567-e89b-42d3-a456-426614174000".to_owned(),
            network_generation: 4,
            attachment_generation: 7,
            ifname: "d2b-t12345678".to_owned(),
            ownership_marker: "d2b managed: attachment:123e4567-e89b-42d3-a456-426614174000"
                .to_owned(),
        }
    }

    #[test]
    fn delete_persistent_tap_checks_both_fences_before_mutation() {
        let backend = FakeTap {
            present: Cell::new(true),
            deletes: Cell::new(0),
        };
        assert_eq!(
            delete_persistent_tap(&backend, &realization(), &request(3, 7)),
            Err(NetworkOpError::StaleNetworkGeneration)
        );
        assert_eq!(
            delete_persistent_tap(&backend, &realization(), &request(4, 6)),
            Err(NetworkOpError::StaleAttachmentGeneration)
        );
        assert_eq!(backend.deletes.get(), 0);
    }

    #[test]
    fn delete_persistent_tap_validated_absence_is_idempotent() {
        let backend = FakeTap {
            present: Cell::new(false),
            deletes: Cell::new(0),
        };
        assert!(delete_persistent_tap(&backend, &realization(), &request(4, 7)).is_ok());
        assert_eq!(backend.deletes.get(), 0);
    }

    #[test]
    fn delete_persistent_tap_foreign_marker_fails_without_deletion() {
        let backend = FakeTap {
            present: Cell::new(true),
            deletes: Cell::new(0),
        };
        let mut foreign = realization();
        foreign.ownership_marker = "foreign marker".to_owned();
        assert_eq!(
            delete_persistent_tap(&backend, &foreign, &request(4, 7)),
            Err(NetworkOpError::ForeignOwnership)
        );
        assert_eq!(backend.deletes.get(), 0);
    }

    #[test]
    fn request_and_audit_digest_do_not_carry_ifname_or_path() {
        let request = request(4, 7);
        let request_json = serde_json::to_string(&request).unwrap();
        let digest = attachment_digest(request.attachment_id.as_str());
        for forbidden in ["d2b-t12345678", "/sys/class/net", "/dev/net/tun"] {
            assert!(!request_json.contains(forbidden));
            assert!(!digest.contains(forbidden));
        }
        assert!(digest.starts_with("sha256:"));
    }
}
