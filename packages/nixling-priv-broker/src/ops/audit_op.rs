//! Broker audit-record schema. Every broker op emits one record per
//! decision, with the common header below plus a per-variant
//! `operation_fields` nested object. The actual append to the root-owned
//! `0640 root:nixlingd` log goes via [`crate::audit::AuditLog`]; this
//! module is the typed shape used by the op call-sites so the JSON drift
//! gate can read back fields per variant.

use std::io;

use nixling_contracts::broker_wire::RunnerAllocation;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ops::store_sync_audit::StoreSyncAuditFields;

/// Terminal disposition of the swtpm-dir hardening step. Path-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwtpmDirResult {
    /// The persistent swtpm dir did not exist and was freshly created
    /// with owner=`nixling-<vm>-swtpm` and mode 0700.
    Created,
    /// The dir already existed with the correct owner/group; its ACLs
    /// were cleared and mode re-asserted to 0700, contents preserved.
    Reconciled,
    /// The dir existed and was already clean (no reconcile mutation
    /// required beyond verification).
    VerifiedClean,
    /// The step refused to proceed (symlink, non-dir, owner mismatch,
    /// tamper-marker mismatch, etc.). The runner spawn is aborted.
    FailedClosed,
}

/// Terminal disposition of the identity-bound tamper-guard marker.
/// Path-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwtpmMarkerResult {
    /// No prior marker existed; a fresh marker recording the trusted
    /// swtpm-dir identity (st_dev/st_ino + first-provision stamp) was
    /// written.
    Created,
    /// A prior marker existed and verified against the swtpm dir's
    /// current identity.
    Verified,
    /// The marker was absent-after-prior-provision, a symlink, a
    /// non-regular file, foreign-owned, or its recorded identity did
    /// not match the swtpm dir. The step fails closed.
    FailedClosed,
}

/// Hashed/path-free audit fields for the swtpm-dir first-run hardening
/// step (issue #64). NO raw `base_dir` / `tpm.sock` / state paths ever
/// appear here — only a `base_dir_hash`, the closed-set result enums,
/// and the resulting owner/mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwtpmDirAudit {
    /// The VM the swtpm runner belongs to (not a path).
    pub vm_id: String,
    /// FNV1a-64 hash of the persistent swtpm-dir path. Lets operators
    /// correlate records for the same dir without recording the path.
    pub base_dir_hash: String,
    /// Terminal result of the dir provisioning/hardening.
    pub result: SwtpmDirResult,
    /// Mode the dir carries after the step (0o700 on success).
    pub mode: u32,
    /// Owner uid the dir carries after the step (the swtpm principal).
    pub owner_uid: u32,
    /// Owner gid the dir carries after the step (the swtpm principal).
    pub owner_gid: u32,
    /// Terminal result of the identity-bound tamper-guard marker.
    pub marker_result: SwtpmMarkerResult,
    /// Closed-set, path-free reason slug present only when `result` is
    /// `FailedClosed` (e.g. `previously-provisioned-swtpm-state-missing`,
    /// `swtpm-dir-owner-mismatch`, `swtpm-dir-not-a-directory`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail_reason: Option<String>,
}

/// Privileged USB audit identity projection. This record is for the
/// root-owned broker audit log only: vendor/product IDs are normalized to
/// lower-case four-hex strings, and serial-like descriptors are represented
/// only by keyed correlation material when a key is available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbAuditDeviceIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_id: Option<String>,
    pub serial_observed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_correlation: Option<UsbSerialCorrelation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_serial_correlation: Option<UsbSerialCorrelation>,
}

/// HMAC-SHA256 keyed serial correlation material for USB forensics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbSerialCorrelation {
    pub key_id: String,
    pub hmac_sha256: String,
}

/// Path-free USB serial-correlation key rotation metadata.
///
/// Key material never appears here. The active-key count and correlation
/// version let non-root observability readers understand a grace-window
/// transition without receiving HMAC secrets or raw USB serial descriptors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbSerialCorrelationKeyRotationAudit {
    pub previous_key_id: String,
    pub current_key_id: String,
    pub active_key_count: u8,
    pub grace_window_seconds: u64,
    pub correlation_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OperationFields {
    ApplyNftables {
        bundle_nft_intent_ref: String,
        scope_id: String,
        desired_hash: Option<String>,
        destroy: bool,
    },
    ApplyRoute {
        bundle_route_intent_ref: String,
        destination: String,
        via: Option<String>,
        destroy: bool,
    },
    DelegateCgroupV2 {
        scope_id: String,
    },
    OpenCgroupDir {
        scope_id: String,
        path_class: String,
        cgroup_path: String,
    },
    CreateTapFd {
        vm_id: String,
        role_id: String,
        tap_ifname: String,
        bridge_ifname: Option<String>,
    },
    CreatePersistentTap {
        vm_id: String,
        role_id: String,
        tap_ifname: String,
        bridge_ifname: Option<String>,
    },
    OpenKvm {
        role_id: String,
        device_class: String,
        device_path: String,
        matrix_entry_id: String,
    },
    OpenVhostNet {
        role_id: String,
        device_class: String,
        device_path: String,
        matrix_entry_id: String,
    },
    OpenFuse {
        role_id: String,
        device_class: String,
        device_path: String,
        matrix_entry_id: String,
    },
    OpenDevice {
        role_id: String,
        device_class: String,
        device_path: String,
        matrix_entry_id: String,
    },
    ModprobeIfAllowed {
        module_name: String,
        matrix_entry_id: String,
        modules_disabled_sysctl: bool,
        disposition: String,
    },
    PrepareRuntimeDir {
        vm_id: String,
        base_dir: String,
        owner_uid: u32,
        owner_gid: u32,
        mode: u32,
    },
    PrepareStateDir {
        vm_id: String,
        base_dir: String,
        owner_uid: u32,
        owner_gid: u32,
        mode: u32,
    },
    /// Terminal audit fields for the swtpm-dir first-run hardening
    /// step (issue #64). Emitted as a `SpawnRunner` side-effect for the
    /// long-lived `Swtpm` runner: the broker provisions and hardens ONLY
    /// the persistent per-VM swtpm state dir (`${stateDir}/swtpm`,
    /// mode 0700) before the userNS child opens the TPM2 NVRAM by
    /// pathname. The record is host-confidential but PATH-FREE: it
    /// carries a hashed `base_dir_hash` and never the raw state-dir /
    /// `tpm.sock` paths. Exactly one record per swtpm spawn attempt
    /// (success or fail-closed).
    PrepareSwtpmDir(SwtpmDirAudit),
    PrepareStoreView {
        vm: String,
        generation: u64,
        hardlink_farm_path: String,
        view_root: String,
    },
    /// Signed ADR 0027 `StoreSync` terminal audit fields. Every
    /// `StoreSync` attempt emits exactly one of these. The full schema,
    /// enums, and invariant-enforcing constructors live in
    /// [`crate::ops::store_sync_audit`]. The record is host-confidential:
    /// it carries audit-only `caller_principal` / `retained_generations`
    /// and host-only context (`bundle_closure_ref`, `hardlink_farm_path`)
    /// but never store-path basenames, `db.dump`, or marker payloads.
    StoreSync(StoreSyncAuditFields),
    StoreVerify {
        vm: String,
        status: String,
        checked: u32,
        drifted: u32,
        repaired: u32,
        repair_requested: bool,
    },
    SetBridgePortFlags {
        vm: String,
        role: String,
        ifname: String,
        flags: Value,
    },
    SetupMountNamespace {
        vm: String,
        role: String,
        mount_count: u32,
        mount_root: String,
        mount_view_path: String,
        source_view_path: String,
    },
    SpawnRunner {
        bundle_runner_intent_ref: String,
        vm_id: String,
        role_id: String,
        role: String,
        runtime_allocations: Vec<RunnerAllocation>,
    },
    OpenPidfd {
        pid: i32,
        expected_start_time_ticks: u64,
    },
    RunHostInstall {
        bundle_installer_intent_ref: String,
        enable: bool,
        start: bool,
        no_start: bool,
    },
    RunMigrate {
        bundle_migrate_intent_ref: String,
    },
    UsbipBind {
        bus_id: String,
        vm: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device_identity: Option<UsbAuditDeviceIdentity>,
    },
    /// Audit/log hook for USB serial-correlation key rotation metadata.
    /// Carries only key IDs, active-key count, grace-window length, and the
    /// closed correlation algorithm label.
    UsbSerialCorrelationKeyRotate(UsbSerialCorrelationKeyRotationAudit),
    UsbipUnbind {
        bus_id: String,
    },
    UsbipProxyReconcile {},
    UsbipBindFirewallRule {
        bundle_usbip_firewall_intent_ref: String,
    },
    QemuMediaEnroll {
        vm_id: String,
        media_ref: String,
        read_only: bool,
        by_id_count: u32,
        udev_rule_written: bool,
        udev_reloaded: bool,
    },
    QemuMediaRefreshRegistry {
        record_count: u32,
        redacted_index_written: bool,
        udev_rule_written: bool,
        udev_reloaded: bool,
    },
    QemuMediaAttach {
        vm_id: String,
        media_ref: String,
        slot: String,
        read_only: bool,
        qmp_commands: Vec<String>,
    },
    QemuMediaBoot {
        vm_id: String,
        media_ref: String,
        slot: String,
        read_only: bool,
        registry_record_written: bool,
        redacted_index_written: bool,
        udev_rule_written: bool,
        udev_reloaded: bool,
        qmp_commands: Vec<String>,
    },
    QemuMediaSystemPowerdown {
        vm_id: String,
        qmp_command: String,
    },
    QemuMediaQuit {
        vm_id: String,
        qmp_command: String,
    },
    QemuMediaDetach {
        vm_id: String,
        media_ref: String,
        slot: String,
        read_only: bool,
        qmp_commands: Vec<String>,
    },
    GuestControlSign {
        vm_id: String,
        role: String,
        purpose: String,
        transcript_len: usize,
        peer_cid_present: bool,
        capabilities_hash_present: bool,
    },
    ApplyNmUnmanaged {
        bundle_nm_intent_ref: String,
        scope_id: String,
        destroy: bool,
    },
    ApplySysctl {
        bundle_sysctl_intent_ref: String,
        key: String,
        destroy: bool,
    },
    UpdateHostsFile {
        bundle_hosts_intent_ref: String,
        destroy: bool,
    },
    /// Live SeedDnsmasqLease op fields. The broker resolves the per-VM
    /// dnsmasq lease intent from the trusted bundle (using `vm_id`) and
    /// ensures `/var/lib/nixling/dnsmasq/<vm>.leases` exists with the
    /// right owner/mode. The audit row records the resolved vm name and
    /// the scope label so operators can correlate failures with the
    /// per-env lease subtree.
    SeedDnsmasqLease {
        vm_id: String,
        scope_id: String,
    },
    /// Live BindMountFromHardlinkFarm op fields. The broker resolves
    /// the per-VM `store-view` intent from the trusted bundle (using
    /// `vm_id`) and surfaces the hardlink farm path it would bind-mount
    /// from. The audit row records the opaque store-view intent ref (or
    /// `None` when the canonical per-VM intent was used) for
    /// traceability.
    BindMountFromHardlinkFarm {
        vm_id: String,
        bundle_store_view_intent_ref: Option<String>,
        hardlink_farm_path: String,
    },
    SignalRunner {
        vm_id: String,
        role_id: String,
        signal: String,
    },
    DeregisterRunnerPidfd {
        vm_id: String,
        role_id: String,
    },
    RunActivation {
        bundle_activation_intent_ref: String,
        mode: String,
        vm: String,
    },
    RunGc {
        bundle_gc_intent_ref: String,
        keep_generations: Option<u32>,
    },
    RunKeysRotate {
        bundle_keys_intent_ref: String,
        vm: String,
    },
    RunHostKeyTrust {
        bundle_trust_intent_ref: String,
        vm: String,
    },
    RunRotateKnownHost {
        bundle_rotate_known_host_intent_ref: String,
        vm: String,
    },
    Hello {
        client_version: String,
    },
    ValidateBundle {},
    ExportBrokerAudit {
        since: Option<String>,
        filter: Option<String>,
    },
    /// Disk-init audit fields. Records the count of plan-ops executed
    /// and a hash of the target paths (never the raw paths themselves)
    /// to avoid leaking filesystem layout in the audit log.
    DiskInit {
        vm_id: String,
        ops_total: u32,
        ops_created: u32,
        ops_skipped: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ops_repaired: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ops_posture_repaired: Option<u32>,
        target_paths_hash: String,
    },
    ReconcileStorageScope {
        storage_ref: String,
        scope: String,
        kind: String,
        status: String,
        applied: bool,
        path_hash: String,
    },
    ValidateLockSpec {
        lock_ref: String,
        scope: String,
        kind: String,
        cloexec_required: bool,
        fd_passing_mechanism: String,
        order_key: String,
    },
}

impl OperationFields {
    pub fn from_operation_value(operation: &str, value: Value) -> serde_json::Result<Self> {
        macro_rules! parse_fields {
            ($value:expr_2021 => $variant:ident { $($field:ident : $ty:ty),* $(,)? }) => {{
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Fields {
                    $(
                        $field: $ty,
                    )*
                }
                let Fields { $( $field, )* } = serde_json::from_value($value)?;
                Ok(Self::$variant { $( $field, )* })
            }};
            ($value:expr_2021 => $variant:ident {}) => {{
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Fields {}
                let Fields {} = serde_json::from_value($value)?;
                Ok(Self::$variant {})
            }};
        }

        match operation {
            "ApplyNftables" => parse_fields!(value => ApplyNftables {
                bundle_nft_intent_ref: String,
                scope_id: String,
                desired_hash: Option<String>,
                destroy: bool,
            }),
            "ApplyRoute" => parse_fields!(value => ApplyRoute {
                bundle_route_intent_ref: String,
                destination: String,
                via: Option<String>,
                destroy: bool,
            }),
            "DelegateCgroupV2" => parse_fields!(value => DelegateCgroupV2 {
                scope_id: String,
            }),
            "OpenCgroupDir" => parse_fields!(value => OpenCgroupDir {
                scope_id: String,
                path_class: String,
                cgroup_path: String,
            }),
            "CreateTapFd" => parse_fields!(value => CreateTapFd {
                vm_id: String,
                role_id: String,
                tap_ifname: String,
                bridge_ifname: Option<String>,
            }),
            "CreatePersistentTap" => parse_fields!(value => CreatePersistentTap {
                vm_id: String,
                role_id: String,
                tap_ifname: String,
                bridge_ifname: Option<String>,
            }),
            "OpenKvm" => parse_fields!(value => OpenKvm {
                role_id: String,
                device_class: String,
                device_path: String,
                matrix_entry_id: String,
            }),
            "OpenVhostNet" => parse_fields!(value => OpenVhostNet {
                role_id: String,
                device_class: String,
                device_path: String,
                matrix_entry_id: String,
            }),
            "OpenFuse" => parse_fields!(value => OpenFuse {
                role_id: String,
                device_class: String,
                device_path: String,
                matrix_entry_id: String,
            }),
            "OpenDevice" => parse_fields!(value => OpenDevice {
                role_id: String,
                device_class: String,
                device_path: String,
                matrix_entry_id: String,
            }),
            "ModprobeIfAllowed" => parse_fields!(value => ModprobeIfAllowed {
                module_name: String,
                matrix_entry_id: String,
                modules_disabled_sysctl: bool,
                disposition: String,
            }),
            "PrepareRuntimeDir" => parse_fields!(value => PrepareRuntimeDir {
                vm_id: String,
                base_dir: String,
                owner_uid: u32,
                owner_gid: u32,
                mode: u32,
            }),
            "PrepareStateDir" => parse_fields!(value => PrepareStateDir {
                vm_id: String,
                base_dir: String,
                owner_uid: u32,
                owner_gid: u32,
                mode: u32,
            }),
            "PrepareSwtpmDir" => Ok(Self::PrepareSwtpmDir(serde_json::from_value(value)?)),
            "PrepareStoreView" => parse_fields!(value => PrepareStoreView {
                vm: String,
                generation: u64,
                hardlink_farm_path: String,
                view_root: String,
            }),
            "StoreSync" => Ok(Self::StoreSync(serde_json::from_value(value)?)),
            "StoreVerify" => parse_fields!(value => StoreVerify {
                vm: String,
                status: String,
                checked: u32,
                drifted: u32,
                repaired: u32,
                repair_requested: bool,
            }),
            "SetBridgePortFlags" => parse_fields!(value => SetBridgePortFlags {
                vm: String,
                role: String,
                ifname: String,
                flags: Value,
            }),
            "SetupMountNamespace" => parse_fields!(value => SetupMountNamespace {
                vm: String,
                role: String,
                mount_count: u32,
                mount_root: String,
                mount_view_path: String,
                source_view_path: String,
            }),
            "SpawnRunner" => parse_fields!(value => SpawnRunner {
                bundle_runner_intent_ref: String,
                vm_id: String,
                role_id: String,
                role: String,
                runtime_allocations: Vec<RunnerAllocation>,
            }),
            "OpenPidfd" => parse_fields!(value => OpenPidfd {
                pid: i32,
                expected_start_time_ticks: u64,
            }),
            "RunHostInstall" => parse_fields!(value => RunHostInstall {
                bundle_installer_intent_ref: String,
                enable: bool,
                start: bool,
                no_start: bool,
            }),
            "RunMigrate" => parse_fields!(value => RunMigrate {
                bundle_migrate_intent_ref: String,
            }),
            "UsbipBind" => parse_fields!(value => UsbipBind {
                bus_id: String,
                vm: String,
                device_identity: Option<UsbAuditDeviceIdentity>,
            }),
            "UsbSerialCorrelationKeyRotate" => Ok(Self::UsbSerialCorrelationKeyRotate(
                serde_json::from_value(value)?,
            )),
            "UsbipUnbind" => parse_fields!(value => UsbipUnbind {
                bus_id: String,
            }),
            "UsbipProxyReconcile" => parse_fields!(value => UsbipProxyReconcile {}),
            "UsbipBindFirewallRule" => parse_fields!(value => UsbipBindFirewallRule {
                bundle_usbip_firewall_intent_ref: String,
            }),
            "QemuMediaEnroll" => parse_fields!(value => QemuMediaEnroll {
                vm_id: String,
                media_ref: String,
                read_only: bool,
                by_id_count: u32,
                udev_rule_written: bool,
                udev_reloaded: bool,
            }),
            "QemuMediaRefreshRegistry" => parse_fields!(value => QemuMediaRefreshRegistry {
                record_count: u32,
                redacted_index_written: bool,
                udev_rule_written: bool,
                udev_reloaded: bool,
            }),
            "QemuMediaAttach" => parse_fields!(value => QemuMediaAttach {
                vm_id: String,
                media_ref: String,
                slot: String,
                read_only: bool,
                qmp_commands: Vec<String>,
            }),
            "QemuMediaBoot" => parse_fields!(value => QemuMediaBoot {
                vm_id: String,
                media_ref: String,
                slot: String,
                read_only: bool,
                registry_record_written: bool,
                redacted_index_written: bool,
                udev_rule_written: bool,
                udev_reloaded: bool,
                qmp_commands: Vec<String>,
            }),
            "QemuMediaSystemPowerdown" => parse_fields!(value => QemuMediaSystemPowerdown {
                vm_id: String,
                qmp_command: String,
            }),
            "QemuMediaQuit" => parse_fields!(value => QemuMediaQuit {
                vm_id: String,
                qmp_command: String,
            }),
            "QemuMediaDetach" => parse_fields!(value => QemuMediaDetach {
                vm_id: String,
                media_ref: String,
                slot: String,
                read_only: bool,
                qmp_commands: Vec<String>,
            }),
            "GuestControlSign" => parse_fields!(value => GuestControlSign {
                vm_id: String,
                role: String,
                purpose: String,
                transcript_len: usize,
                peer_cid_present: bool,
                capabilities_hash_present: bool,
            }),
            "ApplyNmUnmanaged" => parse_fields!(value => ApplyNmUnmanaged {
                bundle_nm_intent_ref: String,
                scope_id: String,
                destroy: bool,
            }),
            "ApplySysctl" => parse_fields!(value => ApplySysctl {
                bundle_sysctl_intent_ref: String,
                key: String,
                destroy: bool,
            }),
            "UpdateHostsFile" => parse_fields!(value => UpdateHostsFile {
                bundle_hosts_intent_ref: String,
                destroy: bool,
            }),
            "SeedDnsmasqLease" => parse_fields!(value => SeedDnsmasqLease {
                vm_id: String,
                scope_id: String,
            }),
            "BindMountFromHardlinkFarm" => parse_fields!(value => BindMountFromHardlinkFarm {
                vm_id: String,
                bundle_store_view_intent_ref: Option<String>,
                hardlink_farm_path: String,
            }),
            "SignalRunner" => parse_fields!(value => SignalRunner {
                vm_id: String,
                role_id: String,
                signal: String,
            }),
            "DeregisterRunnerPidfd" => parse_fields!(value => DeregisterRunnerPidfd {
                vm_id: String,
                role_id: String,
            }),
            "RunActivation" => parse_fields!(value => RunActivation {
                bundle_activation_intent_ref: String,
                mode: String,
                vm: String,
            }),
            "RunGc" => parse_fields!(value => RunGc {
                bundle_gc_intent_ref: String,
                keep_generations: Option<u32>,
            }),
            "RunKeysRotate" => parse_fields!(value => RunKeysRotate {
                bundle_keys_intent_ref: String,
                vm: String,
            }),
            "RunHostKeyTrust" => parse_fields!(value => RunHostKeyTrust {
                bundle_trust_intent_ref: String,
                vm: String,
            }),
            "RunRotateKnownHost" => parse_fields!(value => RunRotateKnownHost {
                bundle_rotate_known_host_intent_ref: String,
                vm: String,
            }),
            "Hello" => parse_fields!(value => Hello {
                client_version: String,
            }),
            "ValidateBundle" => parse_fields!(value => ValidateBundle {}),
            "ExportBrokerAudit" => parse_fields!(value => ExportBrokerAudit {
                since: Option<String>,
                filter: Option<String>,
            }),
            "DiskInit" => parse_fields!(value => DiskInit {
                vm_id: String,
                ops_total: u32,
                ops_created: u32,
                ops_skipped: u32,
                ops_repaired: Option<u32>,
                ops_posture_repaired: Option<u32>,
                target_paths_hash: String,
            }),
            "ReconcileStorageScope" => parse_fields!(value => ReconcileStorageScope {
                storage_ref: String,
                scope: String,
                kind: String,
                status: String,
                applied: bool,
                path_hash: String,
            }),
            "ValidateLockSpec" => parse_fields!(value => ValidateLockSpec {
                lock_ref: String,
                scope: String,
                kind: String,
                cloexec_required: bool,
                fd_passing_mechanism: String,
                order_key: String,
            }),
            other => Err(serde_json::Error::io(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported audit operation `{other}`"),
            ))),
        }
    }
}

fn default_request_fields() -> Value {
    Value::Object(Default::default())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedOpAuditRecord {
    pub ts_ms: u128,
    pub broker_version: String,
    pub bundle_version: String,
    pub bundle_hash: String,
    pub operation: String,
    pub public_operation_id: String,
    #[serde(default)]
    pub event_id: String,
    pub peer_uid: u32,
    pub peer_gid: u32,
    #[serde(default)]
    pub peer_pid: i32,
    #[serde(default)]
    pub peer_role: String,
    pub authz_result: String,
    pub subject_id: String,
    pub scope_id: String,
    #[serde(default)]
    pub verb: String,
    #[serde(default = "default_request_fields")]
    pub request_fields: Value,
    pub decision: String,
    #[serde(default)]
    pub result: String,
    pub error_kind: Option<String>,
    pub tracing_span_id: Option<String>,
    #[serde(default)]
    pub duration_us: u64,
    pub operation_fields: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpAuditRecord<'a> {
    pub ts_ms: u128,
    pub broker_version: &'a str,
    pub bundle_version: &'a str,
    pub bundle_hash: &'a str,
    pub operation: &'a str,
    pub public_operation_id: &'a str,
    pub event_id: &'a str,
    pub peer_uid: u32,
    pub peer_gid: u32,
    pub peer_pid: i32,
    pub peer_role: &'a str,
    pub authz_result: &'a str,
    pub subject_id: &'a str,
    pub scope_id: &'a str,
    pub verb: &'a str,
    pub request_fields: Value,
    pub decision: &'a str,
    pub result: &'a str,
    pub error_kind: Option<&'a str>,
    pub tracing_span_id: Option<&'a str>,
    pub duration_us: u64,
    pub operation_fields: Option<Value>,
}

impl<'a> OpAuditRecord<'a> {
    /// Renders one JSONL line (single object + newline).
    pub fn to_jsonl(&self) -> String {
        let mut s = serde_json::to_string(self).expect("audit record serializes");
        s.push('\n');
        s
    }
}

impl From<&OpAuditRecord<'_>> for OwnedOpAuditRecord {
    fn from(value: &OpAuditRecord<'_>) -> Self {
        Self {
            ts_ms: value.ts_ms,
            broker_version: value.broker_version.to_owned(),
            bundle_version: value.bundle_version.to_owned(),
            bundle_hash: value.bundle_hash.to_owned(),
            operation: value.operation.to_owned(),
            public_operation_id: value.public_operation_id.to_owned(),
            event_id: value.event_id.to_owned(),
            peer_uid: value.peer_uid,
            peer_gid: value.peer_gid,
            peer_pid: value.peer_pid,
            peer_role: value.peer_role.to_owned(),
            authz_result: value.authz_result.to_owned(),
            subject_id: value.subject_id.to_owned(),
            scope_id: value.scope_id.to_owned(),
            verb: value.verb.to_owned(),
            request_fields: value.request_fields.clone(),
            decision: value.decision.to_owned(),
            result: value.result.to_owned(),
            error_kind: value.error_kind.map(str::to_owned),
            tracing_span_id: value.tracing_span_id.map(str::to_owned),
            duration_us: value.duration_us,
            operation_fields: value.operation_fields.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_contracts::broker_wire::RunnerAllocationKind;
    use serde_json::json;

    macro_rules! roundtrip_test {
        ($name:ident, $operation:literal, $fields:expr_2021) => {
            #[test]
            fn $name() {
                let fields = $fields;
                let value = serde_json::to_value(&fields).expect("serialize operation fields");
                let reparsed = OperationFields::from_operation_value($operation, value.clone())
                    .expect("reparse operation fields");
                assert_eq!(reparsed, fields);
                assert_eq!(serde_json::to_value(&reparsed).unwrap(), value);
            }
        };
    }

    roundtrip_test!(
        apply_nftables_round_trip,
        "ApplyNftables",
        OperationFields::ApplyNftables {
            bundle_nft_intent_ref: "nft:env:work".to_owned(),
            scope_id: "env:work".to_owned(),
            desired_hash: Some("fnv1a64:1234".to_owned()),
            destroy: false,
        }
    );
    roundtrip_test!(
        apply_route_round_trip,
        "ApplyRoute",
        OperationFields::ApplyRoute {
            bundle_route_intent_ref: "route:env:work".to_owned(),
            destination: "default".to_owned(),
            via: Some("192.0.2.1".to_owned()),
            destroy: false,
        }
    );
    roundtrip_test!(
        prepare_store_view_round_trip,
        "PrepareStoreView",
        OperationFields::PrepareStoreView {
            vm: "corp-vm".to_owned(),
            generation: 42,
            hardlink_farm_path: "/var/lib/nixling/vms/corp-vm/store".to_owned(),
            view_root: "/run/nixling/store-views/corp-vm/42".to_owned(),
        }
    );
    roundtrip_test!(
        store_sync_round_trip,
        "StoreSync",
        OperationFields::StoreSync(StoreSyncAuditFields::ok_non_fast_path(
            crate::ops::store_sync_audit::StoreSyncAuditContext {
                vm: "corp-vm".to_owned(),
                vm_id: "store-view:vm:corp-vm".to_owned(),
                env: Some("work".to_owned()),
                bundle_closure_ref: "store-view:vm:corp-vm".to_owned(),
                hardlink_farm_path: "/var/lib/nixling/vms/corp-vm/store-view".to_owned(),
                generation_id: "g-deadbeef".to_owned(),
                generation_token: 42,
                caller_principal: Some("uid:998/role:daemon".to_owned()),
                closure_count: 17,
                timings: crate::ops::store_sync_audit::StoreSyncTimings::default(),
            },
            5,
            12,
            vec![42, 41],
        ))
    );
    roundtrip_test!(
        set_bridge_port_flags_round_trip,
        "SetBridgePortFlags",
        OperationFields::SetBridgePortFlags {
            vm: "corp-vm".to_owned(),
            role: "lan".to_owned(),
            ifname: "tap-corp-vm".to_owned(),
            flags: json!({
                "isolated": true,
                "neighSuppress": true,
            }),
        }
    );
    roundtrip_test!(
        qemu_media_attach_round_trip,
        "QemuMediaAttach",
        OperationFields::QemuMediaAttach {
            vm_id: "media".to_owned(),
            media_ref: "installer-usb".to_owned(),
            slot: "cdrom".to_owned(),
            read_only: true,
            qmp_commands: vec![
                "add-fd".to_owned(),
                "blockdev-add:file".to_owned(),
                "blockdev-add:raw".to_owned(),
                "device_add".to_owned(),
            ],
        }
    );
    roundtrip_test!(
        qemu_media_enroll_round_trip,
        "QemuMediaEnroll",
        OperationFields::QemuMediaEnroll {
            vm_id: "media".to_owned(),
            media_ref: "installer-usb".to_owned(),
            read_only: true,
            by_id_count: 1,
            udev_rule_written: true,
            udev_reloaded: true,
        }
    );
    roundtrip_test!(
        qemu_media_refresh_registry_round_trip,
        "QemuMediaRefreshRegistry",
        OperationFields::QemuMediaRefreshRegistry {
            record_count: 2,
            redacted_index_written: true,
            udev_rule_written: true,
            udev_reloaded: true,
        }
    );
    roundtrip_test!(
        qemu_media_boot_round_trip,
        "QemuMediaBoot",
        OperationFields::QemuMediaBoot {
            vm_id: "media".to_owned(),
            media_ref: "installer-usb".to_owned(),
            slot: "boot".to_owned(),
            read_only: true,
            registry_record_written: true,
            redacted_index_written: true,
            udev_rule_written: true,
            udev_reloaded: true,
            qmp_commands: vec![
                "add-fd".to_owned(),
                "blockdev-add:file".to_owned(),
                "blockdev-add:raw".to_owned(),
                "device_add".to_owned(),
                "cont".to_owned(),
            ],
        }
    );
    roundtrip_test!(
        qemu_media_system_powerdown_round_trip,
        "QemuMediaSystemPowerdown",
        OperationFields::QemuMediaSystemPowerdown {
            vm_id: "media".to_owned(),
            qmp_command: "system_powerdown".to_owned(),
        }
    );
    roundtrip_test!(
        qemu_media_quit_round_trip,
        "QemuMediaQuit",
        OperationFields::QemuMediaQuit {
            vm_id: "media".to_owned(),
            qmp_command: "quit".to_owned(),
        }
    );
    roundtrip_test!(
        qemu_media_detach_round_trip,
        "QemuMediaDetach",
        OperationFields::QemuMediaDetach {
            vm_id: "media".to_owned(),
            media_ref: "installer-usb".to_owned(),
            slot: "cdrom".to_owned(),
            read_only: true,
            qmp_commands: vec![
                "device_del".to_owned(),
                "DEVICE_DELETED".to_owned(),
                "blockdev-del:raw".to_owned(),
                "blockdev-del:file".to_owned(),
                "remove-fd".to_owned(),
            ],
        }
    );
    roundtrip_test!(
        setup_mount_namespace_round_trip,
        "SetupMountNamespace",
        OperationFields::SetupMountNamespace {
            vm: "corp-vm".to_owned(),
            role: "ch-runner".to_owned(),
            mount_count: 1,
            mount_root: "/run/nixling/mountns/corp-vm/ch-runner".to_owned(),
            mount_view_path: "/run/nixling/mountns/corp-vm/ch-runner/nix/store".to_owned(),
            source_view_path: "/run/nixling/store-views/corp-vm/42".to_owned(),
        }
    );
    roundtrip_test!(
        spawn_runner_round_trip,
        "SpawnRunner",
        OperationFields::SpawnRunner {
            bundle_runner_intent_ref: "runner:corp-vm:cloud-hypervisor".to_owned(),
            vm_id: "corp-vm".to_owned(),
            role_id: "ch-runner".to_owned(),
            role: "cloud-hypervisor".to_owned(),
            runtime_allocations: vec![RunnerAllocation {
                kind: RunnerAllocationKind::VsockCid,
                opaque_ref: "cid:42".to_owned(),
            }],
        }
    );
    roundtrip_test!(
        open_pidfd_round_trip,
        "OpenPidfd",
        OperationFields::OpenPidfd {
            pid: 4242,
            expected_start_time_ticks: 123456,
        }
    );
    roundtrip_test!(
        run_host_install_round_trip,
        "RunHostInstall",
        OperationFields::RunHostInstall {
            bundle_installer_intent_ref: "installer:host".to_owned(),
            enable: true,
            start: true,
            no_start: false,
        }
    );
    roundtrip_test!(
        run_migrate_round_trip,
        "RunMigrate",
        OperationFields::RunMigrate {
            bundle_migrate_intent_ref: "migrate:wave15".to_owned(),
        }
    );
    roundtrip_test!(
        usbip_bind_round_trip,
        "UsbipBind",
        OperationFields::UsbipBind {
            bus_id: "1-2.3".to_owned(),
            vm: "corp-vm".to_owned(),
            device_identity: Some(UsbAuditDeviceIdentity {
                vendor_id: Some("1050".to_owned()),
                product_id: Some("0407".to_owned()),
                serial_observed: true,
                serial_correlation: Some(UsbSerialCorrelation {
                    key_id: "test-key".to_owned(),
                    hmac_sha256: "a".repeat(64),
                }),
                previous_serial_correlation: Some(UsbSerialCorrelation {
                    key_id: "previous-key".to_owned(),
                    hmac_sha256: "b".repeat(64),
                }),
            }),
        }
    );
    roundtrip_test!(
        usb_serial_correlation_key_rotate_round_trip,
        "UsbSerialCorrelationKeyRotate",
        OperationFields::UsbSerialCorrelationKeyRotate(UsbSerialCorrelationKeyRotationAudit {
            previous_key_id: "usb-audit-old".to_owned(),
            current_key_id: "usb-audit-new".to_owned(),
            active_key_count: 2,
            grace_window_seconds: 86_400,
            correlation_version: "nixling-usb-audit-serial-v1".to_owned(),
        })
    );

    #[test]
    fn usbip_bind_parses_pre_forensics_audit_records() {
        let fields = OperationFields::from_operation_value(
            "UsbipBind",
            serde_json::json!({
                "bus_id": "1-2.3",
                "vm": "corp-vm",
            }),
        )
        .expect("legacy usbip bind fields parse");

        assert_eq!(
            fields,
            OperationFields::UsbipBind {
                bus_id: "1-2.3".to_owned(),
                vm: "corp-vm".to_owned(),
                device_identity: None,
            }
        );
    }

    roundtrip_test!(
        usbip_unbind_round_trip,
        "UsbipUnbind",
        OperationFields::UsbipUnbind {
            bus_id: "1-2.3".to_owned(),
        }
    );
    roundtrip_test!(
        usbip_proxy_reconcile_round_trip,
        "UsbipProxyReconcile",
        OperationFields::UsbipProxyReconcile {}
    );
    roundtrip_test!(
        usbip_bind_firewall_rule_round_trip,
        "UsbipBindFirewallRule",
        OperationFields::UsbipBindFirewallRule {
            bundle_usbip_firewall_intent_ref: "usbip-firewall:1-2.3".to_owned(),
        }
    );
    roundtrip_test!(
        guest_control_sign_round_trip,
        "GuestControlSign",
        OperationFields::GuestControlSign {
            vm_id: "corp-vm".to_owned(),
            role: "Health".to_owned(),
            purpose: "Readiness".to_owned(),
            transcript_len: 96,
            peer_cid_present: true,
            capabilities_hash_present: false,
        }
    );
    roundtrip_test!(
        apply_nm_unmanaged_round_trip,
        "ApplyNmUnmanaged",
        OperationFields::ApplyNmUnmanaged {
            bundle_nm_intent_ref: "nm:host".to_owned(),
            scope_id: "host".to_owned(),
            destroy: false,
        }
    );
    roundtrip_test!(
        apply_sysctl_round_trip,
        "ApplySysctl",
        OperationFields::ApplySysctl {
            bundle_sysctl_intent_ref: "sysctl:work".to_owned(),
            key: "net.ipv6.conf.nl-work.disable_ipv6".to_owned(),
            destroy: false,
        }
    );
    roundtrip_test!(
        update_hosts_file_round_trip,
        "UpdateHostsFile",
        OperationFields::UpdateHostsFile {
            bundle_hosts_intent_ref: "hosts:host".to_owned(),
            destroy: false,
        }
    );
    roundtrip_test!(
        signal_runner_round_trip,
        "SignalRunner",
        OperationFields::SignalRunner {
            vm_id: "corp-vm".to_owned(),
            role_id: "ch-runner".to_owned(),
            signal: "term".to_owned(),
        }
    );
    roundtrip_test!(
        run_activation_round_trip,
        "RunActivation",
        OperationFields::RunActivation {
            bundle_activation_intent_ref: "activation:corp-vm".to_owned(),
            mode: "switch".to_owned(),
            vm: "corp-vm".to_owned(),
        }
    );
    roundtrip_test!(
        run_gc_round_trip,
        "RunGc",
        OperationFields::RunGc {
            bundle_gc_intent_ref: "gc:host".to_owned(),
            keep_generations: Some(3),
        }
    );
    roundtrip_test!(
        run_keys_rotate_round_trip,
        "RunKeysRotate",
        OperationFields::RunKeysRotate {
            bundle_keys_intent_ref: "keys:corp-vm".to_owned(),
            vm: "corp-vm".to_owned(),
        }
    );
    roundtrip_test!(
        run_host_key_trust_round_trip,
        "RunHostKeyTrust",
        OperationFields::RunHostKeyTrust {
            bundle_trust_intent_ref: "trust:corp-vm".to_owned(),
            vm: "corp-vm".to_owned(),
        }
    );
    roundtrip_test!(
        run_rotate_known_host_round_trip,
        "RunRotateKnownHost",
        OperationFields::RunRotateKnownHost {
            bundle_rotate_known_host_intent_ref: "rotate-known-host:corp-vm".to_owned(),
            vm: "corp-vm".to_owned(),
        }
    );
    roundtrip_test!(
        hello_round_trip,
        "Hello",
        OperationFields::Hello {
            client_version: "1.2.3".to_owned(),
        }
    );
    roundtrip_test!(
        validate_bundle_round_trip,
        "ValidateBundle",
        OperationFields::ValidateBundle {}
    );
    roundtrip_test!(
        export_broker_audit_round_trip,
        "ExportBrokerAudit",
        OperationFields::ExportBrokerAudit {
            since: Some("2026-01-01T00:00:00Z".to_owned()),
            filter: Some(r#"{"kind":"op-name","contains":"Run"}"#.to_owned()),
        }
    );
    roundtrip_test!(
        reconcile_storage_scope_round_trip,
        "ReconcileStorageScope",
        OperationFields::ReconcileStorageScope {
            storage_ref: "path:run-root".to_owned(),
            scope: "host".to_owned(),
            kind: "Directory".to_owned(),
            status: "Clean".to_owned(),
            applied: false,
            path_hash: "fnv1a64:abc".to_owned(),
        }
    );
    roundtrip_test!(
        validate_lock_spec_round_trip,
        "ValidateLockSpec",
        OperationFields::ValidateLockSpec {
            lock_ref: "lock:daemon".to_owned(),
            scope: "host".to_owned(),
            kind: "Ofd".to_owned(),
            cloexec_required: true,
            fd_passing_mechanism: "None".to_owned(),
            order_key: "Global:run:lock:daemon:lock:daemon".to_owned(),
        }
    );
    roundtrip_test!(
        disk_init_round_trip,
        "DiskInit",
        OperationFields::DiskInit {
            vm_id: "corp-vm".to_owned(),
            ops_total: 2,
            ops_created: 0,
            ops_skipped: 1,
            ops_repaired: Some(1),
            ops_posture_repaired: Some(1),
            target_paths_hash: "fnv1a64:1234".to_owned(),
        }
    );

    #[test]
    fn disk_init_historical_audit_without_posture_count_parses() {
        let reparsed = OperationFields::from_operation_value(
            "DiskInit",
            json!({
                "vm_id": "corp-vm",
                "ops_total": 1,
                "ops_created": 0,
                "ops_skipped": 1,
                "ops_repaired": 0,
                "target_paths_hash": "fnv1a64:1234",
            }),
        )
        .expect("historical DiskInit audit parses");
        assert_eq!(
            reparsed,
            OperationFields::DiskInit {
                vm_id: "corp-vm".to_owned(),
                ops_total: 1,
                ops_created: 0,
                ops_skipped: 1,
                ops_repaired: Some(0),
                ops_posture_repaired: None,
                target_paths_hash: "fnv1a64:1234".to_owned(),
            }
        );
    }
    roundtrip_test!(
        prepare_swtpm_dir_success_round_trip,
        "PrepareSwtpmDir",
        OperationFields::PrepareSwtpmDir(SwtpmDirAudit {
            vm_id: "work".to_owned(),
            base_dir_hash: "fnv1a64:dead".to_owned(),
            result: SwtpmDirResult::Created,
            mode: 0o700,
            owner_uid: 6001,
            owner_gid: 6001,
            marker_result: SwtpmMarkerResult::Created,
            fail_reason: None,
        })
    );
    roundtrip_test!(
        prepare_swtpm_dir_fail_closed_round_trip,
        "PrepareSwtpmDir",
        OperationFields::PrepareSwtpmDir(SwtpmDirAudit {
            vm_id: "work".to_owned(),
            base_dir_hash: "fnv1a64:beef".to_owned(),
            result: SwtpmDirResult::FailedClosed,
            mode: 0,
            owner_uid: 6001,
            owner_gid: 6001,
            marker_result: SwtpmMarkerResult::FailedClosed,
            fail_reason: Some("previously-provisioned-swtpm-state-missing".to_owned()),
        })
    );
}
